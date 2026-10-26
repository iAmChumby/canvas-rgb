//! The daemon and the tray never run elevated.
//!
//! SignalRGB and the fan service need different rights; the daemon needs none. If the
//! daemon is started with an elevated token it refuses to run, because an elevated
//! daemon would be able to write to the hardware the service owns.

/// Whether the current process token is elevated (has the Administrators group enabled).
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            // If the token cannot be opened, assume the worst and refuse.
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
            tracing::error!("could not read the token's elevation; refusing to run");
            return true;
        }
        elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

/// Why the daemon refused to start.
#[derive(Debug, thiserror::Error)]
#[error("canvas-rgbd must not run elevated (the token is elevated); run it as your normal user")]
pub struct ElevatedError;

impl ElevatedError {
    /// Checks the token and returns the error when the daemon is elevated.
    pub fn check() -> Result<(), Self> {
        if is_elevated() {
            Err(ElevatedError)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_elevation_check_answers() {
        // The build may run elevated or not: the check must answer either way. The
        // daemon refuses to start when it is elevated, which is what `check` enforces.
        let elevated = is_elevated();
        if elevated {
            assert!(ElevatedError::check().is_err(), "an elevated process is refused");
        } else {
            assert!(ElevatedError::check().is_ok(), "a normal process may start");
        }
    }
}
