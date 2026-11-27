//! The read-only, fail-closed reader for the service's log folder (R5, P3).
//!
//! Nothing under `%ProgramData%\canvas-rgb` is ever written, deleted, renamed or adopted. Every
//! component of the path is checked for a reparse point and for a SYSTEM/Administrators owner
//! before a file is opened. Each file is opened once with `FILE_FLAG_OPEN_REPARSE_POINT`, checked
//! on that same handle and read from it — a swapped path between check and read cannot redirect
//! the read. All decisions are behind [`LogFs`], so the refusals are tested on a fake tree.

use std::path::{Path, PathBuf};

/// SID strings of the only owners we trust, as in the service's secure home.
pub const TRUSTED_OWNER_SIDS: [&str; 2] = ["S-1-5-18", "S-1-5-32-544"];

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
        if !trusted_parent(&owner) {
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
        if !owner_is_trusted(&owner) {
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

/// The real filesystem. Compiled for Windows; other targets have no machine tree and refuse.
#[cfg(windows)]
pub struct RealLogFs;

#[cfg(windows)]
impl LogFs for RealLogFs {
    fn exists(&self, path: &Path) -> Result<bool, String> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

    fn is_reparse(&self, path: &Path) -> Result<bool, String> {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        let attrs = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?.file_attributes();
        Ok(attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }

    fn owner(&self, path: &Path) -> Result<String, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows::Win32::Security::{OWNER_SECURITY_INFORMATION, PSID, PSECURITY_DESCRIPTOR};

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut sid: PSID = PSID::default();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the path is NUL-terminated and the out-pointers are valid locals.
        let status = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut sid),
                None,
                None,
                None,
                &mut descriptor,
            )
        };
        if status.0 != 0 {
            return Err(format!("GetNamedSecurityInfoW returned {}", status.0));
        }
        let mut text = windows::core::PWSTR::null();
        let result = unsafe { ConvertSidToStringSidW(sid, &mut text) }
            .map_err(|e| format!("ConvertSidToStringSidW failed: {e}"))
            .and_then(|_| unsafe { text.to_string() }.map_err(|e| format!("the owner SID could not be read: {e}")));
        // SAFETY: both buffers came from the calls above and are freed once.
        unsafe {
            if !text.is_null() {
                LocalFree(Some(HLOCAL(text.0.cast())));
            }
            LocalFree(Some(HLOCAL(descriptor.0)));
        }
        result
    }

    fn files(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .filter_map(|entry| match entry {
                Ok(e) if e.path().is_file() => Some(Ok(e.path())),
                Ok(_) => None,
                Err(e) => Some(Err(e.to_string())),
            })
            .collect()
    }

    fn open_checked(&self, path: &Path) -> Result<Vec<u8>, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
        use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT};
        use windows::Win32::Security::{OWNER_SECURITY_INFORMATION, PSID};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, ReadFile, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows::core::PCWSTR;

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: the path is NUL-terminated; no reparse point is followed on this open.
        let handle: HANDLE = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_READ_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                Some(HANDLE::default()),
            )
        }
        .map_err(|e| format!("could not open the log: {e}"))?;
        if handle == INVALID_HANDLE_VALUE {
            return Err("could not open the log".to_string());
        }
        let result = (|| {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // SAFETY: `handle` is open for the whole closure.
            unsafe { GetFileInformationByHandle(handle, &mut info) }
                .map_err(|e| format!("could not read the log's attributes: {e}"))?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                return Err("the log is a junction or symlink".to_string());
            }
            let mut sid: PSID = PSID::default();
            // SAFETY: the handle is valid and `sid` receives a pointer into the security
            // descriptor; both are freed below.
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
            let owner = unsafe { ConvertSidToStringSidW(sid, &mut text) }
                .map_err(|e| format!("ConvertSidToStringSidW failed: {e}"))
                .and_then(|_| unsafe { text.to_string() }.map_err(|e| e.to_string()))?;
            // SAFETY: the string came from ConvertSidToStringSidW and is freed once.
            unsafe {
                if !text.is_null() {
                    windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(text.0.cast())));
                }
            }
            if !owner_is_trusted(&owner) {
                return Err(format!("the log is owned by {owner}, not SYSTEM or Administrators"));
            }
            // A text log is small; read from the same handle that was checked.
            let mut buf = vec![0u8; 1024 * 1024];
            let mut read = 0u32;
            // SAFETY: `buf` is valid for `buf.len()` bytes and `read` is a valid out-parameter.
            unsafe { ReadFile(handle, Some(&mut buf[..]), Some(&mut read), None) }
                .map_err(|e| format!("could not read the log: {e}"))?;
            buf.truncate(read as usize);
            Ok(buf)
        })();
        // SAFETY: `handle` was opened above and is closed exactly once here.
        unsafe {
            let _ = CloseHandle(handle);
        }
        result
    }
}

#[cfg(not(windows))]
pub struct RealLogFs;

#[cfg(not(windows))]
impl LogFs for RealLogFs {
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
            Ok(self
                .owners
                .get(path)
                .cloned()
                .unwrap_or_else(|| "S-1-5-18".to_string()))
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
        assert!(collect(&fs, Path::new("machine/logs")).is_err());
        assert_eq!(fs.reads.get(), 0);
    }

    #[test]
    fn a_reparse_point_is_refused_without_a_read() {
        let mut fs = fake();
        fs.reparse.insert(PathBuf::from("machine/logs"));
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
    fn a_foreign_owned_file_is_refused_without_a_read() {
        let mut fs = fake();
        fs.owners.insert(PathBuf::from("machine/logs/svc.log"), "S-1-5-21-999".to_string());
        let reason = refusal(&fs, Path::new("machine/logs")).unwrap();
        assert!(reason.contains("A service log is owned by"), "{reason}");
        assert_eq!(fs.reads.get(), 0);
    }

    #[test]
    fn an_unreadable_file_is_a_plain_reason() {
        let mut fs = fake();
        fs.unreadable.insert(PathBuf::from("machine/logs/svc.log"));
        let reason = collect(&fs, Path::new("machine/logs")).unwrap_err();
        assert!(reason.contains("Windows did not let canvas-rgb read them"), "{reason}");
    }

    #[test]
    fn an_empty_folder_is_not_reported_as_included() {
        let mut fs = fake();
        fs.logs.clear();
        let reason = collect(&fs, Path::new("machine/logs")).unwrap_err();
        assert!(reason.contains("folder is empty"), "{reason}");
        assert_eq!(fs.reads.get(), 0);
    }
}
