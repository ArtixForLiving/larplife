use crate::app_resolver::{cache_path, fetch_detectable_apps, resolve_best_candidate, ResolvedAppCandidate};
use crate::config::{AppConfig, Config};
use crate::image_uploader::{extract_icon_from_exe, pick_image_file, MultiHostUploader};
use crate::process_list::{get_running_applications, RunningAppInfo};
use crate::tray::notify_background_process_reload;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(Debug)]
pub enum AsyncGuiEvent {
    CandidateResolved {
        app_idx: usize,
        candidate: Option<ResolvedAppCandidate>,
        error: Option<String>,
    },
    ImageUploaded {
        app_idx: usize,
        url: String,
    },
    UploadFailed {
        app_idx: usize,
        error: String,
    },
}

pub struct SettingsApp {
    pub config: Config,
    pub config_path: PathBuf,
    pub selected_app_idx: usize,
    pub running_apps: Vec<RunningAppInfo>,
    pub status_message: Option<(String, bool)>,
    pub new_strip_word: String,
    pub async_tx: Sender<AsyncGuiEvent>,
    pub async_rx: Receiver<AsyncGuiEvent>,
    pub pending_candidate: Option<(usize, ResolvedAppCandidate)>,
    pub is_busy: bool,
    pub busy_message: Option<String>,
}

impl SettingsApp {
    pub fn new_with_path(config_path: PathBuf) -> Self {
        let config = Config::load_or_create(&config_path).unwrap_or_default();

        let running_apps = get_running_applications();
        let (async_tx, async_rx) = channel();

        Self {
            config,
            config_path,
            selected_app_idx: 0,
            running_apps,
            status_message: None,
            new_strip_word: String::new(),
            async_tx,
            async_rx,
            pending_candidate: None,
            is_busy: false,
            busy_message: None,
        }
    }

    pub fn new() -> Self {
        Self::new_with_path(Config::default_path())
    }

    pub fn refresh_running_apps(&mut self) {
        self.running_apps = get_running_applications();
    }

    pub fn add_app(&mut self) {
        if self.is_busy {
            return;
        }

        let new_app = AppConfig {
            app_name: "New Application".to_string(),
            process_name: "app.exe".to_string(),
            discord_app_id: "123456789012345678".to_string(),
            logo_key: "app_logo".to_string(),
            logo_text: "Application".to_string(),
            default_details: "Working".to_string(),
            state_text: "Active".to_string(),
            hide_window_title: false,
            keep_active_in_background: false,
            strip_title_words: Vec::new(),
        };
        self.config.apps.push(new_app);
        self.selected_app_idx = self.config.apps.len() - 1;
    }

    pub fn remove_selected_app(&mut self) {
        if self.is_busy || self.config.apps.is_empty() {
            return;
        }

        if let Some((cand_idx, _)) = self.pending_candidate {
            if cand_idx == self.selected_app_idx {
                self.pending_candidate = None;
            } else if cand_idx > self.selected_app_idx {
                self.pending_candidate = self
                    .pending_candidate
                    .take()
                    .map(|(idx, cand)| (idx - 1, cand));
            }
        }

        if self.selected_app_idx < self.config.apps.len() {
            self.config.apps.remove(self.selected_app_idx);
        }

        if self.selected_app_idx >= self.config.apps.len() && !self.config.apps.is_empty() {
            self.selected_app_idx = self.config.apps.len() - 1;
        }
    }

    pub fn add_strip_word_to_selected(&mut self) {
        let word = self.new_strip_word.trim();
        if word.is_empty() {
            return;
        }

        if let Some(app) = self.config.apps.get_mut(self.selected_app_idx) {
            app.strip_title_words.push(word.to_string());
            self.new_strip_word.clear();
        }
    }

    pub fn remove_strip_word_from_selected(&mut self, word_idx: usize) {
        if let Some(app) = self.config.apps.get_mut(self.selected_app_idx)
            && word_idx < app.strip_title_words.len()
        {
            app.strip_title_words.remove(word_idx);
        }
    }

