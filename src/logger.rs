use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

pub const DEFAULT_LOG_FILE: &str = "larplife.log";

fn secs_to_utc_parts(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let second = (secs % 60) as u32;
    let minute = ((secs / 60) % 60) as u32;
    let hour = ((secs / 3600) % 24) as u32;
    let days = (secs / 86400) as i64 + 719468;

    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    (year as u32, month, day, hour, minute, second)
}

pub fn format_log_entry(timestamp_secs: u64, level: &str, message: &str) -> String {
    let (y, m, d, hh, mm, ss) = secs_to_utc_parts(timestamp_secs);
    format!(
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] [{}] {}\n",
        y, m, d, hh, mm, ss, level, message
    )
}

pub fn log_message_to_file(path: &Path, level: &str, message: &str) -> std::io::Result<()> {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entry = format_log_entry(now_secs, level, message);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    file.write_all(entry.as_bytes())?;
    file.flush()?;
    Ok(())
}

pub fn log_info(message: &str) {
    let _ = log_message_to_file(Path::new(DEFAULT_LOG_FILE), "INFO", message);
}

pub fn log_error(message: &str) {
    let _ = log_message_to_file(Path::new(DEFAULT_LOG_FILE), "ERROR", message);
}

pub fn show_error_dialog(title: &str, message: &str) {
    let wide_title: Vec<u16> = OsStr::new(title)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let wide_message: Vec<u16> = OsStr::new(message)
        .encode_wide()
        .chain(Some(0))
        .collect();

    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(wide_message.as_ptr()),
            PCWSTR(wide_title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn format_log_entry_formats_iso_like_utc_timestamp() {
        let formatted = format_log_entry(1_700_000_000, "ERROR", "Failed to load config");
        assert_eq!(
            formatted,
            "[2023-11-14 22:13:20] [ERROR] Failed to load config\n"
        );
    }

    #[test]
    fn log_message_to_file_appends_entries() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("larplife_test_log_{}.log", nanos));

        log_message_to_file(&path, "INFO", "Application initialized").expect("first write");
        log_message_to_file(&path, "ERROR", "Connection refused").expect("second write");

        let content = fs::read_to_string(&path).expect("read file");
        assert!(content.contains("[INFO] Application initialized\n"));
        assert!(content.contains("[ERROR] Connection refused\n"));

        let _ = fs::remove_file(path);
    }
}
