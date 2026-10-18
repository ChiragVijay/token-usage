use gpui::{App, Window};

/// Native window title. The Windows single-instance lookup uses this exact text.
pub const WINDOW_TITLE: &str = "TokenDock";

/// Switch to a menu-bar app only after its tray icon is available.
pub fn configure_tray_app() {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        let mtm = MainThreadMarker::new().expect("GPUI runs on the main thread");
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

/// Hide the native window without minimizing it or destroying the dashboard.
pub fn hide(window: &Window, cx: &App) {
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = native_handle(window) {
        use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindowAsync};
        // Queue visibility changes so native callbacks cannot re-enter GPUI's update.
        // Hiding removes the taskbar entry without changing the window's placement.
        unsafe {
            let _ = ShowWindowAsync(hwnd, SW_HIDE);
        }
    }
    #[cfg(target_os = "macos")]
    cx.hide();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    window.minimize_window();
    let _ = (window, cx);
}

/// Explicitly show a hidden window before asking GPUI to focus it.
pub fn show(window: &Window, cx: &App) {
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = native_handle(window) {
        use windows::Win32::UI::WindowsAndMessaging::ShowWindowAsync;
        unsafe {
            let _ = ShowWindowAsync(hwnd, visibility_command(hwnd));
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        let mtm = MainThreadMarker::new().expect("GPUI runs on the main thread");
        NSApplication::sharedApplication(mtm).unhideWithoutActivation();
    }
    cx.activate(true);
    window.activate_window();
}

#[cfg(target_os = "windows")]
fn native_handle(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(windows::Win32::Foundation::HWND(
            handle.hwnd.get() as *mut std::ffi::c_void
        )),
        _ => None,
    }
}

/// Clear Windows' maximized/minimized placement before GPUI changes content size.
/// SetWindowPos alone retains that placement and restores the wrong size later.
pub fn restore_before_resize(window: &Window) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            IsIconic, IsZoomed, SW_RESTORE, ShowWindowAsync,
        };
        if let Some(hwnd) = native_handle(window) {
            // The handle belongs to this GPUI window and remains valid during this call.
            unsafe {
                if IsZoomed(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                    return ShowWindowAsync(hwnd, SW_RESTORE).as_bool();
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
    false
}

/// `Some(true)` opens the window in this process. `Some(false)` means another
/// window was restored and this process should exit. `None` means the other
/// process still holds the lock and its window is not up yet.
#[cfg(any(windows, test))]
fn single_instance_action(owns_lock: bool, found_window: bool) -> Option<bool> {
    if owns_lock {
        Some(true)
    } else if found_window {
        Some(false)
    } else {
        None
    }
}

/// Take the per-session window lock. Returns false when another live window
/// process holds it; that window is brought forward and the caller exits.
///
/// Call this only after the per-user install handoff. The installer starts the
/// installed executable and exits first, so it never holds the lock.
///
/// The lock is taken before the window exists. A second launch keeps its mutex
/// handle and waits. If that owner exits first, this launch takes the abandoned
/// mutex and opens the window instead of leaving none.
#[cfg(windows)]
pub fn claim_single_instance() -> bool {
    use windows::{
        Win32::{
            Foundation::{
                CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, SetLastError, WIN32_ERROR,
            },
            System::Threading::CreateMutexW,
        },
        core::w,
    };

    const ATTEMPTS: u32 = 50;
    const WAIT_MS: u32 = 100;

    let handle = unsafe {
        // CreateMutex leaves a stale last-error when it creates the object.
        SetLastError(WIN32_ERROR(0));
        match CreateMutexW(None, true, w!("Local\\dev.chirag.token-dock")) {
            Ok(handle) => handle,
            // Opening the window is safer than exiting when the lock cannot be created.
            Err(_) => return true,
        }
    };
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        // HANDLE does not close on drop. Keep the kernel object until this
        // process exits so a later launch can find the window.
        let _held = Box::leak(Box::new(handle));
        return true;
    }

    // bInitialOwner does not grant ownership when the mutex already exists.
    // Keep this handle open: if the owner exits, the mutex stays abandoned
    // instead of disappearing, and WaitForSingleObject reports that.
    for _ in 0..ATTEMPTS {
        let found = find_usage_window();
        let found_window = found.is_some();
        if let Some(hwnd) = found {
            bring_forward(hwnd);
        }
        let owns = acquired(handle, if found_window { 0 } else { WAIT_MS });
        match single_instance_action(owns, found_window) {
            Some(true) => {
                let _held = Box::leak(Box::new(handle));
                return true;
            }
            Some(false) => {
                let _ = unsafe { CloseHandle(handle) };
                return false;
            }
            None => {}
        }
    }

    let found = find_usage_window();
    let found_window = found.is_some();
    if let Some(hwnd) = found {
        bring_forward(hwnd);
    }
    if single_instance_action(acquired(handle, 0), found_window) == Some(true) {
        let _held = Box::leak(Box::new(handle));
        return true;
    }
    let _ = unsafe { CloseHandle(handle) };
    false
}

#[cfg(windows)]
fn find_usage_window() -> Option<windows::Win32::Foundation::HWND> {
    use windows::{Win32::UI::WindowsAndMessaging::FindWindowW, core::HSTRING, core::w};
    unsafe { FindWindowW(w!("Zed::Window"), &HSTRING::from(WINDOW_TITLE)).ok() }
}

#[cfg(windows)]
fn acquired(handle: windows::Win32::Foundation::HANDLE, wait_ms: u32) -> bool {
    use windows::Win32::{
        Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0},
        System::Threading::WaitForSingleObject,
    };
    let status = unsafe { WaitForSingleObject(handle, wait_ms) };
    status == WAIT_OBJECT_0 || status == WAIT_ABANDONED
}

