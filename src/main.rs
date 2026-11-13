#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod app_icon;
pub mod app_resolver;
pub mod image_uploader;
mod config;
mod discord_client;
mod logger;
mod process_list;
mod settings_gui;
mod startup;
mod tray;
mod window_reader;
mod worker;

use config::{AppConfig, Config};
use discord_client::{format_fallback_details, sanitize_title, PresencePayload};
use logger::{log_error, log_info, show_error_dialog};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tray::run_tray_ui;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use window_reader::ForegroundReading;
use worker::{run_worker, WorkerCommand};

#[derive(Debug)]
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[derive(Debug)]
pub enum InstanceAcquireError {
    AlreadyRunning,
    Win32(String),
}

pub fn acquire_single_instance(mutex_name: &str) -> Result<InstanceGuard, InstanceAcquireError> {
    let wide_name: Vec<u16> = OsStr::new(mutex_name)
        .encode_wide()
        .chain(Some(0))
        .collect();

    let handle = unsafe { CreateMutexW(None, true, PCWSTR(wide_name.as_ptr())) }
        .map_err(|err| InstanceAcquireError::Win32(err.to_string()))?;

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Err(InstanceAcquireError::AlreadyRunning);
    }

    Ok(InstanceGuard(handle))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackerAction {
    None,
    Set(PresencePayload),
    Clear,
}

#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub process_name: String,
    pub start_timestamp: i64,
    pub last_details: String,
    pub grace_started_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct PendingCandidate {
    pub process_name: String,
    pub consecutive_hits: u32,
}

#[derive(Debug, Default)]
pub struct PresenceTracker {
    pub active: Option<ActiveSession>,
    pub pending: Option<PendingCandidate>,
}

impl PresenceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build_payload(
        config: &Config,
        app: &AppConfig,
        window_title: &str,
        start_timestamp: i64,
    ) -> PresencePayload {
        let app_id = if app.discord_app_id.trim().is_empty() {
            if config.default_discord_app_id.trim().is_empty() {
                crate::config::DEFAULT_LARPLIFE_APP_ID.to_string()
            } else {
                config.default_discord_app_id.trim().to_string()
            }
        } else {
            app.discord_app_id.trim().to_string()
        };

        let sanitized = sanitize_title(window_title, app);
        let details = if app.discord_app_id.trim().is_empty() {
            format_fallback_details(&app.app_name, &sanitized)
        } else {
            sanitized
        };

        let (small_image, small_text) = if config.badge_enabled && !config.badge_image_url.trim().is_empty() {
            (
                Some(config.badge_image_url.trim().to_string()),
                Some(config.badge_text.trim().to_string()),
            )
        } else {
            (None, None)
        };

        PresencePayload {
            discord_app_id: app_id,
            details,
            state: app.state_text.clone(),
            logo_key: app.logo_key.clone(),
            logo_text: app.logo_text.clone(),
            start_timestamp,
            small_image,
            small_text,
        }
    }

    pub fn step<F>(
        &mut self,
        config: &Config,
        reading: ForegroundReading,
        now: Instant,
        unix_secs: i64,
        is_bg_running: F,
    ) -> TrackerAction
    where
        F: Fn(&str) -> bool,
    {
        let window = match reading {
            ForegroundReading::AccessDenied => {
                // Skip cycle safely on Administrator or elevated window
                return TrackerAction::None;
            }
            ForegroundReading::Transient => None,
            ForegroundReading::Window(win) => Some(win),
        };

        let matched_app = window
            .as_ref()
            .and_then(|w| config.find_app(&w.process_name));

        if let (Some(win), Some(app)) = (window.as_ref(), matched_app) {
            let is_same_app = self
                .active
                .as_ref()
                .map(|a| a.process_name.eq_ignore_ascii_case(&app.process_name))
                .unwrap_or(false);

            if is_same_app {
                self.pending = None;
                let active = self.active.as_mut().expect("active must exist");
                active.grace_started_at = None;

                let payload = Self::build_payload(
                    config,
                    app,
                    &win.window_title,
                    active.start_timestamp,
                );
                active.last_details = payload.details.clone();

                return TrackerAction::Set(payload);
            }

            // Zero-debounce on empty state (candidate_hits >= 1),
            // or 2 consecutive hits required when switching from another active app.
            let candidate_hits = match &mut self.pending {
                Some(candidate)
                    if candidate
                        .process_name
                        .eq_ignore_ascii_case(&app.process_name) =>
                {
                    candidate.consecutive_hits += 1;
                    candidate.consecutive_hits
                }
                _ => {
                    self.pending = Some(PendingCandidate {
                        process_name: app.process_name.clone(),
                        consecutive_hits: 1,
                    });
                    1
                }
            };

            let required_hits = if self.active.is_none() { 1 } else { 2 };

            if candidate_hits >= required_hits {
                self.pending = None;
                let start_ts = unix_secs;
                let payload = Self::build_payload(
                    config,
                    app,
                    &win.window_title,
                    start_ts,
                );

                self.active = Some(ActiveSession {
                    process_name: app.process_name.clone(),
                    start_timestamp: start_ts,
                    last_details: payload.details.clone(),
                    grace_started_at: None,
                });

                return TrackerAction::Set(payload);
            }

            return TrackerAction::None;
        }

        // Foreground is either unregistered app or transient system overlay
        self.pending = None;

        if let Some(active) = &mut self.active {
            let app = config.find_app(&active.process_name);

            if let Some(app) = app
                && app.keep_active_in_background
                && is_bg_running(&active.process_name)
            {
                active.grace_started_at = None;
                let mut payload = Self::build_payload(config, app, "", active.start_timestamp);
                payload.details = active.last_details.clone();
                return TrackerAction::Set(payload);
            }

            match active.grace_started_at {
                None => {
                    active.grace_started_at = Some(now);
                    TrackerAction::None
                }
                Some(started_at) => {
                    if now.duration_since(started_at).as_secs() >= config.clear_timeout_seconds {
                        self.active = None;
                        TrackerAction::Clear
                    } else {
                        TrackerAction::None
                    }
                }
            }
        } else {
            TrackerAction::None
        }
    }
}

