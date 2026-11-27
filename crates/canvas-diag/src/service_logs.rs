//! The read-only, fail-closed reader for the service's log folder (R5, P3, F1).
//!
//! Nothing under `%ProgramData%\canvas-rgb` is ever written, deleted, renamed or adopted. Every
//! component of the path is checked for a reparse point and for a SYSTEM/Administrators owner
//! before a file is opened. Each file is opened once with `FILE_FLAG_OPEN_REPARSE_POINT`, checked
//! on that same handle and read from it — a swapped path between check and read cannot redirect
//! the read. All decisions are behind [`LogFs`], so the refusals are tested on a fake tree.

use std::path::{Path, PathBuf};

/// SID strings of the only owners we trust, as in the service's secure home.
pub const TRUSTED_OWNER_SIDS: [&str; 2] = ["S-1-5-18", "S-1-5-32-544"];

/// A service log over this size is refused, not silently cut (F1).
pub const MAX_LOG_BYTES: usize = 4 * 1024 * 1024;

/// Windows itself owns the folders above the program tree; they are trusted parents, but never
/// an entry inside the canvas-rgb tree.
pub const TRUSTED_INSTALLER_SID: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

pub fn owner_is_trusted(sid: &str) -> bool {
    TRUSTED_OWNER_SIDS.contains(&sid)
}

fn trusted_parent(sid: &str) -> bool {
    owner_is_trusted(sid) || sid == TRUSTED_INSTALLER_SID
}

/// The filesystem facts the fail-closed rule needs. A fake supplies them in tests.
pub trait LogFs {
    /// The owner rule this filesystem applies; tests on a user-owned folder can inject one.
    fn trusts(&self, sid: &str) -> bool {
        owner_is_trusted(sid)
    }
    fn exists(&self, path: &Path) -> Result<bool, String>;
    /// True for a junction or symlink, checked without following it.
    fn is_reparse(&self, path: &Path) -> Result<bool, String>;
    /// The owner's SID string (`S-1-5-18`, ...).
    fn owner(&self, path: &Path) -> Result<String, String>;
    /// The files in a directory, checked without following their links.
    fn files(&self, dir: &Path) -> Result<Vec<PathBuf>, String>;
    /// Opens once with `FILE_FLAG_OPEN_REPARSE_POINT`, checks the handle's attributes and owner,
    /// then reads from that handle. Never re-opens by path.
    fn open_checked(&self, path: &Path) -> Result<Vec<u8>, String>;
}

/// One service log, named for its place inside the zip.
pub type Log = (String, Vec<u8>);

/// `None` when the folder may be read; `Some(reason)` when it must be refused. The reason is
/// written into `DiagBundle.service_logs` verbatim, so it is plain words a person can act on.
/// This does not read the files: it is the check the tray uses before opening Explorer.
pub fn refusal(fs: &dyn LogFs, dir: &Path) -> Option<String> {
    check(fs, dir).err()
}

/// Reads the logs after the whole path passes. An empty folder is a refusal too: the UI must
/// never say "included" unless at least one log really went into the zip.
pub fn collect(fs: &dyn LogFs, dir: &Path) -> Result<Vec<Log>, String> {
    check(fs, dir)?;
    let mut out = Vec::new();
    for path in fs.files(dir)? {
        let bytes = fs
            .open_checked(&path)
            .map_err(|e| format!("Service logs: not included. Windows did not let canvas-rgb read them. ({e})"))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "Service logs: not included. A service log has no readable name.".to_string())?;
        out.push((format!("service-logs/{name}"), bytes));
    }
    if out.is_empty() {
        return Err("Service logs: not included. The service log folder is empty.".to_string());
    }
    Ok(out)
}

