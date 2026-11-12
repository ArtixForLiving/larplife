use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetAncestor, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, GA_ROOTOWNER,
};

const TRANSIENT_PROCESSES: &[&str] = &[
    "ShellExperienceHost.exe",
    "SearchHost.exe",
    "LockApp.exe",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWindow {
    pub process_name: String,
    pub window_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForegroundReading {
    Window(ActiveWindow),
    Transient,
    AccessDenied,
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

pub fn extract_exe_name(full_path: &str) -> &str {
    full_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(full_path)
}

pub fn is_transient_process(exe_name: &str) -> bool {
    TRANSIENT_PROCESSES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(exe_name))
}

enum ProcessQueryResult {
    Found(String),
    AccessDenied,
    Unavailable,
}

fn query_process_exe(pid: u32) -> ProcessQueryResult {
    if pid == 0 {
        return ProcessQueryResult::Unavailable;
    }

    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => OwnedHandle(h),
        Err(err) => {
            if err.code() == ERROR_ACCESS_DENIED.to_hresult() {
                return ProcessQueryResult::AccessDenied;
            }
            return ProcessQueryResult::Unavailable;
        }
    };

    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;

    if unsafe {
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    }
    .is_err()
    {
        return ProcessQueryResult::Unavailable;
    }

    let full_path = OsString::from_wide(&buffer[..len as usize])
        .to_string_lossy()
        .into_owned();
    let exe_name = extract_exe_name(&full_path).to_string();
    if exe_name.is_empty() {
        ProcessQueryResult::Unavailable
    } else {
        ProcessQueryResult::Found(exe_name)
    }
}

fn read_window_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }

    let mut buffer = vec![0u16; (len as usize) + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(&buffer[..copied as usize])
}

struct UwpSearchContext {
    host_pid: u32,
    found_exe: Option<String>,
    access_denied: bool,
}

unsafe extern "system" fn enum_uwp_child(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lparam.0 as *mut UwpSearchContext) };
    let mut child_pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut child_pid)) };

    if child_pid != 0 && child_pid != ctx.host_pid {
        match query_process_exe(child_pid) {
            ProcessQueryResult::Found(exe) => {
                ctx.found_exe = Some(exe);
                return BOOL(0);
            }
            ProcessQueryResult::AccessDenied => {
                ctx.access_denied = true;
            }
            ProcessQueryResult::Unavailable => {}
        }
    }

    BOOL(1)
}

fn resolve_uwp_process(hwnd: HWND, host_pid: u32) -> ProcessQueryResult {
    let mut ctx = UwpSearchContext {
        host_pid,
        found_exe: None,
        access_denied: false,
    };

    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(enum_uwp_child),
            LPARAM(&mut ctx as *mut UwpSearchContext as isize),
        );
    }

    if let Some(exe) = ctx.found_exe {
        ProcessQueryResult::Found(exe)
    } else if ctx.access_denied {
        ProcessQueryResult::AccessDenied
    } else {
        ProcessQueryResult::Unavailable
    }
}

pub fn read_foreground_window() -> ForegroundReading {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() || hwnd.0.is_null() {
        return ForegroundReading::Transient;
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

    let process_name = match query_process_exe(pid) {
        ProcessQueryResult::Found(exe) => {
            if exe.eq_ignore_ascii_case("ApplicationFrameHost.exe") {
                match resolve_uwp_process(hwnd, pid) {
                    ProcessQueryResult::Found(real_exe) => real_exe,
                    ProcessQueryResult::AccessDenied => return ForegroundReading::AccessDenied,
                    ProcessQueryResult::Unavailable => exe,
                }
            } else {
                exe
            }
        }
        ProcessQueryResult::AccessDenied => return ForegroundReading::AccessDenied,
        ProcessQueryResult::Unavailable => return ForegroundReading::Transient,
    };

    if is_transient_process(&process_name) {
        return ForegroundReading::Transient;
    }

    // Prefer root owner window title when a child/dialog popup (e.g. Save As) is active.
    let root_hwnd = unsafe { GetAncestor(hwnd, GA_ROOTOWNER) };
    let window_title = if !root_hwnd.is_invalid() && !root_hwnd.0.is_null() {
        let root_title = read_window_title(root_hwnd);
        if root_title.is_empty() {
            read_window_title(hwnd)
        } else {
            root_title
        }
    } else {
        read_window_title(hwnd)
    };

    ForegroundReading::Window(ActiveWindow {
        process_name,
        window_title,
    })
}

pub fn is_process_running(target_exe: &str) -> bool {
    if target_exe.trim().is_empty() {
        return false;
    }

    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(h) if h != INVALID_HANDLE_VALUE => OwnedHandle(h),
        _ => return false,
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    if unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_err() {
        return false;
    }

    loop {
        let name_len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);
        if exe_name.eq_ignore_ascii_case(target_exe) {
            return true;
        }

        if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
            break;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_exe_name_from_windows_path() {
        assert_eq!(
            extract_exe_name(r"C:\Program Files\Adobe\Photoshop.exe"),
            "Photoshop.exe"
        );
        assert_eq!(extract_exe_name("blender.exe"), "blender.exe");
        assert_eq!(extract_exe_name(""), "");
    }

    #[test]
    fn identifies_transient_system_processes() {
        assert!(is_transient_process("ShellExperienceHost.exe"));
        assert!(is_transient_process("searchhost.exe"));
        assert!(is_transient_process("LOCKAPP.EXE"));
        assert!(!is_transient_process("Photoshop.exe"));
    }

    #[test]
    fn detects_running_current_process_and_missing_process() {
        let current_exe = std::env::current_exe()
            .expect("current_exe should resolve")
            .file_name()
            .expect("exe should have file name")
            .to_string_lossy()
            .into_owned();

        assert!(is_process_running(&current_exe));
        assert!(is_process_running(&current_exe.to_ascii_uppercase()));
        assert!(!is_process_running(
            "definitely_nonexistent_larplife_process_12345.exe"
        ));
    }

    #[test]
    fn read_foreground_window_does_not_panic() {
        let _ = read_foreground_window();
    }
}
