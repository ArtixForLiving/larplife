use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

pub fn is_run_at_startup_enabled(value_name: &str) -> bool {
    let wide_subkey = to_wide(RUN_SUBKEY);
    let mut hkey = HKEY::default();

    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide_subkey.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        )
    };

    if open_status.is_err() {
        return false;
    }

    let wide_val = to_wide(value_name);
    let query_status = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(wide_val.as_ptr()),
            None,
            None,
            None,
            None,
        )
    };

    let _ = unsafe { RegCloseKey(hkey) };
    query_status.is_ok()
}

pub fn set_run_at_startup(value_name: &str, enabled: bool) -> Result<(), String> {
    let wide_subkey = to_wide(RUN_SUBKEY);
    let wide_val = to_wide(value_name);
    let mut hkey = HKEY::default();

    if enabled {
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("Failed to get current executable path: {}", e))?;
        let quoted_path = format!("\"{}\"", exe_path.display());
        let wide_path = to_wide(&quoted_path);

        unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(wide_subkey.as_ptr()),
                None,
                KEY_WRITE,
                &mut hkey,
            )
            .ok()
            .map_err(|e| format!("Failed to open registry key for write: {}", e))?;

            let bytes = std::slice::from_raw_parts(
                wide_path.as_ptr() as *const u8,
                wide_path.len() * std::mem::size_of::<u16>(),
            );

            let res = RegSetValueExW(
                hkey,
                PCWSTR(wide_val.as_ptr()),
                None,
                REG_SZ,
                Some(bytes),
            );

            let _ = RegCloseKey(hkey);
            res.ok()
                .map_err(|e| format!("Failed to write registry value: {}", e))?;
        }
    } else {
        unsafe {
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(wide_subkey.as_ptr()),
                None,
                KEY_WRITE,
                &mut hkey,
            )
            .is_ok()
            {
                let _ = RegDeleteValueW(hkey, PCWSTR(wide_val.as_ptr()));
                let _ = RegCloseKey(hkey);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_startup_registry_value_cleanly() {
        let test_key = "larplife_test_entry_xyz";

        // Cleanup before test
        let _ = set_run_at_startup(test_key, false);
        assert!(!is_run_at_startup_enabled(test_key));

        // Enable
        set_run_at_startup(test_key, true).expect("enable startup entry");
        assert!(is_run_at_startup_enabled(test_key));

        // Disable
        set_run_at_startup(test_key, false).expect("disable startup entry");
        assert!(!is_run_at_startup_enabled(test_key));
    }
}
