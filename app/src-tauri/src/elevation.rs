//! The tray never runs elevated (done-line). An elevated launch shows a message and exits.

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SAFETY: plain Win32 token queries on the current process. The out-pointers are to
    // locals that outlive the calls, and the token handle is not used after it is read.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            // Cannot read the token: refuse rather than guess.
            return true;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::addr_of_mut!(elevation).cast::<core::ffi::c_void>()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        if ok.is_err() {
            return true;
        }
        elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

/// Tells the user why canvas-rgb did not start.
#[cfg(windows)]
pub fn refuse() {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    // SAFETY: both strings are static, NUL-terminated wide literals; no owner handle.
    unsafe {
        MessageBoxW(
            None,
            w!("canvas-rgb must not run elevated. Start it as your normal user; the fan service runs the privileged part."),
            w!("canvas-rgb"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
pub fn refuse() {}
