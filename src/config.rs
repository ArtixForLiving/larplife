use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub check_interval_seconds: u64,
    pub clear_timeout_seconds: u64,
    pub apps: Vec<AppConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub app_name: String,
    pub process_name: String,
    pub discord_app_id: String,
    pub logo_key: String,
    pub logo_text: String,
    pub default_details: String,
    pub state_text: String,
    #[serde(default)]
    pub hide_window_title: bool,
    #[serde(default)]
    pub keep_active_in_background: bool,
    #[serde(default)]
    pub strip_title_words: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        line: usize,
        column: usize,
        message: String,
    },
    Validation {
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io { path, source } => {
                write!(f, "Failed to access config '{}': {}", path.display(), source)
            }
            ConfigError::Parse { line, column, message } => {
                write!(f, "Invalid config JSON at line {}, column {}: {}", line, column, message)
            }
            ConfigError::Validation { message } => {
                write!(f, "Config validation failed: {}", message)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn default_presets() -> Self {
        Self {
            check_interval_seconds: 2,
            clear_timeout_seconds: 30,
            apps: vec![
                AppConfig {
                    app_name: "Adobe Photoshop".to_string(),
                    process_name: "Photoshop.exe".to_string(),
                    discord_app_id: "1182390192837482910".to_string(),
                    logo_key: "photoshop_logo".to_string(),
                    logo_text: "Adobe Photoshop".to_string(),
                    default_details: "Designing graphics".to_string(),
                    state_text: "Active Workspace".to_string(),
                    hide_window_title: false,
                    keep_active_in_background: false,
                    strip_title_words: vec![
                        " - Adobe Photoshop".to_string(),
                        "@ 100%".to_string(),
                        "(RGB/8)".to_string(),
                        "(CMYK/8)".to_string(),
                    ],
                },
                AppConfig {
                    app_name: "Blender".to_string(),
                    process_name: "blender.exe".to_string(),
                    discord_app_id: "1182390998877665544".to_string(),
                    logo_key: "blender_logo".to_string(),
                    logo_text: "Blender 3D".to_string(),
                    default_details: "Working on 3D scene".to_string(),
                    state_text: "3D Viewport / Rendering".to_string(),
                    hide_window_title: false,
                    keep_active_in_background: true,
                    strip_title_words: vec!["Blender".to_string()],
                },
                AppConfig {
                    app_name: "CapCut".to_string(),
                    process_name: "CapCut.exe".to_string(),
                    discord_app_id: "1182391480119521360".to_string(),
                    logo_key: "capcut_logo".to_string(),
                    logo_text: "CapCut Video Editor".to_string(),
                    default_details: "Editing video".to_string(),
                    state_text: "Video Timeline".to_string(),
                    hide_window_title: false,
                    keep_active_in_background: false,
                    strip_title_words: vec!["CapCut".to_string()],
                },
                AppConfig {
                    app_name: "Visual Studio Code".to_string(),
                    process_name: "Code.exe".to_string(),
                    discord_app_id: "1182391850166190130".to_string(),
                    logo_key: "vscode_logo".to_string(),
                    logo_text: "Visual Studio Code".to_string(),
                    default_details: "Writing code".to_string(),
                    state_text: "Workspace Active".to_string(),
                    hide_window_title: false,
                    keep_active_in_background: false,
                    strip_title_words: vec![" - Visual Studio Code".to_string()],
                },
            ],
        }
    }

    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            let config = Self::default_presets();
            config.validate()?;

            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
            }

            let serialized = serde_json::to_string_pretty(&config).map_err(|err| {
                ConfigError::Validation {
                    message: format!("Failed to serialize default config: {}", err),
                }
            })?;

            fs::write(path, serialized).map_err(|source| ConfigError::Io {
                path: path.to_path_buf(),
                source,
            })?;

            return Ok(config);
        }

        let content = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        let config: Self = serde_json::from_str(&content).map_err(|err| ConfigError::Parse {
            line: err.line(),
            column: err.column(),
            message: err.to_string(),
        })?;

        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.check_interval_seconds == 0 {
            return Err(ConfigError::Validation {
                message: "check_interval_seconds must be at least 1".to_string(),
            });
        }

        for (index, app) in self.apps.iter().enumerate() {
            if app.process_name.trim().is_empty() {
                return Err(ConfigError::Validation {
                    message: format!("App at index {} has an empty process_name", index),
                });
            }

            let app_id = app.discord_app_id.trim();
            if app_id.is_empty() {
                return Err(ConfigError::Validation {
                    message: format!("App '{}' has an empty discord_app_id", app.app_name),
                });
            }

            if !app_id.chars().all(|c| c.is_ascii_digit()) {
                return Err(ConfigError::Validation {
                    message: format!(
                        "App '{}' has invalid discord_app_id '{}': must contain numeric digits only",
                        app.app_name, app.discord_app_id
                    ),
                });
            }
        }

        Ok(())
    }

    pub fn find_app(&self, exe_name: &str) -> Option<&AppConfig> {
        self.apps
            .iter()
            .find(|app| app.process_name.eq_ignore_ascii_case(exe_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("larplife_test_{}_{}.json", name, nanos))
    }

    #[test]
    fn creates_default_config_when_missing() {
        let path = temp_config_path("missing");
        let _ = fs::remove_file(&path);

        let cfg = Config::load_or_create(&path).expect("should create default config");
        assert_eq!(cfg.check_interval_seconds, 2);
        assert_eq!(cfg.clear_timeout_seconds, 30);
        assert!(cfg.apps.len() >= 4);
        assert!(path.exists());

        let reloaded = Config::load_or_create(&path).expect("created config must parse cleanly");
        assert_eq!(reloaded, cfg);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn reports_line_and_column_on_syntax_error() {
        let path = temp_config_path("syntax");
        fs::write(&path, "{\n  \"check_interval_seconds\": 2,\n  \"clear_timeout_seconds\": bad\n}").unwrap();

        let err = Config::load_or_create(&path).expect_err("should fail on syntax error");
        let _ = fs::remove_file(&path);

        match err {
            ConfigError::Parse { line, column, .. } => {
                assert_eq!(line, 3);
                assert!(column > 1);
            }
            other => panic!("expected Parse error, got {:?}", other),
        }
    }

    #[test]
    fn rejects_non_numeric_discord_app_id() {
        let mut cfg = Config::default_presets();
        cfg.apps[0].discord_app_id = "abc123".to_string();

        let err = cfg.validate().expect_err("non-numeric app ID must fail");
        assert!(matches!(err, ConfigError::Validation { .. }));
    }

    #[test]
    fn rejects_empty_discord_app_id_or_process_name() {
        let mut cfg = Config::default_presets();
        cfg.apps[0].discord_app_id = "".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));

        let mut cfg2 = Config::default_presets();
        cfg2.apps[0].process_name = "   ".to_string();
        assert!(matches!(cfg2.validate(), Err(ConfigError::Validation { .. })));
    }

    #[test]
    fn rejects_zero_check_interval() {
        let mut cfg = Config::default_presets();
        cfg.check_interval_seconds = 0;
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));
    }

    #[test]
    fn find_app_matches_case_insensitively() {
        let cfg = Config::default_presets();
        let found = cfg.find_app("PHOTOSHOP.EXE").expect("should match Photoshop.exe");
        assert_eq!(found.app_name, "Adobe Photoshop");
        assert!(cfg.find_app("unknown.exe").is_none());
    }
}
