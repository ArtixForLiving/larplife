use crate::config::AppConfig;
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresencePayload {
    pub discord_app_id: String,
    pub details: String,
    pub state: String,
    pub logo_key: String,
    pub logo_text: String,
    pub start_timestamp: i64,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
}

pub fn format_fallback_details(app_name: &str, sanitized_details: &str) -> String {
    let trimmed_app = app_name.trim();
    let trimmed_details = sanitized_details.trim();

    if trimmed_details.is_empty() {
        return trimmed_app.to_string();
    }

    if trimmed_app.is_empty() {
        return trimmed_details.to_string();
    }

    if trimmed_details
        .to_lowercase()
        .contains(&trimmed_app.to_lowercase())
    {
        return trimmed_details.to_string();
    }

    format!("{} — {}", trimmed_app, trimmed_details)
}

pub fn format_discord_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "  ".to_string();
    }

    let limit = 120;
    let cut = if trimmed.len() > limit {
        trimmed.floor_char_boundary(limit)
    } else {
        trimmed.len()
    };
    let truncated = &trimmed[..cut];

    if truncated.chars().count() < 2 {
        let mut padded = truncated.to_string();
        while padded.chars().count() < 2 {
            padded.push(' ');
        }
        padded
    } else {
        truncated.to_string()
    }
}

fn find_path_end(rest: &str) -> usize {
    let bytes = rest.as_bytes();
    let mut i = 3; // Skip "<drive>:\"
    while i < bytes.len() {
        let b = bytes[i];
        if matches!(b, b']' | b')' | b'"' | b'\'' | b'|' | b'\r' | b'\n') {
            return i;
        }
        if rest[i..].starts_with(" - ") || rest[i..].starts_with(" @ ") {
            return i;
        }
        i += 1;
    }
    rest.len()
}

fn strip_path_segment(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        let starts_drive_path = i + 3 <= bytes.len()
            && bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/')
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'[' | b'(' | b'"' | b'\''));

        if starts_drive_path {
            let end = i + find_path_end(&input[i..]);
            let path_slice = &input[i..end];
            if let Some(sep_idx) = path_slice.rfind(['\\', '/']) {
                result.push_str(&path_slice[sep_idx + 1..]);
            } else {
                result.push_str(path_slice);
            }
            i = end;
        } else if let Some(ch) = input[i..].chars().next() {
            result.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }

    result
}

pub fn sanitize_title(raw_title: &str, app: &AppConfig) -> String {
    if app.hide_window_title {
        return app.default_details.clone();
    }

    let trimmed = raw_title.trim();
    if trimmed.is_empty() {
        return app.default_details.clone();
    }

    let mut cleaned = strip_path_segment(trimmed);

    for strip_word in &app.strip_title_words {
        cleaned = cleaned.replace(strip_word, "");
    }

    let trimmed_result = cleaned.trim().trim_matches(['-', ' ']).trim();

    if trimmed_result.is_empty() || trimmed_result == "[]" {
        return app.default_details.clone();
    }

    trimmed_result.to_string()
}

pub fn build_activity(payload: &PresencePayload) -> activity::Activity<'static> {
    let formatted_details = format_discord_text(&payload.details);
    let formatted_state = format_discord_text(&payload.state);

    let mut activity = activity::Activity::new()
        .details(formatted_details)
        .state(formatted_state)
        .timestamps(activity::Timestamps::new().start(payload.start_timestamp));

    let key_trimmed = payload.logo_key.trim();
    let text_trimmed = payload.logo_text.trim();
    let small_image_trimmed = payload
        .small_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let small_text_trimmed = payload
        .small_text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let has_large = !key_trimmed.is_empty();
    let has_small = small_image_trimmed.is_some();

    if has_large || has_small {
        let mut assets = activity::Assets::new();
        if has_large {
            assets = assets.large_image(key_trimmed.to_string());
            if !text_trimmed.is_empty() {
                assets = assets.large_text(format_discord_text(text_trimmed));
            }
        }
        if let Some(small_key) = small_image_trimmed {
            assets = assets.small_image(small_key.to_string());
            if let Some(small_txt) = small_text_trimmed {
                assets = assets.small_text(format_discord_text(small_txt));
            }
        }
        activity = activity.assets(assets);
    }

    activity
}

