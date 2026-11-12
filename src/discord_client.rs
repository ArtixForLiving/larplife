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

        if self.current_app_id.as_deref() != Some(&payload.discord_app_id) {
            self.disconnect();
        }

        if self.ipc.is_none() {
            if let Some(last_attempt) = self.last_connect_attempt
                && now.duration_since(last_attempt) < self.connect_cooldown
            {
                return Err("Discord reconnect cooldown active".to_string());
            }

            self.last_connect_attempt = Some(now);
            let mut client = DiscordIpcClient::new(&payload.discord_app_id);
            match client.connect() {
                Ok(()) => {
                    self.ipc = Some(client);
                    self.current_app_id = Some(payload.discord_app_id.clone());
                    self.last_connect_attempt = None;
                }
                Err(err) => {
                    return Err(format!("Failed to connect to Discord IPC: {}", err));
                }
            }
        }

        let formatted_details = format_discord_text(&payload.details);
        let formatted_state = format_discord_text(&payload.state);

        let activity = activity::Activity::new()
            .details(&formatted_details)
            .state(&formatted_state)
            .timestamps(activity::Timestamps::new().start(payload.start_timestamp))
            .assets(
                activity::Assets::new()
                    .large_image(&payload.logo_key)
                    .large_text(&payload.logo_text),
            );

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
}
