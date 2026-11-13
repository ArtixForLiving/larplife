use crate::logger::{log_error, log_info, show_error_dialog};
use crate::startup::{is_run_at_startup_enabled, set_run_at_startup};
use crate::worker::{WorkerCommand, WorkerEvent};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, KillTimer, LoadIconW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer,
    TrackPopupMenu, TranslateMessage, IDI_APPLICATION, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    MF_UNCHECKED, MSG, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_COMMAND,
    WM_DESTROY, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

pub const ID_STATUS_TOGGLE: usize = 1001;
pub const ID_RELOAD_CONFIG: usize = 1002;
pub const ID_STARTUP_TOGGLE: usize = 1003;
pub const ID_EXIT: usize = 1004;
pub const ID_OPEN_SETTINGS: usize = 1005;

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const TIMER_EVENT_POLL_ID: usize = 1;
const APP_NAME: &str = "larplife";

static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub fn build_status_menu_text(paused: bool) -> &'static str {
    if paused {
        "Status: Paused (Click to Resume)"
    } else {
        "Status: Active (Click to Pause)"
    }
}

pub fn build_startup_menu_text(enabled: bool) -> &'static str {
    if enabled {
        "Run at Windows Startup [Enabled]"
    } else {
        "Run at Windows Startup [Disabled]"
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

struct TrayState {
    paused: bool,
    cmd_tx: Sender<WorkerCommand>,
    event_rx: Receiver<WorkerEvent>,
}

static GLOBAL_STATE: Mutex<Option<TrayState>> = Mutex::new(None);

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar_created = WM_TASKBAR_CREATED.load(Ordering::Relaxed);
    if taskbar_created != 0 && msg == taskbar_created {
        // Explorer restarted: re-register the tray icon
        unsafe { register_tray_icon(hwnd) };
        return LRESULT(0);
    }

    match msg {
        WM_TRAY_CALLBACK => {
            let event = lparam.0 as u32;
            if event == WM_RBUTTONUP {
                unsafe { show_context_menu(hwnd) };
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let command_id = wparam.0;
            unsafe { handle_menu_command(hwnd, command_id) };
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_EVENT_POLL_ID {
                poll_worker_events();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_EVENT_POLL_ID);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn poll_worker_events() {
    if let Ok(mut guard) = GLOBAL_STATE.lock()
        && let Some(state) = guard.as_mut()
    {
        while let Ok(event) = state.event_rx.try_recv() {
            match event {
                WorkerEvent::ConfigReloaded {
                    success,
                    app_count,
                    error_message,
                } => {
                    if success {
                        log_info(&format!("Tray received config reload: {} apps loaded", app_count));
                    } else if let Some(err) = error_message {
                        show_error_dialog("Larplife Configuration Error", &err);
                    }
                }
                WorkerEvent::StatusChanged { paused, .. } => {
                    state.paused = paused;
                }
            }
        }
    }
}

unsafe fn show_context_menu(hwnd: HWND) {
    let paused = if let Ok(guard) = GLOBAL_STATE.lock() {
        guard.as_ref().map(|s| s.paused).unwrap_or(false)
    } else {
        false
    };

    let hmenu = match unsafe { CreatePopupMenu() } {
        Ok(m) => m,
        Err(_) => return,
    };

    let status_text = to_wide(build_status_menu_text(paused));
    let settings_text = to_wide("Settings...");
    let reload_text = to_wide("Reload Configuration");
    let is_startup = is_run_at_startup_enabled(APP_NAME);
    let startup_text = to_wide(build_startup_menu_text(is_startup));
    let exit_text = to_wide("Exit");

    unsafe {
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            ID_OPEN_SETTINGS,
            PCWSTR(settings_text.as_ptr()),
        );
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            ID_STATUS_TOGGLE,
            PCWSTR(status_text.as_ptr()),
        );
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            ID_RELOAD_CONFIG,
            PCWSTR(reload_text.as_ptr()),
        );

        let startup_flags = if is_startup {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING | MF_UNCHECKED
        };
        let _ = AppendMenuW(
            hmenu,
            startup_flags,
            ID_STARTUP_TOGGLE,
            PCWSTR(startup_text.as_ptr()),
        );

        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(hmenu, MF_STRING, ID_EXIT, PCWSTR(exit_text.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);

        let _ = TrackPopupMenu(
            hmenu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );

        let _ = DestroyMenu(hmenu);
    }
}

unsafe fn handle_menu_command(hwnd: HWND, command_id: usize) {
    match command_id {
        ID_OPEN_SETTINGS => {
            if let Ok(current_exe) = std::env::current_exe() {
                let mut cmd = std::process::Command::new(&current_exe);
                cmd.arg("--settings");
                if let Some(parent) = current_exe.parent() {
                    cmd.current_dir(parent);
                }
                let _ = cmd.spawn();
            }
        }
        ID_STATUS_TOGGLE => {
            if let Ok(guard) = GLOBAL_STATE.lock()
                && let Some(state) = guard.as_ref()
            {
                let _ = state.cmd_tx.send(WorkerCommand::TogglePause);
            }
        }
        ID_RELOAD_CONFIG => {
            if let Ok(guard) = GLOBAL_STATE.lock()
                && let Some(state) = guard.as_ref()
            {
                let _ = state.cmd_tx.send(WorkerCommand::ReloadConfig);
            }
        }
        ID_STARTUP_TOGGLE => {
            let current = is_run_at_startup_enabled(APP_NAME);
            if let Err(err) = set_run_at_startup(APP_NAME, !current) {
                log_error(&format!("Failed to toggle startup: {}", err));
                show_error_dialog("Startup Registry Error", &err);
            }
        }
        ID_EXIT => {
            if let Ok(guard) = GLOBAL_STATE.lock()
                && let Some(state) = guard.as_ref()
            {
                let _ = state.cmd_tx.send(WorkerCommand::Shutdown);
            }
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        _ => {}
    }
}

unsafe fn register_tray_icon(hwnd: HWND) -> bool {
    let hicon = match unsafe { LoadIconW(None, IDI_APPLICATION) } {
        Ok(h) => h,
        Err(_) => return false,
    };

    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY_CALLBACK,
        hIcon: hicon,
        ..Default::default()
    };

    let tip = to_wide("larplife - Discord RPC");
    let copy_len = tip.len().min(nid.szTip.len());
    nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);

    unsafe { Shell_NotifyIconW(NIM_ADD, &nid).as_bool() }
}

pub fn notify_background_process_reload() {
    let class_name = w!("LarplifeTrayWindowClass");
    if let Ok(hwnd) = unsafe { FindWindowW(class_name, PCWSTR::null()) }
        && !hwnd.is_invalid()
        && !hwnd.0.is_null()
    {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(ID_RELOAD_CONFIG), LPARAM(0));
        }
    }
}