    pub fn select_running_app(&mut self, app_info: &RunningAppInfo) {
        if let Some(app) = self.config.apps.get_mut(self.selected_app_idx) {
            app.process_name = app_info.process_name.clone();
            if app.app_name == "New Application" || app.app_name.trim().is_empty() {
                let name = app_info
                    .process_name
                    .strip_suffix(".exe")
                    .unwrap_or(&app_info.process_name);
                app.app_name = name.to_string();
                app.logo_text = name.to_string();
            }
        }
    }

    pub fn handle_async_events(&mut self) {
        while let Ok(event) = self.async_rx.try_recv() {
            self.apply_async_event(event);
        }
    }

    pub fn apply_async_event(&mut self, event: AsyncGuiEvent) {
        self.is_busy = false;
        self.busy_message = None;

        match event {
            AsyncGuiEvent::CandidateResolved {
                app_idx,
                candidate,
                error,
            } => {
                if let Some(err) = error {
                    self.status_message = Some((format!("Resolver error: {}", err), true));
                } else if let Some(cand) = candidate {
                    self.status_message = Some((
                        format!(
                            "Found Discord candidate: {} (Score: {}/100)",
                            cand.discord_app_name, cand.score
                        ),
                        false,
                    ));
                    self.pending_candidate = Some((app_idx, cand));
                } else {
                    self.status_message = Some((
                        "No matching Discord application found in database. Using larplife fallback mode."
                            .to_string(),
                        false,
                    ));
                }
            }
            AsyncGuiEvent::ImageUploaded { app_idx, url } => {
                if let Some(app) = self.config.apps.get_mut(app_idx) {
                    app.logo_key = url.clone();
                    self.status_message =
                        Some((format!("Logo uploaded successfully: {}", url), false));
                }
            }
            AsyncGuiEvent::UploadFailed { app_idx, error } => {
                let is_cancelled = error.to_ascii_lowercase().contains("cancelled");
                let app_name = self
                    .config
                    .apps
                    .get(app_idx)
                    .map(|a| a.app_name.as_str())
                    .unwrap_or("App");
                self.status_message = Some((
                    if is_cancelled {
                        error
                    } else {
                        format!("{}: Upload failed: {}", app_name, error)
                    },
                    !is_cancelled,
                ));
            }
        }
    }

    pub fn accept_candidate(&mut self) {
        if let Some((app_idx, candidate)) = self.pending_candidate.take()
            && let Some(app) = self.config.apps.get_mut(app_idx)
        {
            app.discord_app_id = candidate.discord_app_id.clone();
            self.status_message = Some((
                format!(
                    "Set Discord App ID to {} ({})",
                    app.discord_app_id, candidate.discord_app_name
                ),
                false,
            ));
        }
    }

    pub fn reject_candidate(&mut self) {
        if let Some((app_idx, _candidate)) = self.pending_candidate.take()
            && let Some(app) = self.config.apps.get_mut(app_idx)
        {
            app.discord_app_id.clear();
            self.status_message = Some((
                "Discord App ID cleared (larplife fallback mode active)".to_string(),
                false,
            ));
        }
    }