/// Checks every component of `dir`, the folder itself and every file. `collect` then opens each
/// file once with `open_checked`, which re-checks the handle before reading from it.
fn check(fs: &dyn LogFs, dir: &Path) -> Result<(), String> {
    let exists = fs
        .exists(dir)
        .map_err(|e| format!("Service logs: not included. Windows did not let canvas-rgb check the folder. ({e})"))?;
    if !exists {
        return Err("Service logs: not included. The service log folder is not there.".to_string());
    }
    for component in dir.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        refuse_reparse(fs, component, "the folder")?;
        let owner = fs.owner(component).map_err(|e| {
            format!("Service logs: not included. Windows did not let canvas-rgb read the owner. ({e})")
        })?;
        if !trusted_parent(&owner) && !fs.trusts(&owner) {
            return Err(format!(
                "Service logs: not included. The folder is owned by {owner}, not SYSTEM or Administrators."
            ));
        }
    }
    let files = fs
        .files(dir)
        .map_err(|e| format!("Service logs: not included. Windows did not let canvas-rgb list the folder. ({e})"))?;
    if files.is_empty() {
        return Err("Service logs: not included. The service log folder is empty.".to_string());
    }
    for path in files {
        refuse_reparse(fs, &path, "a file")?;
        let owner = fs
            .owner(&path)
            .map_err(|e| format!("Service logs: not included. Windows did not let canvas-rgb read an owner. ({e})"))?;
        if !fs.trusts(&owner) {
            return Err(format!(
                "Service logs: not included. A service log is owned by {owner}, not SYSTEM or Administrators."
            ));
        }
    }
    Ok(())
}

fn refuse_reparse(fs: &dyn LogFs, path: &Path, what: &str) -> Result<(), String> {
    match fs.is_reparse(path) {
        Ok(true) => Err(format!("Service logs: not included. {what} is a junction or symlink.")),
        Ok(false) => Ok(()),
        Err(e) => Err(format!("Service logs: not included. Windows did not let canvas-rgb check {what}. ({e})")),
    }
}

/// The real filesystem. The owner predicate is a field so a test can inject "trust this temp
/// folder"; production uses the SYSTEM/Administrators rule.
pub struct RealLogFs {
    owner_check: fn(&str) -> bool,
}

impl Default for RealLogFs {
    fn default() -> Self {
        Self { owner_check: owner_is_trusted }
    }
}

impl RealLogFs {
    /// Only for tests on a folder this process owns; never used in production.
    pub fn with_owner_check(check: fn(&str) -> bool) -> Self {
        Self { owner_check: check }
    }
}