pub struct DiscordClient {
    pub current_app_id: Option<String>,
    ipc: Option<DiscordIpcClient>,
    pub last_payload: Option<PresencePayload>,
    pub last_connect_attempt: Option<Instant>,
    pub connect_cooldown: Duration,
}

impl DiscordClient {
    pub fn new() -> Self {
        Self {
            current_app_id: None,
            ipc: None,
            last_payload: None,
            last_connect_attempt: None,
            connect_cooldown: Duration::from_secs(10),
        }
    }

    pub fn update_presence(
        &mut self,
        payload: &PresencePayload,
        now: Instant,
    ) -> Result<bool, String> {
        if self.last_payload.as_ref() == Some(payload) && self.ipc.is_some() {
            return Ok(false);
        }

        let app_id = payload.discord_app_id.trim();

        if self.current_app_id.as_deref() != Some(app_id) {
            self.disconnect();
        }

        if self.ipc.is_none() {
            if let Some(last_attempt) = self.last_connect_attempt
                && now.duration_since(last_attempt) < self.connect_cooldown
            {
                return Err("Discord reconnect cooldown active".to_string());
            }

            self.last_connect_attempt = Some(now);
            let mut client = DiscordIpcClient::new(app_id);
            match client.connect() {
                Ok(()) => {
                    self.ipc = Some(client);
                    self.current_app_id = Some(app_id.to_string());
                    self.last_connect_attempt = None;
                }
                Err(err) => {
                    return Err(format!("Failed to connect to Discord IPC: {}", err));
                }
            }
        }

        let activity = build_activity(payload);

        if let Some(client) = self.ipc.as_mut() {
            match client.set_activity(activity) {
                Ok(()) => {
                    self.last_payload = Some(payload.clone());
                    Ok(true)
                }
                Err(err) => {
                    let _ = client.close();
                    self.ipc = None;
                    self.current_app_id = None;
                    self.last_payload = None;
                    self.last_connect_attempt = Some(now);
                    Err(format!("Failed to set activity (connection dropped): {}", err))
                }
            }
        } else {
            Err("Discord client unavailable".to_string())
        }
    }

    pub fn clear_presence(&mut self) -> Result<(), String> {
        if self.last_payload.is_none() {
            return Ok(());
        }

        self.last_payload = None;
        if let Some(client) = self.ipc.as_mut()
            && let Err(err) = client.clear_activity()
        {
            let _ = client.close();
            self.ipc = None;
            self.current_app_id = None;
            return Err(format!("Failed to clear activity: {}", err));
        }

        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(mut client) = self.ipc.take() {
            let _ = client.clear_activity();
            let _ = client.close();
        }
        self.current_app_id = None;
        self.last_payload = None;
    }
}