pub fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn is_version_mode(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--version" || arg == "-v")
}

pub fn is_settings_mode(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--settings")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if is_version_mode(&args) {
        show_error_dialog("Larplife", &format!("larplife v{}", APP_VERSION));
        return;
    }

    if is_settings_mode(&args) {
        if let Err(err) = settings_gui::run_settings_gui() {
            let msg = format!("Failed to run settings GUI: {}", err);
            log_error(&msg);
            show_error_dialog("Larplife Settings Error", &msg);
        }
        return;
    }

    let _instance_guard = match acquire_single_instance(r"Local\larplife_single_instance_mutex") {
        Ok(guard) => guard,
        Err(InstanceAcquireError::AlreadyRunning) => {
            log_info("Larplife is already running on this machine. Exiting.");
            show_error_dialog("Larplife", "Larplife is already running in the background.");
            std::process::exit(0);
        }
        Err(InstanceAcquireError::Win32(err)) => {
            let msg = format!("Failed to acquire single instance lock: {}", err);
            log_error(&msg);
            show_error_dialog("Larplife Startup Error", &msg);
            std::process::exit(1);
        }
    };

    let config_path = Config::default_path();
    if let Err(err) = Config::load_or_create(&config_path) {
        let msg = format!("Failed to load configuration: {}", err);
        log_error(&msg);
        show_error_dialog("Larplife Configuration Error", &msg);
        std::process::exit(1);
    }

    log_info(&format!(
        "Larplife v{} starting background worker and system tray.",
        APP_VERSION
    ));

    let (cmd_tx, cmd_rx) = channel();
    let (event_tx, event_rx) = channel();

    let worker_config_path = config_path.clone();
    let worker_handle = thread::spawn(move || {
        run_worker(worker_config_path, cmd_rx, event_tx);
    });

    if let Err(err) = run_tray_ui(cmd_tx.clone(), event_rx) {
        let msg = format!("System tray UI encountered an error: {}", err);
        log_error(&msg);
        show_error_dialog("Larplife Tray Error", &msg);
    }

    let _ = cmd_tx.send(WorkerCommand::Shutdown);
    let _ = worker_handle.join();
    log_info("Larplife shutdown complete.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use window_reader::ActiveWindow;

    #[test]
    fn detects_version_mode_from_args() {
        assert!(is_version_mode(&[
            "larplife.exe".to_string(),
            "--version".to_string()
        ]));
        assert!(is_version_mode(&[
            "larplife.exe".to_string(),
            "-v".to_string()
        ]));
        assert!(!is_version_mode(&["larplife.exe".to_string()]));
        assert_eq!(APP_VERSION, "0.3.0");
    }

    #[test]
    fn detects_settings_mode_from_args() {
        assert!(is_settings_mode(&[
            "larplife.exe".to_string(),
            "--settings".to_string()
        ]));
        assert!(!is_settings_mode(&["larplife.exe".to_string()]));
        assert!(!is_settings_mode(&[
            "larplife.exe".to_string(),
            "--other".to_string()
        ]));
    }

    #[test]
    fn single_instance_mutex_blocks_duplicate_and_releases_on_drop() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!(r"Local\larplife_test_mutex_{}", nanos);

        let guard = acquire_single_instance(&name).expect("first acquire must succeed");
        assert!(matches!(
            acquire_single_instance(&name),
            Err(InstanceAcquireError::AlreadyRunning)
        ));

        drop(guard);
        let _guard2 =
            acquire_single_instance(&name).expect("re-acquire after drop must succeed");
    }

    #[test]
    fn zero_debounce_activates_immediately_on_first_cycle_from_empty_state() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        let ps = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "poster.psd - Adobe Photoshop".to_string(),
        });

        let action = tracker.step(&cfg, ps, t0, 1_700_000_000, |_| false);
        match action {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.details, "poster.psd");
                assert_eq!(payload.start_timestamp, 1_700_000_000);
                assert!(tracker.active.is_some());
            }
            other => panic!("expected immediate Set on cycle 1, got {:?}", other),
        }
    }

    #[test]
    fn switch_debounce_requires_two_consecutive_checks_between_active_apps() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        let ps = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "poster.psd - Adobe Photoshop".to_string(),
        });
        let blender = ForegroundReading::Window(ActiveWindow {
            process_name: "blender.exe".to_string(),
            window_title: "Blender - scene.blend".to_string(),
        });

        // App A activates immediately on cycle 1
        let act1 = tracker.step(&cfg, ps, t0, 1000, |_| false);
        assert!(matches!(act1, TrackerAction::Set(_)));

        // App B candidate hit 1 -> TrackerAction::None, active remains App A
        let act2 = tracker.step(&cfg, blender.clone(), t0 + Duration::from_secs(2), 1002, |_| false);
        assert_eq!(act2, TrackerAction::None);
        assert_eq!(
            tracker.active.as_ref().map(|a| a.process_name.as_str()),
            Some("Photoshop.exe")
        );

        // App B candidate hit 2 -> TrackerAction::Set, switches to App B
        let act3 = tracker.step(&cfg, blender, t0 + Duration::from_secs(4), 1004, |_| false);
        match act3 {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.start_timestamp, 1004);
                assert_eq!(
                    tracker.active.as_ref().map(|a| a.process_name.as_str()),
                    Some("blender.exe")
                );
            }
            other => panic!("expected Set on 2nd cycle of App B, got {:?}", other),
        }
    }

    #[test]
    fn alt_tab_quick_switch_back_cancels_pending_candidate_without_flapping() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        let ps = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "poster.psd - Adobe Photoshop".to_string(),
        });
        let blender = ForegroundReading::Window(ActiveWindow {
            process_name: "blender.exe".to_string(),
            window_title: "Blender - scene.blend".to_string(),
        });

        // App A activates at t0
        tracker.step(&cfg, ps.clone(), t0, 1000, |_| false);

        // Quick switch to App B for only 1 cycle
        let b1 = tracker.step(&cfg, blender.clone(), t0 + Duration::from_secs(2), 1002, |_| false);
        assert_eq!(b1, TrackerAction::None);
        assert!(tracker.pending.is_some());

        // Switch back to App A -> resumes without resetting timer, clears candidate
        let a_resumed = tracker.step(&cfg, ps, t0 + Duration::from_secs(4), 1004, |_| false);
        match a_resumed {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.start_timestamp, 1000);
                assert!(tracker.pending.is_none());
            }
            other => panic!("expected Set resuming App A, got {:?}", other),
        }

        // Switching to App B again starts hit 1 fresh, not hit 2
        let b2 = tracker.step(&cfg, blender, t0 + Duration::from_secs(6), 1006, |_| false);
        assert_eq!(b2, TrackerAction::None);
    }

    #[test]
    fn fallback_app_id_uses_default_discord_app_id_and_formats_fallback_details() {
        let mut cfg = Config::default_presets();
        cfg.default_discord_app_id = "1350000000000000000".to_string();
        cfg.apps.push(AppConfig {
            app_name: "Obsidian".to_string(),
            process_name: "Obsidian.exe".to_string(),
            discord_app_id: "".to_string(),
            logo_key: "".to_string(),
            logo_text: "".to_string(),
            default_details: "Taking notes".to_string(),
            state_text: "Writing".to_string(),
            hide_window_title: false,
            keep_active_in_background: false,
            strip_title_words: vec![],
        });

        let mut tracker = PresenceTracker::new();
        let obs = ForegroundReading::Window(ActiveWindow {
            process_name: "Obsidian.exe".to_string(),
            window_title: "Daily Note 2026-11-13".to_string(),
        });

        let action = tracker.step(&cfg, obs, Instant::now(), 1000, |_| false);
        match action {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.discord_app_id, "1350000000000000000");
                assert_eq!(
                    payload.details,
                    "Obsidian — Daily Note 2026-11-13"
                );
            }
            other => panic!("expected Set on cycle 1, got {:?}", other),
        }
    }

    #[test]
    fn fallback_app_id_uses_default_constant_when_default_id_empty() {
        let mut cfg = Config::default_presets();
        cfg.default_discord_app_id = "   ".to_string();
        cfg.apps.push(AppConfig {
            app_name: "Obsidian".to_string(),
            process_name: "Obsidian.exe".to_string(),
            discord_app_id: "".to_string(),
            logo_key: "".to_string(),
            logo_text: "".to_string(),
            default_details: "Taking notes".to_string(),
            state_text: "Writing".to_string(),
            hide_window_title: false,
            keep_active_in_background: false,
            strip_title_words: vec![],
        });

        let mut tracker = PresenceTracker::new();
        let obs = ForegroundReading::Window(ActiveWindow {
            process_name: "Obsidian.exe".to_string(),
            window_title: "Daily Note 2026-11-13".to_string(),
        });

        let action = tracker.step(&cfg, obs, Instant::now(), 1000, |_| false);
        match action {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.discord_app_id, crate::config::DEFAULT_LARPLIFE_APP_ID);
            }
            other => panic!("expected Set on cycle 1, got {:?}", other),
        }
    }

    #[test]
    fn badge_fields_attached_when_badge_enabled_and_omitted_when_disabled() {
        let mut cfg = Config::default_presets();
        cfg.badge_enabled = true;
        cfg.badge_image_url = "https://example.com/badge.png".to_string();
        cfg.badge_text = "larplife v0.3.0".to_string();

        let mut tracker = PresenceTracker::new();
        let ps = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "poster.psd - Adobe Photoshop".to_string(),
        });

        let action_enabled = tracker.step(&cfg, ps.clone(), Instant::now(), 1000, |_| false);
        match action_enabled {
            TrackerAction::Set(payload) => {
                assert_eq!(
                    payload.small_image.as_deref(),
                    Some("https://example.com/badge.png")
                );
                assert_eq!(payload.small_text.as_deref(), Some("larplife v0.3.0"));
            }
            other => panic!("expected Set, got {:?}", other),
        }

        // Test disabled badge
        cfg.badge_enabled = false;
        let mut tracker2 = PresenceTracker::new();
        let action_disabled = tracker2.step(&cfg, ps.clone(), Instant::now(), 2000, |_| false);
        match action_disabled {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.small_image, None);
                assert_eq!(payload.small_text, None);
            }
            other => panic!("expected Set, got {:?}", other),
        }

        // Test empty/whitespace URL
        cfg.badge_enabled = true;
        cfg.badge_image_url = "   ".to_string();
        let mut tracker3 = PresenceTracker::new();
        let action_empty_url = tracker3.step(&cfg, ps, Instant::now(), 3000, |_| false);
        match action_empty_url {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.small_image, None);
                assert_eq!(payload.small_text, None);
            }
            other => panic!("expected Set, got {:?}", other),
        }
    }

    #[test]
    fn elapsed_timestamp_persists_across_title_changes_in_same_app() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        let ps_a = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "file_a.psd - Adobe Photoshop".to_string(),
        });
        let ps_b = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "file_b.psd - Adobe Photoshop".to_string(),
        });

        tracker.step(&cfg, ps_a.clone(), t0, 1000, |_| false);
        let first = tracker.step(&cfg, ps_a, t0 + Duration::from_secs(2), 1002, |_| false);
        assert!(matches!(first, TrackerAction::Set(ref p) if p.start_timestamp == 1000));

        let second = tracker.step(&cfg, ps_b, t0 + Duration::from_secs(4), 1004, |_| false);
        match second {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.details, "file_b.psd");
                assert_eq!(payload.start_timestamp, 1000);
            }
            other => panic!("expected updated Set with same timestamp, got {:?}", other),
        }
    }

    #[test]
    fn grace_period_keeps_presence_until_timeout_and_cancels_on_return() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        let ps = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "art.psd - Adobe Photoshop".to_string(),
        });
        let explorer = ForegroundReading::Window(ActiveWindow {
            process_name: "explorer.exe".to_string(),
            window_title: "Documents".to_string(),
        });

        tracker.step(&cfg, ps.clone(), t0, 1000, |_| false);
        tracker.step(&cfg, ps.clone(), t0 + Duration::from_secs(2), 1002, |_| false);

        // Switch to unregistered app for 20s (< 30s clear_timeout_seconds)
        assert_eq!(
            tracker.step(
                &cfg,
                explorer.clone(),
                t0 + Duration::from_secs(4),
                1004,
                |_| false
            ),
            TrackerAction::None
        );
        assert_eq!(
            tracker.step(
                &cfg,
                ForegroundReading::Transient,
                t0 + Duration::from_secs(20),
                1020,
                |_| false
            ),
            TrackerAction::None
        );

        // Return before timeout: timer preserved, grace cancelled
        let resumed = tracker.step(&cfg, ps, t0 + Duration::from_secs(25), 1025, |_| false);
        assert!(matches!(resumed, TrackerAction::Set(ref p) if p.start_timestamp == 1000));

        // Now leave and let grace period expire (>= 30s)
        assert_eq!(
            tracker.step(
                &cfg,
                explorer.clone(),
                t0 + Duration::from_secs(30),
                1030,
                |_| false
            ),
            TrackerAction::None
        );
        assert_eq!(
            tracker.step(&cfg, explorer, t0 + Duration::from_secs(60), 1060, |_| false),
            TrackerAction::Clear
        );
    }

    #[test]
    fn background_mode_keeps_presence_while_process_running_and_clears_after_exit() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        // Blender has keep_active_in_background: true in default presets
        let blender = ForegroundReading::Window(ActiveWindow {
            process_name: "blender.exe".to_string(),
            window_title: r"Blender - [D:\Secret\render.blend]".to_string(),
        });
        let chrome = ForegroundReading::Window(ActiveWindow {
            process_name: "chrome.exe".to_string(),
            window_title: "Browser".to_string(),
        });

        tracker.step(&cfg, blender.clone(), t0, 1000, |_| true);
        tracker.step(
            &cfg,
            blender,
            t0 + Duration::from_secs(2),
            1002,
            |_| true,
        );

        // Even 120s later in Chrome, if blender.exe is running in background, keep presence
        let bg_action = tracker.step(
            &cfg,
            chrome.clone(),
            t0 + Duration::from_secs(122),
            1122,
            |exe| exe.eq_ignore_ascii_case("blender.exe"),
        );
        assert!(matches!(bg_action, TrackerAction::Set(ref p) if p.start_timestamp == 1000));

        // Once blender.exe exits, grace period starts and clears after 30s
        assert_eq!(
            tracker.step(
                &cfg,
                chrome.clone(),
                t0 + Duration::from_secs(124),
                1124,
                |_| false
            ),
            TrackerAction::None
        );
        assert_eq!(
            tracker.step(&cfg, chrome, t0 + Duration::from_secs(154), 1154, |_| false),
            TrackerAction::Clear
        );
    }

    #[test]
    fn access_denied_skips_cycle_without_clearing_active_presence() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        let ps = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "art.psd - Adobe Photoshop".to_string(),
        });
        tracker.step(&cfg, ps.clone(), t0, 1000, |_| false);
        tracker.step(&cfg, ps, t0 + Duration::from_secs(2), 1002, |_| false);

        assert_eq!(
            tracker.step(
                &cfg,
                ForegroundReading::AccessDenied,
                t0 + Duration::from_secs(100),
                1100,
                |_| false
            ),
            TrackerAction::None
        );
        assert!(tracker.active.is_some());
    }
}