#[cfg(windows)]
mod real {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{OWNER_SECURITY_INFORMATION, PSID};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, ReadFile, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_READ, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        READ_CONTROL,
    };

    /// Opens without following a reparse point. `directory` adds `FILE_FLAG_BACKUP_SEMANTICS`.
    fn open(path: &Path, access: u32, directory: bool) -> Result<HANDLE, String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if directory {
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        // SAFETY: the path is NUL-terminated; no reparse point is followed on this open.
        let handle: HANDLE = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                flags,
                Some(HANDLE::default()),
            )
        }
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("could not open {}", path.display()));
        }
        Ok(handle)
    }

    /// Reads the owner SID from an already-open handle: `GetSecurityInfo` needs `READ_CONTROL`.
    fn owner_from_handle(handle: HANDLE) -> Result<String, String> {
        let mut sid: PSID = PSID::default();
        // SAFETY: the handle is open and `sid` receives a pointer into the security descriptor.
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut sid),
                None,
                None,
                None,
                None,
            )
        };
        if status.0 != 0 {
            return Err(format!("GetSecurityInfo returned {}", status.0));
        }
        let mut text = windows::core::PWSTR::null();
        let result = unsafe { ConvertSidToStringSidW(sid, &mut text) }
            .map_err(|e| format!("ConvertSidToStringSidW failed: {e}"))
            .and_then(|_| unsafe { text.to_string() }.map_err(|e| e.to_string()));
        // SAFETY: the string came from ConvertSidToStringSidW and is freed once.
        unsafe {
            if !text.is_null() {
                LocalFree(Some(HLOCAL(text.0.cast())));
            }
        }
        result
    }

    impl LogFs for RealLogFs {
        fn trusts(&self, sid: &str) -> bool {
            (self.owner_check)(sid)
        }
        fn exists(&self, path: &Path) -> Result<bool, String> {
            match std::fs::symlink_metadata(path) {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e.to_string()),
            }
        }

        fn is_reparse(&self, path: &Path) -> Result<bool, String> {
            let handle = open(path, FILE_READ_ATTRIBUTES.0, true)?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // SAFETY: `handle` is open for the whole call.
            let result = unsafe { GetFileInformationByHandle(handle, &mut info) }
                .map_err(|e| e.to_string())
                .map(|()| info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0);
            // SAFETY: `handle` was opened above and is closed once here.
            unsafe {
                let _ = CloseHandle(handle);
            }
            result
        }

        fn owner(&self, path: &Path) -> Result<String, String> {
            // Directories need BACKUP_SEMANTICS; files open the same way without it.
            let is_dir = std::fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false);
            let handle = open(path, FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0, is_dir)?;
            let result = owner_from_handle(handle);
            // SAFETY: `handle` was opened above and is closed once here.
            unsafe {
                let _ = CloseHandle(handle);
            }
            result
        }

        fn files(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
            std::fs::read_dir(dir)
                .map_err(|e| e.to_string())?
                // A junction is not a file: include every entry so `check` can refuse it (F1).
                .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
                .collect()
        }

        fn open_checked(&self, path: &Path) -> Result<Vec<u8>, String> {
            // FILE_GENERIC_READ = FILE_READ_DATA | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE:
            // both GetSecurityInfo and ReadFile need rights the old attributes-only open lacked.
            let handle = open(path, FILE_GENERIC_READ.0, false)?;
            let result = (|| {
                let mut info = BY_HANDLE_FILE_INFORMATION::default();
                // SAFETY: `handle` is open for the whole closure.
                unsafe { GetFileInformationByHandle(handle, &mut info) }
                    .map_err(|e| format!("could not read the log's attributes: {e}"))?;
                if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                    return Err("the log is a junction or symlink".to_string());
                }
                let owner = owner_from_handle(handle)?;
                if !(self.owner_check)(&owner) {
                    return Err(format!("the log is owned by {owner}, not SYSTEM or Administrators"));
                }
                // Read in a loop until EOF, with a cap: an oversized log is refused, never cut.
                let mut out = Vec::new();
                loop {
                    let mut chunk = vec![0u8; 64 * 1024];
                    let mut read = 0u32;
                    // SAFETY: `chunk` is valid for its length and `read` is a valid out-parameter.
                    unsafe { ReadFile(handle, Some(&mut chunk[..]), Some(&mut read), None) }
                        .map_err(|e| format!("could not read the log: {e}"))?;
                    if read == 0 {
                        break;
                    }
                    if out.len() + read as usize > MAX_LOG_BYTES {
                        return Err("larger than 4 MiB".to_string());
                    }
                    out.extend_from_slice(&chunk[..read as usize]);
                }
                Ok(out)
            })();
            // SAFETY: `handle` was opened above and is closed exactly once here.
            unsafe {
                let _ = CloseHandle(handle);
            }
            result
        }
    }
}

#[cfg(not(windows))]
mod real {
    use super::*;

    impl LogFs for RealLogFs {
        fn trusts(&self, sid: &str) -> bool {
            (self.owner_check)(sid)
        }
        fn exists(&self, _path: &Path) -> Result<bool, String> {
            Ok(false)
        }
        fn is_reparse(&self, _path: &Path) -> Result<bool, String> {
            Ok(false)
        }
        fn owner(&self, _path: &Path) -> Result<String, String> {
            Err("the machine folder is a Windows-only location".to_string())
        }
        fn files(&self, _dir: &Path) -> Result<Vec<PathBuf>, String> {
            Ok(Vec::new())
        }
        fn open_checked(&self, _path: &Path) -> Result<Vec<u8>, String> {
            Err("the machine folder is a Windows-only location".to_string())
        }
    }
}

