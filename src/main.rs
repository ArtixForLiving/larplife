#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
use discord_client::{sanitize_title, PresencePayload};
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

    fn build_payload(
        app: &AppConfig,
        window_title: &str,
        start_timestamp: i64,
    ) -> PresencePayload {
        let details = sanitize_title(window_title, app);
        PresencePayload {
            discord_app_id: app.discord_app_id.clone(),
            details,
            state: app.state_text.clone(),
            logo_key: app.logo_key.clone(),
            logo_text: app.logo_text.clone(),
            start_timestamp,
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

                let details = sanitize_title(&win.window_title, app);
                active.last_details = details;

                return TrackerAction::Set(Self::build_payload(
                    app,
                    &win.window_title,
                    active.start_timestamp,
                ));
            }

            // Switch debounce: candidate requires at least 2 consecutive cycles
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

            if candidate_hits >= 2 {
                self.pending = None;
                let start_ts = unix_secs;
                let details = sanitize_title(&win.window_title, app);

                self.active = Some(ActiveSession {
                    process_name: app.process_name.clone(),
                    start_timestamp: start_ts,
                    last_details: details,
                    grace_started_at: None,
                });

                return TrackerAction::Set(Self::build_payload(
                    app,
                    &win.window_title,
                    start_ts,
                ));
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
                return TrackerAction::Set(PresencePayload {
                    discord_app_id: app.discord_app_id.clone(),
                    details: active.last_details.clone(),
                    state: app.state_text.clone(),
                    logo_key: app.logo_key.clone(),
                    logo_text: app.logo_text.clone(),
                    start_timestamp: active.start_timestamp,
                });
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

pub fn is_settings_mode(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--settings")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
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

    log_info("Larplife v3.0 Phase 2 starting background worker and system tray.");

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
    fn debounce_requires_two_consecutive_checks_before_activating() {
        let cfg = Config::default_presets();
        let mut tracker = PresenceTracker::new();
        let t0 = Instant::now();

        let ps = ForegroundReading::Window(ActiveWindow {
            process_name: "Photoshop.exe".to_string(),
            window_title: "poster.psd - Adobe Photoshop".to_string(),
        });

        assert_eq!(
            tracker.step(&cfg, ps.clone(), t0, 1_700_000_000, |_| false),
            TrackerAction::None
        );

        let action = tracker.step(
            &cfg,
            ps,
            t0 + Duration::from_secs(2),
            1_700_000_002,
            |_| false,
        );
        match action {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.details, "poster.psd");
                assert_eq!(payload.start_timestamp, 1_700_000_002);
            }
            other => panic!("expected Set on 2nd check, got {:?}", other),
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
        assert!(matches!(first, TrackerAction::Set(ref p) if p.start_timestamp == 1002));

        let second = tracker.step(&cfg, ps_b, t0 + Duration::from_secs(4), 1004, |_| false);
        match second {
            TrackerAction::Set(payload) => {
                assert_eq!(payload.details, "file_b.psd");
                assert_eq!(payload.start_timestamp, 1002);
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
        assert!(matches!(resumed, TrackerAction::Set(ref p) if p.start_timestamp == 1002));

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
        assert!(matches!(bg_action, TrackerAction::Set(ref p) if p.start_timestamp == 1002));

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