impl Drop for DiscordClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_app(hide_title: bool, strip_words: Vec<&str>) -> AppConfig {
        AppConfig {
            app_name: "Blender".to_string(),
            process_name: "blender.exe".to_string(),
            discord_app_id: "1182390998877665544".to_string(),
            logo_key: "blender_logo".to_string(),
            logo_text: "Blender 3D".to_string(),
            default_details: "Working on 3D project".to_string(),
            state_text: "3D Viewport".to_string(),
            hide_window_title: hide_title,
            keep_active_in_background: true,
            strip_title_words: strip_words.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn format_discord_text_pads_short_strings_to_min_two_chars() {
        assert_eq!(format_discord_text(""), "  ");
        assert_eq!(format_discord_text("A"), "A ");
        assert_eq!(format_discord_text("AB"), "AB");
    }

    #[test]
    fn format_discord_text_truncates_utf8_safely_at_120_bytes() {
        let mut s = "a".repeat(119);
        s.push('🦀'); // 4 bytes: total 123 bytes, boundary at 119
        let formatted = format_discord_text(&s);
        assert!(formatted.len() <= 120);
        assert_eq!(formatted, "a".repeat(119));
    }

    #[test]
    fn sanitize_title_respects_privacy_flag() {
        let app = sample_app(true, vec![]);
        let result = sanitize_title(r"Blender - [D:\Secret_Client\model.blend]", &app);
        assert_eq!(result, "Working on 3D project");
    }

    #[test]
    fn sanitize_title_strips_folder_paths_and_noise_words() {
        let app = sample_app(false, vec!["Blender"]);
        let result = sanitize_title(r"Blender - [D:\Secret_Client\project_new.blend]", &app);
        assert_eq!(result, "[project_new.blend]");

        let ps_app = sample_app(
            false,
            vec![" - Adobe Photoshop", "@ 100%", "(RGB/8)", "*"],
        );
        let ps_result = sanitize_title("poster_event.psd @ 100% (RGB/8) * - Adobe Photoshop", &ps_app);
        assert_eq!(ps_result, "poster_event.psd");
    }

    #[test]
    fn sanitize_title_falls_back_to_default_details_when_empty() {
        let app = sample_app(false, vec!["Blender"]);
        assert_eq!(sanitize_title("Blender", &app), "Working on 3D project");
        assert_eq!(sanitize_title("   ", &app), "Working on 3D project");
    }

    #[test]
    fn sanitize_title_strips_unix_and_windows_paths() {
        let app = sample_app(false, vec![]);
        let result = sanitize_title(r"C:\Users\Artix\Documents\secret\scene.blend", &app);
        assert_eq!(result, "scene.blend");

        let result_slash = sanitize_title("D:/Projects/Confidential/edit.mp4 - CapCut", &app);
        assert_eq!(result_slash, "edit.mp4 - CapCut");

        let result_spaces = sanitize_title(r"D:\My Secret Client\Final Cut\video edit.mp4 - CapCut", &app);
        assert_eq!(result_spaces, "video edit.mp4 - CapCut");
    }

    #[test]
    fn build_activity_omits_empty_assets_and_formats_text() {
        let empty_assets_payload = PresencePayload {
            discord_app_id: "1182390998877665544".to_string(),
            details: "A".to_string(),
            state: "Working".to_string(),
            logo_key: "".to_string(),
            logo_text: "".to_string(),
            start_timestamp: 1_700_000_000,
            small_image: None,
            small_text: None,
        };

        let json = serde_json::to_value(build_activity(&empty_assets_payload))
            .expect("activity must serialize");
        assert_eq!(json["details"], "A ");
        assert_eq!(json["state"], "Working");
        assert!(json.get("assets").is_none());

        let with_key_only = PresencePayload {
            logo_key: "blender_logo".to_string(),
            ..empty_assets_payload
        };
        let json2 =
            serde_json::to_value(build_activity(&with_key_only)).expect("activity must serialize");
        assert_eq!(json2["assets"]["large_image"], "blender_logo");
        assert!(json2["assets"].get("large_text").is_none());
    }

    #[test]
    fn build_activity_attaches_small_image_and_text_when_present() {
        let payload = PresencePayload {
            discord_app_id: "1182390998877665544".to_string(),
            details: "Working".to_string(),
            state: "Active".to_string(),
            logo_key: "blender_logo".to_string(),
            logo_text: "Blender 3D".to_string(),
            start_timestamp: 1_700_000_000,
            small_image: Some("https://larplife.app/badge.png".to_string()),
            small_text: Some("LarpLife Active".to_string()),
        };

        let json = serde_json::to_value(build_activity(&payload))
            .expect("activity must serialize");
        assert_eq!(json["assets"]["large_image"], "blender_logo");
        assert_eq!(json["assets"]["large_text"], "Blender 3D");
        assert_eq!(json["assets"]["small_image"], "https://larplife.app/badge.png");
        assert_eq!(json["assets"]["small_text"], "LarpLife Active");
    }

    #[test]
    fn build_activity_attaches_small_image_without_large_image() {
        let payload = PresencePayload {
            discord_app_id: "1182390998877665544".to_string(),
            details: "Coding".to_string(),
            state: "Active".to_string(),
            logo_key: "".to_string(),
            logo_text: "".to_string(),
            start_timestamp: 1_700_000_000,
            small_image: Some("larplife_badge".to_string()),
            small_text: Some("LarpLife".to_string()),
        };

        let json = serde_json::to_value(build_activity(&payload))
            .expect("activity must serialize");
        assert!(json["assets"].get("large_image").is_none());
        assert_eq!(json["assets"]["small_image"], "larplife_badge");
        assert_eq!(json["assets"]["small_text"], "LarpLife");
    }

    #[test]
    fn build_activity_omits_small_image_and_text_when_none() {
        let payload = PresencePayload {
            discord_app_id: "1182390998877665544".to_string(),
            details: "Coding".to_string(),
            state: "Active".to_string(),
            logo_key: "blender_logo".to_string(),
            logo_text: "".to_string(),
            start_timestamp: 1_700_000_000,
            small_image: None,
            small_text: None,
        };

        let json = serde_json::to_value(build_activity(&payload))
            .expect("activity must serialize");
        assert_eq!(json["assets"]["large_image"], "blender_logo");
        assert!(json["assets"].get("small_image").is_none());
        assert!(json["assets"].get("small_text").is_none());
    }

    #[test]
    fn format_fallback_details_formats_app_and_details_cleanly() {
        assert_eq!(
            format_fallback_details("Cursor", "project.rs"),
            "Cursor — project.rs"
        );
        assert_eq!(
            format_fallback_details("Notepad", "notes.txt"),
            "Notepad — notes.txt"
        );
    }

    #[test]
    fn format_fallback_details_returns_app_name_when_details_empty() {
        assert_eq!(format_fallback_details("Cursor", ""), "Cursor");
        assert_eq!(format_fallback_details("Cursor", "   "), "Cursor");
        assert_eq!(format_fallback_details("  Cursor  ", ""), "Cursor");
        assert_eq!(format_fallback_details("", "project.rs"), "project.rs");
        assert_eq!(format_fallback_details("", ""), "");
    }

    #[test]
    fn format_fallback_details_avoids_duplicate_prefix_if_details_already_contain_app_name() {
        assert_eq!(
            format_fallback_details("Cursor", "Cursor — project.rs"),
            "Cursor — project.rs"
        );
        assert_eq!(
            format_fallback_details("Blender", "blender scene.blend"),
            "blender scene.blend"
        );
        assert_eq!(
            format_fallback_details("Photoshop", "Photoshop"),
            "Photoshop"
        );
        assert_eq!(
            format_fallback_details("VSCode", "Editing in VSCode workspace"),
            "Editing in VSCode workspace"
        );
    }

    #[test]
    fn build_activity_ignores_whitespace_only_badge_assets() {
        let payload = PresencePayload {
            discord_app_id: "1182390998877665544".to_string(),
            details: "Coding".to_string(),
            state: "Active".to_string(),
            logo_key: "".to_string(),
            logo_text: "".to_string(),
            start_timestamp: 1_700_000_000,
            small_image: Some("   ".to_string()),
            small_text: Some("   ".to_string()),
        };

        let json = serde_json::to_value(build_activity(&payload))
            .expect("activity must serialize");
        assert!(json.get("assets").is_none());
    }

    #[test]
    fn build_activity_formats_asset_texts_with_min_two_chars() {
        let payload = PresencePayload {
            discord_app_id: "1182390998877665544".to_string(),
            details: "Coding".to_string(),
            state: "Active".to_string(),
            logo_key: "logo_key".to_string(),
            logo_text: "X".to_string(),
            start_timestamp: 1_700_000_000,
            small_image: Some("small_badge".to_string()),
            small_text: Some("Y".to_string()),
        };

        let json = serde_json::to_value(build_activity(&payload))
            .expect("activity must serialize");
        assert_eq!(json["assets"]["large_text"], "X ");
        assert_eq!(json["assets"]["small_text"], "Y ");
    }
}
