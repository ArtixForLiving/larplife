use crate::config::{AppConfig, Config};
use crate::process_list::{get_running_applications, RunningAppInfo};
use crate::tray::notify_background_process_reload;
use eframe::egui;
use std::path::PathBuf;

pub struct SettingsApp {
    pub config: Config,
    pub config_path: PathBuf,
    pub selected_app_idx: usize,
    pub running_apps: Vec<RunningAppInfo>,
    pub status_message: Option<(String, bool)>,
    pub new_strip_word: String,
}

impl SettingsApp {
    pub fn new_with_path(config_path: PathBuf) -> Self {
        let config = match Config::load_or_create(&config_path) {
            Ok(cfg) => cfg,
            Err(_) => Config::default_presets(),
        };

        let running_apps = get_running_applications();

        Self {
            config,
            config_path,
            selected_app_idx: 0,
            running_apps,
            status_message: None,
            new_strip_word: String::new(),
        }
    }

    pub fn new() -> Self {
        Self::new_with_path(Config::default_path())
    }

    pub fn refresh_running_apps(&mut self) {
        self.running_apps = get_running_applications();
    }

    pub fn add_app(&mut self) {
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
        if self.config.apps.is_empty() {
            return;
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
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Larplife Settings");
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
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("💾 Save & Apply").clicked() {
                    let _ = self.save_changes();
                }

                if let Some((msg, is_error)) = &self.status_message {
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
                    if ui.button("➕ Add").clicked() {
                        self.add_app();
                    }
                    if ui.button("🗑 Del").clicked() {
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

                        ui.label("Discord App ID (numeric):");
                        ui.text_edit_singleline(&mut app.discord_app_id);
                        ui.end_row();

                        ui.label("Logo Asset Key:");
                        ui.text_edit_singleline(&mut app.logo_key);
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
        });
    }
}

pub fn run_settings_gui() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([640.0, 460.0])
            .with_title("larplife - Settings"),
        ..Default::default()
    };

    eframe::run_native(
        "larplife - Settings",
        native_options,
        Box::new(|_cc| Ok(Box::new(SettingsApp::new()))),
    )
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
}