#[cfg(target_os = "windows")]
fn bring_forward(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::{
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::WindowsAndMessaging::{
            BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
            ShowWindow,
        },
    };
    unsafe {
        let _ = ShowWindow(hwnd, visibility_command(hwnd));
        let foreground = GetForegroundWindow();
        let current = GetCurrentThreadId();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let target_thread = GetWindowThreadProcessId(hwnd, None);
        let attached_foreground = foreground_thread != 0
            && foreground_thread != current
            && AttachThreadInput(current, foreground_thread, true).as_bool();
        let attached_target = target_thread != 0
            && target_thread != current
            && target_thread != foreground_thread
            && AttachThreadInput(current, target_thread, true).as_bool();
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
        if attached_target {
            let _ = AttachThreadInput(current, target_thread, false);
        }
        if attached_foreground {
            let _ = AttachThreadInput(current, foreground_thread, false);
        }
    }
}

#[cfg(target_os = "windows")]
fn visibility_command(
    hwnd: windows::Win32::Foundation::HWND,
) -> windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, SW_SHOW};
    if unsafe { IsIconic(hwnd).as_bool() } {
        SW_RESTORE
    } else {
        SW_SHOW
    }
}

#[cfg(test)]
mod tests {
    use super::{WINDOW_TITLE, single_instance_action};

    #[test]
    fn window_title_is_the_single_instance_lookup_text() {
        assert_eq!(WINDOW_TITLE, "TokenDock");
        assert!(WINDOW_TITLE.is_ascii());
    }

    #[test]
    fn a_second_launch_leaves_when_it_can_see_the_window() {
        assert_eq!(single_instance_action(false, true), Some(false));
    }

    #[test]
    fn a_second_launch_opens_when_the_owner_has_released_the_lock() {
        assert_eq!(single_instance_action(true, false), Some(true));
        assert_eq!(single_instance_action(true, true), Some(true));
    }

    #[test]
    fn a_second_launch_waits_while_the_owner_has_no_window() {
        assert_eq!(single_instance_action(false, false), None);
    }
}