pub fn run_tray_ui(
    cmd_tx: Sender<WorkerCommand>,
    event_rx: Receiver<WorkerEvent>,
) -> Result<(), String> {
    if let Ok(mut guard) = GLOBAL_STATE.lock() {
        *guard = Some(TrayState {
            paused: false,
            cmd_tx,
            event_rx,
        });
    }

    let hinstance = unsafe { GetModuleHandleW(None) }
        .map_err(|e| format!("Failed to get module handle: {}", e))?;

    let class_name = w!("LarplifeTrayWindowClass");
    let window_title = w!("Larplife Tray Message Window");

    let hicon = unsafe { LoadIconW(None, IDI_APPLICATION) }
        .map_err(|e| format!("Failed to load system icon: {}", e))?;

    let wnd_class = WNDCLASSW {
        hInstance: hinstance.into(),
        lpfnWndProc: Some(tray_window_proc),
        lpszClassName: class_name,
        hIcon: hicon,
        ..Default::default()
    };

    let atom = unsafe { RegisterClassW(&wnd_class) };
    if atom == 0 {
        return Err("Failed to register tray window class".to_string());
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            window_title,
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .map_err(|e| format!("Failed to create tray window: {}", e))?;

    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    WM_TASKBAR_CREATED.store(taskbar_created, Ordering::Relaxed);

    // Attempt to register tray icon with retry loop
    let mut icon_added = false;
    for _ in 0..5 {
        if unsafe { register_tray_icon(hwnd) } {
            icon_added = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    if icon_added {
        log_info("System tray icon added successfully.");
    } else {
        log_info("System tray icon not available in current desktop session; background activity tracker remains active.");
    }

    unsafe {
        SetTimer(Some(hwnd), TIMER_EVENT_POLL_ID, 500, None);
    }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        let del_nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &del_nid);
    }

    if let Ok(mut guard) = GLOBAL_STATE.lock() {
        *guard = None;
    }

    log_info("Tray message loop terminated.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_clear_status_and_startup_menu_labels() {
        assert_eq!(
            build_status_menu_text(false),
            "Status: Active (Click to Pause)"
        );
        assert_eq!(
            build_status_menu_text(true),
            "Status: Paused (Click to Resume)"
        );

        assert_eq!(
            build_startup_menu_text(true),
            "Run at Windows Startup [Enabled]"
        );
        assert_eq!(
            build_startup_menu_text(false),
            "Run at Windows Startup [Disabled]"
        );
    }
}
