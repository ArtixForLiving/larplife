use crate::window_reader::{extract_exe_name, is_transient_process};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAppInfo {
    pub process_name: String,
    pub window_title: String,
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return BOOL(1);
    }

    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    if title_len <= 0 {
        return BOOL(1);
    }

    let mut title_buf = vec![0u16; (title_len as usize) + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut title_buf) };
    if copied <= 0 {
        return BOOL(1);
    }
    let title = String::from_utf16_lossy(&title_buf[..copied as usize]);
    let trimmed_title = title.trim();
    if trimmed_title.is_empty() {
        return BOOL(1);
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return BOOL(1);
    }

    if let Ok(handle) = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
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
            let exe_name = extract_exe_name(&full_path).to_string();
            if !exe_name.is_empty() && !is_transient_process(&exe_name) {
                let list = unsafe { &mut *(lparam.0 as *mut Vec<RunningAppInfo>) };
                if !list.iter().any(|item| {
                    item.process_name.eq_ignore_ascii_case(&exe_name)
                        && item.window_title == trimmed_title
                }) {
                    list.push(RunningAppInfo {
                        process_name: exe_name,
                        window_title: trimmed_title.to_string(),
                    });
                }
            }
        }
        let _ = unsafe { CloseHandle(handle) };
    }

    BOOL(1)
}

pub fn get_running_applications() -> Vec<RunningAppInfo> {
    let mut apps: Vec<RunningAppInfo> = Vec::new();

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut apps as *mut Vec<RunningAppInfo> as isize),
        );
    }

    // Fallback: If no visible window found (e.g. headless environment), enumerate processes via Toolhelp32
    if apps.is_empty() {
        if let Ok(snapshot) = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
            if snapshot != INVALID_HANDLE_VALUE {
                let mut entry = PROCESSENTRY32W {
                    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };

                if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
                    loop {
                        let name_len = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);

                        if !exe_name.trim().is_empty()
                            && !is_transient_process(&exe_name)
                            && !apps
                                .iter()
                                .any(|a| a.process_name.eq_ignore_ascii_case(&exe_name))
                        {
                            apps.push(RunningAppInfo {
                                process_name: exe_name.clone(),
                                window_title: exe_name,
                            });
                        }

                        if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                            break;
                        }
                    }
                }
                let _ = unsafe { CloseHandle(snapshot) };
            }
        }
    }

    apps.sort_by(|a, b| a.process_name.to_lowercase().cmp(&b.process_name.to_lowercase()));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_running_applications_returns_non_empty_list() {
        let apps = get_running_applications();
        assert!(!apps.is_empty(), "should find running applications");
        for app in &apps {
            assert!(!app.process_name.trim().is_empty());
            assert!(!app.window_title.trim().is_empty());
        }
    }
}