    pub fn save_changes(&mut self) -> Result<(), String> {
        if let Err(err) = self.config.validate() {
            let msg = format!("Validation failed: {}", err);
            self.status_message = Some((msg.clone(), true));
            return Err(msg);
        }

        if let Err(err) = self.config.save_to_file(&self.config_path) {
            let msg = format!("Failed to save config: {}", err);
            self.status_message = Some((msg.clone(), true));
            return Err(msg);
        }

        notify_background_process_reload();
        self.status_message = Some(("Settings saved and reloaded successfully!".to_string(), false));
        Ok(())
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.is_busy {
            ctx.request_repaint();
        }

        self.handle_async_events();

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(format!("Larplife v{} Settings", env!("CARGO_PKG_VERSION")));
                ui.separator();
                ui.label("Poll Interval (s):");
                ui.add(egui::DragValue::new(&mut self.config.check_interval_seconds).range(1..=60));
                ui.separator();
                ui.label("Clear Timeout (s):");
                ui.add(egui::DragValue::new(&mut self.config.clear_timeout_seconds).range(5..=300));
                ui.separator();
                if ui.button("🔄 Refresh Running Apps").clicked() {
                    self.refresh_running_apps();
                }
            });

            ui.separator();
            egui::CollapsingHeader::new("Global & Fallback Settings")
                .default_open(true)
                .show(ui, |ui| {
                    egui::Grid::new("global_settings_grid")
                        .num_columns(2)
                        .spacing([20.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Default Discord App ID:");
                            ui.text_edit_singleline(&mut self.config.default_discord_app_id);
                            ui.end_row();

                            ui.label("Discord RPC Badge:");
                            ui.checkbox(
                                &mut self.config.badge_enabled,
                                "Enable larplife badge in Discord RPC",
                            );
                            ui.end_row();

                            ui.label("Badge Image URL:");
                            ui.add_enabled(
                                self.config.badge_enabled,
                                egui::TextEdit::singleline(&mut self.config.badge_image_url),
                            );
                            ui.end_row();

                            ui.label("Badge Text:");
                            ui.add_enabled(
                                self.config.badge_enabled,
                                egui::TextEdit::singleline(&mut self.config.badge_text),
                            );
                            ui.end_row();
                        });
                });
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("💾 Save & Apply").clicked() {
                    let _ = self.save_changes();
                }

                if self.is_busy {
                    ui.spinner();
                    if let Some(msg) = &self.busy_message {
                        ui.label(msg);
                    }
                } else if let Some((msg, is_error)) = &self.status_message {
                    if *is_error {
                        ui.colored_label(egui::Color32::RED, msg);
                    } else {
                        ui.colored_label(egui::Color32::GREEN, msg);
                    }
                }
            });
        });

        egui::SidePanel::left("apps_list_panel")
            .resizable(true)
            .default_width(200.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Configured Apps");
                    if ui
                        .add_enabled(!self.is_busy, egui::Button::new("➕ Add"))
                        .clicked()
                    {
                        self.add_app();
                    }
                    if ui
                        .add_enabled(!self.is_busy, egui::Button::new("🗑 Del"))
                        .clicked()
                    {
                        self.remove_selected_app();
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for i in 0..self.config.apps.len() {
                        let label = {
                            let app = &self.config.apps[i];
                            if app.app_name.is_empty() {
                                format!("{}. (unnamed)", i + 1)
                            } else {
                                format!("{}. {}", i + 1, app.app_name)
                            }
                        };
                        let is_selected = self.selected_app_idx == i;
                        if ui.selectable_label(is_selected, label).clicked() {
                            self.selected_app_idx = i;
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.config.apps.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("No applications configured. Click 'Add' on the left to add an app.");
                });
                return;
            }

            if self.selected_app_idx >= self.config.apps.len() {
                self.selected_app_idx = 0;
            }

            let app_idx = self.selected_app_idx;
            let mut chosen_running: Option<RunningAppInfo> = None;
            let mut remove_word_idx: Option<usize> = None;
            let mut add_word_clicked = false;
            let mut confirm_action: Option<bool> = None;
            let mut search_id_clicked = false;
            let mut create_id_clicked = false;
            let mut pick_image_clicked = false;
            let mut extract_icon_clicked = false;

            egui::ScrollArea::vertical().show(ui, |ui| {
                let app = &mut self.config.apps[app_idx];
                ui.heading(format!("Edit: {}", app.app_name));
                ui.separator();

                egui::Grid::new("app_edit_grid")
                    .num_columns(2)
                    .spacing([20.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Display Name:");
                        ui.text_edit_singleline(&mut app.app_name);
                        ui.end_row();

                        ui.label("Process Name (.exe):");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut app.process_name);

                            let current_proc = app.process_name.clone();
                            egui::ComboBox::from_id_salt("running_apps_combo")
                                .selected_text(if current_proc.is_empty() {
                                    "Pick from running...".to_string()
                                } else {
                                    format!("Current: {}", current_proc)
                                })
                                .show_ui(ui, |ui| {
                                    for running in &self.running_apps {
                                        let text = format!("{} ({})", running.process_name, running.window_title);
                                        if ui.selectable_label(running.process_name.eq_ignore_ascii_case(&current_proc), text).clicked() {
                                            chosen_running = Some(running.clone());
                                        }
                                    }
                                });
                        });
                        ui.end_row();

                        ui.label("Discord App ID (leave empty for larplife fallback):");
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut app.discord_app_id);

                                if ui
                                    .add_enabled(!self.is_busy, egui::Button::new("🔍 Search ID Automatically"))
                                    .clicked()
                                {
                                    search_id_clicked = true;
                                }

                                if ui.button("🌐 Create ID in Browser").clicked() {
                                    create_id_clicked = true;
                                }
                            });

                            if let Some((cand_app_idx, candidate)) = &self.pending_candidate
                                && *cand_app_idx == app_idx
                            {
                                ui.add_space(4.0);
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.label(format!(
                                        "Found candidate in Discord database: {} (ID: {}, Score: {}/100)",
                                        candidate.discord_app_name, candidate.discord_app_id, candidate.score
                                    ));
                                    ui.horizontal(|ui| {
                                        if ui.button("Use This ID").clicked() {
                                            confirm_action = Some(true);
                                        }
                                        if ui.button("Not This App (Use larplife Mode)").clicked() {
                                            confirm_action = Some(false);
                                        }
                                    });
                                });
                            }
                        });
                        ui.end_row();

                        ui.label("Logo Key / HTTPS URL:");
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut app.logo_key);

                                if ui
                                    .add_enabled(!self.is_busy, egui::Button::new("🖼 Pick Local Image..."))
                                    .clicked()
                                {
                                    pick_image_clicked = true;
                                }

                                if ui
                                    .add_enabled(!self.is_busy, egui::Button::new("⚡ Extract Icon from .exe"))
                                    .clicked()
                                {
                                    extract_icon_clicked = true;
                                }
                            });

                            ui.colored_label(
                                egui::Color32::from_rgb(180, 180, 180),
                                "Privacy Notice: Uploaded images are stored on public image hosting (Catbox.moe / 0x0.st) so Discord can display them. Use a manual URL if you prefer not to upload local files.",
                            );
                        });
                        ui.end_row();

                        ui.label("Logo Hover Text:");
                        ui.text_edit_singleline(&mut app.logo_text);
                        ui.end_row();

                        ui.label("Default Details:");
                        ui.text_edit_singleline(&mut app.default_details);
                        ui.end_row();

                        ui.label("State Text:");
                        ui.text_edit_singleline(&mut app.state_text);
                        ui.end_row();

                        ui.label("Privacy & Behavior:");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut app.hide_window_title, "Hide window title (show default details only)");
                            ui.checkbox(&mut app.keep_active_in_background, "Keep active while running in background");
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.heading("Strip Title Words");
                ui.label("Words or phrases removed from the window title before sending to Discord:");

                let app = &self.config.apps[app_idx];
                for (idx, word) in app.strip_title_words.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button("✕").clicked() {
                            remove_word_idx = Some(idx);
                        }
                        ui.label(word);
                    });
                }

                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_strip_word);
                    if ui.button("Add Word").clicked() {
                        add_word_clicked = true;
                    }
                });
            });

            if let Some(chosen) = chosen_running {
                self.select_running_app(&chosen);
            }
            if let Some(idx) = remove_word_idx {
                self.remove_strip_word_from_selected(idx);
            }
            if add_word_clicked {
                self.add_strip_word_to_selected();
            }
            if let Some(accept) = confirm_action {
                if accept {
                    self.accept_candidate();
                } else {
                    self.reject_candidate();
                }
            }
            if search_id_clicked {
                let proc_name = self.config.apps[app_idx].process_name.clone();
                if proc_name.trim().is_empty() {
                    self.status_message = Some((
                        "Cannot search: Process name is empty. Please specify a process name first."
                            .to_string(),
                        true,
                    ));
                } else {
                    self.is_busy = true;
                    self.busy_message =
                        Some(format!("Searching Discord database for '{}'...", proc_name));
                    let tx = self.async_tx.clone();
                    std::thread::spawn(move || {
                        let cache_file = cache_path();
                        match fetch_detectable_apps(&cache_file, false) {
                            Ok(apps) => {
                                let candidate = resolve_best_candidate(&proc_name, &apps);
                                let _ = tx.send(AsyncGuiEvent::CandidateResolved {
                                    app_idx,
                                    candidate,
                                    error: None,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(AsyncGuiEvent::CandidateResolved {
                                    app_idx,
                                    candidate: None,
                                    error: Some(e),
                                });
                            }
                        }
                    });
                }
            }
            if create_id_clicked {
                let app_name = self.config.apps[app_idx].app_name.clone();
                ctx.copy_text(app_name.clone());
                match open_browser("https://discord.com/developers/applications") {
                    Ok(_) => {
                        self.status_message = Some((
                            format!(
                                "Copied '{}' to clipboard and opened Discord Developer Portal in browser.",
                                app_name
                            ),
                            false,
                        ));
                    }
                    Err(e) => {
                        self.status_message = Some((
                            format!(
                                "Copied '{}' to clipboard, but failed to open browser: {}",
                                app_name, e
                            ),
                            true,
                        ));
                    }
                }
            }
            if pick_image_clicked {
                self.is_busy = true;
                self.busy_message = Some("Selecting local image file...".to_string());
                let tx = self.async_tx.clone();
                std::thread::spawn(move || {
                    let picked = pick_image_file();
                    match picked {
                        Some(path) => match std::fs::read(&path) {
                            Ok(bytes) => {
                                let filename = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("logo.png");
                                let uploader = MultiHostUploader::new();
                                match uploader.upload(&bytes, filename) {
                                    Ok(url) => {
                                        let _ =
                                            tx.send(AsyncGuiEvent::ImageUploaded { app_idx, url });
                                    }
                                    Err(e) => {
                                        let _ = tx.send(AsyncGuiEvent::UploadFailed {
                                            app_idx,
                                            error: e,
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(AsyncGuiEvent::UploadFailed {
                                    app_idx,
                                    error: format!("Failed to read selected image: {}", e),
                                });
                            }
                        },
                        None => {
                            let _ = tx.send(AsyncGuiEvent::UploadFailed {
                                app_idx,
                                error: "Image file selection was cancelled.".to_string(),
                            });
                        }
                    }
                });
            }
            if extract_icon_clicked {
                let proc_name = self.config.apps[app_idx].process_name.clone();
                let app_name = self.config.apps[app_idx].app_name.clone();
                if proc_name.trim().is_empty() {
                    self.status_message = Some((
                        "Cannot extract icon: Process name is empty.".to_string(),
                        true,
                    ));
                } else {
                    self.is_busy = true;
                    self.busy_message = Some(format!("Extracting icon from '{}'...", proc_name));
                    let tx = self.async_tx.clone();
                    std::thread::spawn(move || {
                        match resolve_exe_path(&proc_name) {
                            Some(exe_path) => {
                                match extract_icon_from_exe(&exe_path) {
                                    Ok(icon_bytes) => {
                                        let safe_name = app_name
                                            .chars()
                                            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                                            .collect::<String>();
                                        let filename = if safe_name.is_empty() {
                                            "app_icon.png".to_string()
                                        } else {
                                            format!("{}_icon.png", safe_name)
                                        };

                                        let uploader = MultiHostUploader::new();
                                        match uploader.upload(&icon_bytes, &filename) {
                                            Ok(url) => {
                                                let _ = tx.send(AsyncGuiEvent::ImageUploaded {
                                                    app_idx,
                                                    url,
                                                });
                                            }
                                            Err(e) => {
                                                let _ = tx.send(AsyncGuiEvent::UploadFailed {
                                                    app_idx,
                                                    error: e,
                                                });
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(AsyncGuiEvent::UploadFailed {
                                            app_idx,
                                            error: format!("Icon extraction failed: {}", e),
                                        });
                                    }
                                }
                            }
                            None => {
                                let _ = tx.send(AsyncGuiEvent::UploadFailed {
                                    app_idx,
                                    error: format!(
                                        "Could not locate executable for '{}'. Please ensure the application is running or provide full path.",
                                        proc_name
                                    ),
                                });
                            }
                        }
                    });
                }
            }
        });
    }
}

pub fn run_settings_gui() -> Result<(), eframe::Error> {
    let window_title = format!("larplife v{} - Settings", env!("CARGO_PKG_VERSION"));
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([640.0, 460.0])
            .with_title(&window_title),
        ..Default::default()
    };

    eframe::run_native(
        &window_title,
        native_options,
        Box::new(|_cc| Ok(Box::new(SettingsApp::new()))),
    )
}

#[cfg(windows)]
pub fn open_browser(url: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide_op: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
    let wide_url: Vec<u16> = OsStr::new(url).encode_wide().chain(Some(0)).collect();

    let res = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(wide_op.as_ptr()),
            PCWSTR(wide_url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    if (res.0 as usize) > 32 {
        Ok(())
    } else {
        Err(format!(
            "ShellExecuteW failed with error code: {}",
            res.0 as usize
        ))
    }
}

#[cfg(not(windows))]
pub fn open_browser(_url: &str) -> Result<(), String> {
    Ok(())
}

pub fn resolve_exe_path(process_name: &str) -> Option<PathBuf> {
    let trimmed = process_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let as_path = PathBuf::from(trimmed);
    if as_path.is_absolute() && as_path.exists() {
        return Some(as_path);
    }

    #[cfg(windows)]
    {
        if let Some(path) = find_running_process_path(trimmed) {
            return Some(path);
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(trimmed);
            if candidate.exists() {
                return Some(candidate);
            }
            if !trimmed.to_ascii_lowercase().ends_with(".exe") {
                let with_exe = dir.join(format!("{}.exe", trimmed));
                if with_exe.exists() {
                    return Some(with_exe);
                }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Ok(windir) = std::env::var("WINDIR") {
            let win_path = PathBuf::from(&windir);
            let sys32 = win_path.join("System32").join(trimmed);
            if sys32.exists() {
                return Some(sys32);
            }
            let win_root = win_path.join(trimmed);
            if win_root.exists() {
                return Some(win_root);
            }
        }
    }

    if as_path.exists() {
        return Some(as_path);
    }

    None
}

#[cfg(windows)]
fn find_running_process_path(target_exe: &str) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(h) if h != INVALID_HANDLE_VALUE => h,
        _ => return None,
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found_path = None;
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let exe_len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let current_exe = OsString::from_wide(&entry.szExeFile[..exe_len])
                .to_string_lossy()
                .into_owned();

            if (current_exe.eq_ignore_ascii_case(target_exe)
                || current_exe.eq_ignore_ascii_case(&format!("{}.exe", target_exe)))
                && let Ok(handle) = unsafe {
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID)
                }
            {
                let mut path_buf = vec![0u16; 1024];
                let mut len = path_buf.len() as u32;
                if unsafe {
                    QueryFullProcessImageNameW(
                        handle,
                        PROCESS_NAME_WIN32,
                        PWSTR(path_buf.as_mut_ptr()),
                        &mut len,
                    )
                }
                .is_ok()
                {
                    let full_path = OsString::from_wide(&path_buf[..len as usize])
                        .to_string_lossy()
                        .into_owned();
                    let p = PathBuf::from(full_path);
                    if p.exists() {
                        found_path = Some(p);
                    }
                }
                let _ = unsafe { CloseHandle(handle) };
                if found_path.is_some() {
                    break;
                }
            }

            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    let _ = unsafe { CloseHandle(snapshot) };
    found_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("larplife_gui_test_{}_{}.json", name, nanos))
    }

    #[test]
    fn settings_app_initializes_with_default_or_existing_config() {
        let path = temp_test_path("init");
        let app = SettingsApp::new_with_path(path.clone());
        assert!(!app.config.apps.is_empty());
        assert_eq!(app.selected_app_idx, 0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn adds_and_removes_configured_apps() {
        let path = temp_test_path("add_remove");
        let mut app = SettingsApp::new_with_path(path.clone());
        let initial_count = app.config.apps.len();

        app.add_app();
        assert_eq!(app.config.apps.len(), initial_count + 1);
        assert_eq!(app.selected_app_idx, initial_count);
        assert_eq!(app.config.apps.last().unwrap().app_name, "New Application");

        app.remove_selected_app();
        assert_eq!(app.config.apps.len(), initial_count);
        assert!(app.selected_app_idx < app.config.apps.len());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn manages_strip_words_for_selected_app() {
        let path = temp_test_path("strip_words");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.selected_app_idx = 0;
        let initial_strip_count = app.config.apps[0].strip_title_words.len();

        app.new_strip_word = "  [SuperSecret]  ".to_string();
        app.add_strip_word_to_selected();
        assert_eq!(
            app.config.apps[0].strip_title_words.len(),
            initial_strip_count + 1
        );
        assert_eq!(
            app.config.apps[0].strip_title_words.last().unwrap(),
            "[SuperSecret]"
        );
        assert!(app.new_strip_word.is_empty());

        app.remove_strip_word_from_selected(initial_strip_count);
        assert_eq!(
            app.config.apps[0].strip_title_words.len(),
            initial_strip_count
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn selects_running_app_info_and_autofills() {
        let path = temp_test_path("running_app_select");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.add_app();

        let running = RunningAppInfo {
            process_name: "Godot_v4.exe".to_string(),
            window_title: "Godot Engine - MyGame".to_string(),
        };

        app.select_running_app(&running);
        let selected = &app.config.apps[app.selected_app_idx];
        assert_eq!(selected.process_name, "Godot_v4.exe");
        assert_eq!(selected.app_name, "Godot_v4");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn saves_valid_configuration_and_reports_validation_error() {
        let path = temp_test_path("save_validation");
        let mut app = SettingsApp::new_with_path(path.clone());

        // Save valid config
        let res = app.save_changes();
        assert!(res.is_ok());
        assert!(path.exists());
        assert!(!app.status_message.as_ref().unwrap().1);

        // Intentionally invalidate
        app.config.apps[0].discord_app_id = "invalid_id_not_numeric".to_string();
        let err_res = app.save_changes();
        assert!(err_res.is_err());
        assert!(app.status_message.as_ref().unwrap().1);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn candidate_acceptance_updates_app_id() {
        let path = temp_test_path("cand_accept");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.config.apps[0].discord_app_id = "initial_id".to_string();

        let candidate = crate::app_resolver::ResolvedAppCandidate {
            discord_app_id: "987654321098765432".to_string(),
            discord_app_name: "Godot Engine".to_string(),
            score: 95,
        };

        app.pending_candidate = Some((0, candidate));
        app.accept_candidate();

        assert_eq!(app.config.apps[0].discord_app_id, "987654321098765432");
        assert!(app.pending_candidate.is_none());
        assert!(app.status_message.as_ref().unwrap().0.contains("987654321098765432"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn candidate_rejection_clears_app_id_to_empty_string() {
        let path = temp_test_path("cand_reject");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.config.apps[0].discord_app_id = "123456789".to_string();

        let candidate = crate::app_resolver::ResolvedAppCandidate {
            discord_app_id: "987654321098765432".to_string(),
            discord_app_name: "Godot Engine".to_string(),
            score: 95,
        };

        app.pending_candidate = Some((0, candidate));
        app.reject_candidate();

        assert_eq!(app.config.apps[0].discord_app_id, "");
        assert!(app.pending_candidate.is_none());
        assert!(app.status_message.as_ref().unwrap().0.contains("larplife fallback mode"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn badge_toggle_and_default_app_id_persist_upon_save() {
        let path = temp_test_path("badge_persist");
        let mut app = SettingsApp::new_with_path(path.clone());

        app.config.badge_enabled = false;
        app.config.default_discord_app_id = "998877665544332211".to_string();
        app.config.badge_image_url = "https://example.com/custom_badge.png".to_string();
        app.config.badge_text = "Custom Badge Text".to_string();

        let res = app.save_changes();
        assert!(res.is_ok());

        let reloaded = Config::load_or_create(&path).expect("Failed to reload saved config");
        assert!(!reloaded.badge_enabled);
        assert_eq!(reloaded.default_discord_app_id, "998877665544332211");
        assert_eq!(reloaded.badge_image_url, "https://example.com/custom_badge.png");
        assert_eq!(reloaded.badge_text, "Custom Badge Text");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn async_events_drained_without_blocking() {
        let path = temp_test_path("async_drain");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.is_busy = true;
        app.busy_message = Some("Uploading image...".to_string());

        app.async_tx
            .send(AsyncGuiEvent::ImageUploaded {
                app_idx: 0,
                url: "https://files.catbox.moe/test1234.png".to_string(),
            })
            .unwrap();

        app.handle_async_events();

        assert_eq!(app.config.apps[0].logo_key, "https://files.catbox.moe/test1234.png");
        assert!(!app.is_busy);
        assert!(app.busy_message.is_none());
        assert!(app.status_message.as_ref().unwrap().0.contains("https://files.catbox.moe/test1234.png"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn async_upload_failed_reports_error_and_clears_busy() {
        let path = temp_test_path("async_fail");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.is_busy = true;
        app.busy_message = Some("Extracting icon...".to_string());

        app.async_tx
            .send(AsyncGuiEvent::UploadFailed {
                app_idx: 0,
                error: "Network connection refused".to_string(),
            })
            .unwrap();

        app.handle_async_events();

        assert!(!app.is_busy);
        assert!(app.busy_message.is_none());
        assert!(app.status_message.as_ref().unwrap().1);
        assert!(app.status_message.as_ref().unwrap().0.contains("Network connection refused"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn candidate_resolved_event_sets_pending_candidate() {
        let path = temp_test_path("cand_event");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.is_busy = true;

        app.async_tx
            .send(AsyncGuiEvent::CandidateResolved {
                app_idx: 0,
                candidate: Some(crate::app_resolver::ResolvedAppCandidate {
                    discord_app_id: "555555555".to_string(),
                    discord_app_name: "Blender".to_string(),
                    score: 90,
                }),
                error: None,
            })
            .unwrap();

        app.handle_async_events();

        assert!(!app.is_busy);
        assert_eq!(
            app.pending_candidate,
            Some((
                0,
                crate::app_resolver::ResolvedAppCandidate {
                    discord_app_id: "555555555".to_string(),
                    discord_app_name: "Blender".to_string(),
                    score: 90,
                }
            ))
        );
        assert!(app.status_message.as_ref().unwrap().0.contains("Blender"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn candidate_resolved_event_with_none_notifies_larplife_mode() {
        let path = temp_test_path("cand_none");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.is_busy = true;

        app.async_tx
            .send(AsyncGuiEvent::CandidateResolved {
                app_idx: 0,
                candidate: None,
                error: None,
            })
            .unwrap();

        app.handle_async_events();

        assert!(!app.is_busy);
        assert!(app.pending_candidate.is_none());
        assert!(app.status_message.as_ref().unwrap().0.contains("larplife fallback mode"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn resolve_exe_path_handles_empty_and_nonexistent_paths() {
        assert_eq!(resolve_exe_path(""), None);
        assert_eq!(resolve_exe_path("   "), None);
        assert_eq!(resolve_exe_path("nonexistent_binary_xyzzz_987.exe"), None);
    }

    #[test]
    fn candidate_resolved_event_with_error_reports_resolver_error() {
        let path = temp_test_path("cand_error");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.is_busy = true;

        app.async_tx
            .send(AsyncGuiEvent::CandidateResolved {
                app_idx: 0,
                candidate: None,
                error: Some("DNS resolution failed".to_string()),
            })
            .unwrap();

        app.handle_async_events();

        assert!(!app.is_busy);
        assert!(app.pending_candidate.is_none());
        assert!(app.status_message.as_ref().unwrap().1);
        assert!(app.status_message.as_ref().unwrap().0.contains("DNS resolution failed"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn removing_selected_app_cleans_up_pending_candidate() {
        let path = temp_test_path("remove_cleanup");
        let mut app = SettingsApp::new_with_path(path.clone());
        app.add_app();
        app.selected_app_idx = 1;

        let candidate = crate::app_resolver::ResolvedAppCandidate {
            discord_app_id: "12345".to_string(),
            discord_app_name: "App 2".to_string(),
            score: 80,
        };
        app.pending_candidate = Some((1, candidate));

        app.remove_selected_app();
        assert!(app.pending_candidate.is_none());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn resolve_exe_path_locates_windows_system_binary() {
        let resolved = resolve_exe_path("cmd.exe");
        assert!(resolved.is_some());
        assert!(resolved.unwrap().exists());
    }

    #[test]
    fn app_mutation_blocked_while_busy() {
        let path = temp_test_path("busy_mutation");
        let mut app = SettingsApp::new_with_path(path.clone());
        let initial_count = app.config.apps.len();
        app.is_busy = true;

        app.add_app();
        assert_eq!(app.config.apps.len(), initial_count);

        app.remove_selected_app();
        assert_eq!(app.config.apps.len(), initial_count);

        app.is_busy = false;
        app.add_app();
        assert_eq!(app.config.apps.len(), initial_count + 1);

        let _ = fs::remove_file(&path);
    }
}
