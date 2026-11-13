use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_LARPLIFE_APP_ID: &str = "1350000000000000000";
pub const DEFAULT_BADGE_IMAGE_URL: &str =
    "https://raw.githubusercontent.com/larplife-project/larplife/main/assets/larplife_badge.png";
pub const DEFAULT_BADGE_TEXT: &str = concat!("larplife v", env!("CARGO_PKG_VERSION"));

fn default_discord_app_id() -> String {
    DEFAULT_LARPLIFE_APP_ID.to_string()
}

fn default_badge_enabled() -> bool {
    true
}

fn default_badge_image_url() -> String {
    DEFAULT_BADGE_IMAGE_URL.to_string()
}

fn default_badge_text() -> String {
    DEFAULT_BADGE_TEXT.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub check_interval_seconds: u64,
    pub clear_timeout_seconds: u64,
    #[serde(default = "default_discord_app_id")]
    pub default_discord_app_id: String,
    #[serde(default = "default_badge_enabled")]
    pub badge_enabled: bool,
    #[serde(default = "default_badge_image_url")]
    pub badge_image_url: String,
    #[serde(default = "default_badge_text")]
    pub badge_text: String,
    pub apps: Vec<AppConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self::default_presets()
    }
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
            default_discord_app_id: DEFAULT_LARPLIFE_APP_ID.to_string(),
            badge_enabled: true,
            badge_image_url: DEFAULT_BADGE_IMAGE_URL.to_string(),
            badge_text: DEFAULT_BADGE_TEXT.to_string(),
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

    pub fn default_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            let exe_dir_config = parent.join("config.json");
            if exe_dir_config.exists() {
                return exe_dir_config;
            }
        }

        let local = PathBuf::from("config.json");
        if local.exists() {
            return local;
        }

        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            return parent.join("config.json");
        }

        local
    }

    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            let config = Self::default_presets();
            config.validate()?;

            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
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

    pub fn save_to_file(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let serialized = serde_json::to_string_pretty(self).map_err(|err| {
            ConfigError::Validation {
                message: format!("Failed to serialize config: {}", err),
            }
        })?;

        fs::write(path, serialized).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.check_interval_seconds == 0 {
            return Err(ConfigError::Validation {
                message: "check_interval_seconds must be at least 1".to_string(),
            });
        }

        let default_app_id = self.default_discord_app_id.trim();
        if !default_app_id.is_empty() && !default_app_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(ConfigError::Validation {
                message: format!(
                    "Invalid default_discord_app_id '{}': must contain numeric digits only",
                    self.default_discord_app_id
                ),
            });
        }

        if self.badge_enabled {
            let url = &self.badge_image_url;
            if !url.starts_with("https://") || url.contains(' ') || url.len() > 256 {
                return Err(ConfigError::Validation {
                    message: format!(
                        "Invalid badge_image_url '{}': must start with 'https://', contain no spaces, and be at most 256 characters",
                        url
                    ),
                });
            }
        }

        for (index, app) in self.apps.iter().enumerate() {
            if app.process_name.trim().is_empty() {
                return Err(ConfigError::Validation {
                    message: format!("App at index {} has an empty process_name", index),
                });
            }

            let app_id = app.discord_app_id.trim();
            if !app_id.is_empty() && !app_id.chars().all(|c| c.is_ascii_digit()) {
                return Err(ConfigError::Validation {
                    message: format!(
                        "App '{}' has invalid discord_app_id '{}': must contain numeric digits only",
                        app.app_name, app.discord_app_id
                    ),
                });
            }

            let logo = &app.logo_key;
            if logo.len() > 256 {
                return Err(ConfigError::Validation {
                    message: format!(
                        "App '{}' has invalid logo_key: length {} exceeds maximum of 256 characters",
                        app.app_name,
                        logo.len()
                    ),
                });
            }
            if logo.contains(' ') {
                return Err(ConfigError::Validation {
                    message: format!(
                        "App '{}' has invalid logo_key '{}': must not contain spaces",
                        app.app_name, logo
                    ),
                });
            }
            if logo.starts_with("http://") {
                return Err(ConfigError::Validation {
                    message: format!(
                        "App '{}' has invalid logo_key '{}': insecure HTTP is not allowed, must use HTTPS or an asset key",
                        app.app_name, logo
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
    fn rejects_empty_process_name() {
        let mut cfg = Config::default_presets();
        cfg.apps[0].process_name = "   ".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));
    }

    #[test]
    fn backward_compatibility_deserializes_missing_v0_3_0_fields_with_defaults() {
        let legacy_json = r#"{
            "check_interval_seconds": 2,
            "clear_timeout_seconds": 30,
            "apps": []
        }"#;

        let cfg: Config = serde_json::from_str(legacy_json).expect("should parse legacy config");
        assert_eq!(cfg.default_discord_app_id, DEFAULT_LARPLIFE_APP_ID);
        assert!(cfg.badge_enabled);
        assert_eq!(cfg.badge_image_url, DEFAULT_BADGE_IMAGE_URL);
        assert_eq!(cfg.badge_text, DEFAULT_BADGE_TEXT);
    }

    #[test]
    fn validation_allows_empty_discord_app_id() {
        let mut cfg = Config::default_presets();
        cfg.apps[0].discord_app_id = "".to_string();
        assert!(cfg.validate().is_ok());

        cfg.apps[0].discord_app_id = "   ".to_string();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validation_rejects_non_digit_non_empty_discord_app_id() {
        let mut cfg = Config::default_presets();
        cfg.apps[0].discord_app_id = "1234abc".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));

        let mut cfg2 = Config::default_presets();
        cfg2.default_discord_app_id = "non_numeric_id".to_string();
        assert!(matches!(cfg2.validate(), Err(ConfigError::Validation { .. })));
    }

    #[test]
    fn validation_rejects_invalid_logo_key() {
        let mut cfg = Config::default_presets();
        cfg.apps[0].logo_key = "http://insecure.com/logo.png".to_string();
        let err = cfg.validate().expect_err("http:// URL must be rejected");
        assert!(matches!(err, ConfigError::Validation { .. }));

        cfg.apps[0].logo_key = "logo key with spaces".to_string();
        let err = cfg.validate().expect_err("logo_key with spaces must be rejected");
        assert!(matches!(err, ConfigError::Validation { .. }));

        cfg.apps[0].logo_key = "a".repeat(257);
        let err = cfg.validate().expect_err("logo_key > 256 chars must be rejected");
        assert!(matches!(err, ConfigError::Validation { .. }));

        // Valid logo keys
        cfg.apps[0].logo_key = "https://example.com/logo.png".to_string();
        assert!(cfg.validate().is_ok());

        cfg.apps[0].logo_key = "valid_asset_key_123".to_string();
        assert!(cfg.validate().is_ok());

        cfg.apps[0].logo_key = "".to_string();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validation_rejects_invalid_badge_image_url_when_badge_enabled() {
        let mut cfg = Config::default_presets();
        cfg.badge_enabled = true;

        cfg.badge_image_url = "http://insecure.com/badge.png".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));

        cfg.badge_image_url = "https://example.com/badge with spaces.png".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));

        cfg.badge_image_url = format!("https://example.com/{}", "a".repeat(250));
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));

        cfg.badge_image_url = "".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation { .. })));

        // When badge_enabled is false, invalid url does not fail validation
        cfg.badge_enabled = false;
        cfg.badge_image_url = "".to_string();
        assert!(cfg.validate().is_ok());

        cfg.badge_image_url = "http://insecure.com/badge.png".to_string();
        assert!(cfg.validate().is_ok());
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

    #[test]
    fn saves_and_reloads_config_cleanly() {
        let path = temp_config_path("save");
        let mut cfg = Config::default_presets();
        cfg.check_interval_seconds = 5;
        cfg.apps[0].app_name = "Custom App".to_string();

        cfg.save_to_file(&path).expect("save must succeed");
        assert!(path.exists());

        let loaded = Config::load_or_create(&path).expect("load must succeed");
        assert_eq!(loaded.check_interval_seconds, 5);
        assert_eq!(loaded.apps[0].app_name, "Custom App");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn default_path_returns_valid_path() {
        let path = Config::default_path();
        assert_eq!(path.file_name().unwrap(), "config.json");
    }
}