#[cfg(windows)]
#[cfg(test)]
mod windows_tests {
    use super::*;
    use std::os::windows::fs::symlink_dir;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-logs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_real_file_is_read_whole_with_an_injected_owner_check() {
        let dir = temp_dir("read");
        let file = dir.join("svc.log");
        let bytes = vec![b'x'; 100_000];
        std::fs::write(&file, &bytes).unwrap();
        // The injected check covers the read and reparse behaviour; it does not cover the real
        // SYSTEM/Administrators owner rule (a temp folder belongs to the test user).
        let fs = RealLogFs::with_owner_check(|_| true);
        assert_eq!(fs.open_checked(&file).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_real_file_over_the_cap_is_refused() {
        let dir = temp_dir("cap");
        let file = dir.join("big.log");
        std::fs::write(&file, vec![b'x'; MAX_LOG_BYTES + 1]).unwrap();
        let fs = RealLogFs::with_owner_check(|_| true);
        let error = fs.open_checked(&file).unwrap_err();
        assert!(error.contains("larger than 4 MiB"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_real_junction_inside_the_folder_is_refused() {
        let dir = temp_dir("junction");
        let elsewhere = temp_dir("junction-target");
        symlink_dir(&elsewhere, dir.join("link")).unwrap();
        let fs = RealLogFs::with_owner_check(|_| true);
        let error = refusal(&fs, &dir).unwrap();
        assert!(error.contains("junction or symlink"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct Fake {
        reparse: HashSet<PathBuf>,
        owners: HashMap<PathBuf, String>,
        unreadable: HashSet<PathBuf>,
        reads: Cell<usize>,
        folder: bool,
        logs: Vec<PathBuf>,
    }

    impl LogFs for Fake {
        fn exists(&self, _path: &Path) -> Result<bool, String> {
            Ok(self.folder)
        }
        fn is_reparse(&self, path: &Path) -> Result<bool, String> {
            Ok(self.reparse.contains(path))
        }
        fn owner(&self, path: &Path) -> Result<String, String> {
            Ok(self.owners.get(path).cloned().unwrap_or_else(|| "S-1-5-18".to_string()))
        }
        fn files(&self, _dir: &Path) -> Result<Vec<PathBuf>, String> {
            Ok(self.logs.clone())
        }
        fn open_checked(&self, path: &Path) -> Result<Vec<u8>, String> {
            self.reads.set(self.reads.get() + 1);
            if self.unreadable.contains(path) {
                Err("access denied".to_string())
            } else {
                Ok(b"service line".to_vec())
            }
        }
    }

    fn fake() -> Fake {
        let dir = Path::new("machine/logs");
        let file = PathBuf::from("machine/logs/svc.log");
        Fake {
            folder: true,
            owners: HashMap::from([(dir.to_path_buf(), "S-1-5-18".to_string())]),
            logs: vec![file],
            ..Default::default()
        }
    }

    #[test]
    fn a_system_owned_plain_folder_is_read() {
        let fs = fake();
        let logs = collect(&fs, Path::new("machine/logs")).unwrap();
        assert_eq!(logs, vec![("service-logs/svc.log".to_string(), b"service line".to_vec())]);
        assert_eq!(fs.reads.get(), 1);
    }

    #[test]
    fn a_junction_on_the_parent_is_refused_without_a_read() {
        let mut fs = fake();
        fs.reparse.insert(PathBuf::from("machine"));
        let reason = refusal(&fs, Path::new("machine/logs")).unwrap();
        assert!(reason.contains("junction or symlink"), "{reason}");
        assert_eq!(fs.reads.get(), 0);
    }

    #[test]
    fn a_foreign_owner_is_refused_without_a_read() {
        let mut fs = fake();
        fs.owners.insert(PathBuf::from("machine/logs"), "S-1-5-21-999".to_string());
        let reason = refusal(&fs, Path::new("machine/logs")).unwrap();
        assert!(reason.contains("not SYSTEM or Administrators"), "{reason}");
        assert_eq!(fs.reads.get(), 0);
    }

    #[test]
    fn an_empty_folder_is_not_reported_as_included() {
        let mut fs = fake();
        fs.logs.clear();
        let reason = collect(&fs, Path::new("machine/logs")).unwrap_err();
        assert!(reason.contains("folder is empty"), "{reason}");
    }
}
