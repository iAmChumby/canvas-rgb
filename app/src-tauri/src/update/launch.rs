//! Running the downloaded installer. The file is opened denying write and delete to everyone
//! else, read back, checked against the release key once more, and started while that handle is
//! held, so nothing can swap the file between the check and the start.

use std::io::Read;
use std::path::Path;

/// Inno Setup switches. `/UPDATE` makes the setup relaunch the tray with `--after-update` and
/// write its result file. No `/SUPPRESSMSGBOXES`: a failure page must be able to show.
pub const INSTALLER_ARGS: &str = "/SILENT /NORESTART /UPDATE";

/// `ERROR_CANCELLED`: the person answered No to the permission prompt.
#[cfg(windows)]
const ERROR_CANCELLED: u32 = 1223;

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    Declined,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// The file failed its signature check and was deleted.
    VerifyFailed,
    Declined,
    Failed(String),
}

/// The started installer process, kept so the tray can see it end. A raw handle value so it can
/// cross threads; closed by [`wait_and_close`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessHandle(pub isize);

/// What starting the installer gives back: the process, when the platform handed one out.
pub type Started = Option<ProcessHandle>;

#[cfg(windows)]
fn open_deny_write(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_SHARE_READ only: no other handle may write, delete or rename the file while this lives.
    std::fs::OpenOptions::new().read(true).share_mode(1).open(path)
}

#[cfg(not(windows))]
fn open_deny_write(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Opens `path` denying write/delete, reads it, runs `verify` on the bytes and, when they pass,
/// `run` with the handle still open. A failed check deletes the file.
pub fn run_locked(
    path: &Path,
    verify: &dyn Fn(&[u8]) -> bool,
    run: &dyn Fn(&Path) -> Result<Started, RunError>,
) -> Result<Started, LaunchError> {
    let mut file = open_deny_write(path).map_err(|e| LaunchError::Failed(format!("could not open the installer: {e}")))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| LaunchError::Failed(format!("could not read the installer: {e}")))?;
    if !verify(&bytes) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(LaunchError::VerifyFailed);
    }
    let outcome = run(path);
    drop(file);
    outcome.map_err(|e| match e {
        RunError::Declined => LaunchError::Declined,
        RunError::Failed(m) => LaunchError::Failed(m),
    })
}

/// The verb the installer is started with: none, so Windows runs it with the tray's own,
/// non-elevated token (the default "open" action). Setup has `PrivilegesRequired=admin` and raises
/// itself with a permission prompt of its own; because that first Setup instance was not elevated,
/// `ExecAsOriginalUser` can start the tray again as this same non-elevated user (#28). An elevating
/// verb here would start Setup already elevated and the tray it relaunches would be elevated too.
/// Inno Setup 6: PrivilegesRequired ("Windows will ask for the password to an account that has
/// administrative privileges") and ExecAsOriginalUser ("the (normally non-elevated) credentials of
/// the user that started Setup initially").
pub const INSTALLER_VERB: Option<&str> = None;

/// Setup's exit code for "the user cancelled before installation started"
/// (Inno Setup "Setup Exit Codes", code 2). The permission prompt Setup raises for itself is the
/// only prompt in this flow, so a Setup that ends with this code and wrote no result was declined.
/// UNKNOWN: whether Inno returns exactly 2 for a declined elevation prompt; its documentation does
/// not say. Settle by declining the prompt on a real update.
pub const SETUP_CANCELLED_CODE: u32 = 2;

/// What [`start_non_elevated`] hands to Windows, kept apart from the call so a test can see it.
#[derive(Debug, PartialEq, Eq)]
pub struct StartRequest<'a> {
    pub verb: Option<&'a str>,
    pub args: &'a str,
}

pub fn installer_request(args: &str) -> StartRequest<'_> {
    StartRequest { verb: INSTALLER_VERB, args }
}

/// `ShellExecuteExW` with no verb: starts the installer with the tray's own non-elevated token.
/// Setup raises the permission prompt itself. `ERROR_CANCELLED` is still mapped to `Declined` in
/// case Windows prompts at this call (a Setup whose manifest demands elevation).
#[cfg(windows)]
pub fn start_non_elevated(path: &Path, args: &str) -> Result<Started, RunError> {
    use windows::core::{HRESULT, PCWSTR};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(std::iter::once(0)).collect()
    }
    let request = installer_request(args);
    let verb = request.verb.map(|v| wide(std::ffi::OsStr::new(v)));
    let file = wide(path.as_os_str());
    let params = wide(std::ffi::OsStr::new(request.args));
    let dir = path.parent().map(|d| wide(d.as_os_str()));

    // SAFETY: `info` and the wide strings outlive the call; ShellExecuteExW reads them and
    // writes only `hProcess` and the result. COM is initialised for this thread and released after.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
            lpVerb: verb.as_ref().map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // token: the tray's own, non-elevated (no verb). Setup elevates itself.
        let result = ShellExecuteExW(&mut info);
        if com {
            CoUninitialize();
        }
        match result {
            Ok(()) => Ok((!info.hProcess.is_invalid()).then_some(ProcessHandle(info.hProcess.0 as isize))),
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED) => Err(RunError::Declined),
            Err(e) => Err(RunError::Failed(format!("Windows could not start the installer: {e}"))),
        }
    }
}

#[cfg(not(windows))]
pub fn start_non_elevated(_path: &Path, _args: &str) -> Result<Started, RunError> {
    Err(RunError::Failed("the installer only runs on Windows".into()))
}

/// Blocks until the installer process ends, closes the handle and returns its exit code.
#[cfg(windows)]
pub fn wait_and_close(h: ProcessHandle) -> Option<u32> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    let handle = HANDLE(h.0 as *mut std::ffi::c_void);
    // SAFETY: the handle came from ShellExecuteExW with SEE_MASK_NOCLOSEPROCESS and is closed once, here.
    unsafe {
        WaitForSingleObject(handle, INFINITE);
        let mut code = 0u32;
        let got = GetExitCodeProcess(handle, &mut code).is_ok();
        let _ = CloseHandle(handle);
        got.then_some(code)
    }
}

#[cfg(not(windows))]
pub fn wait_and_close(_h: ProcessHandle) -> Option<u32> {
    None
}
