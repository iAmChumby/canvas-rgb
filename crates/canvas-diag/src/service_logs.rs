//! The read-only, fail-closed reader for the service's log folder (R5).
//!
//! `%ProgramData%\canvas-rgb` belongs to the service. We never write, delete, rename or adopt
//! anything there. Before a single byte is read we refuse a junction, symlink or reparse point
//! anywhere on the path and check that the folder is owned by SYSTEM or Administrators — the
//! same rule the service applies to its home on every start. On any refusal the bundle says why
//! in `service_logs` and carries on without the logs. The decision is behind [`LogFs`] so the
//! refusals are tested on a fake tree with no real machine folder.

use std::path::{Path, PathBuf};

/// SID strings of the only owners we trust, as in the service's secure home.
pub const TRUSTED_OWNER_SIDS: [&str; 2] = ["S-1-5-18", "S-1-5-32-544"];

pub fn owner_is_trusted(sid: &str) -> bool {
    TRUSTED_OWNER_SIDS.contains(&sid)
}

/// The filesystem facts the fail-closed rule needs. A fake supplies them in tests.
pub trait LogFs {
    fn exists(&self, path: &Path) -> Result<bool, String>;
    /// True for a junction or symlink, checked without following it.
    fn is_reparse(&self, path: &Path) -> Result<bool, String>;
    /// The owner's SID string (`S-1-5-18`, ...).
    fn owner(&self, path: &Path) -> Result<String, String>;
    fn files(&self, dir: &Path) -> Result<Vec<PathBuf>, String>;
    fn read(&self, path: &Path) -> Result<Vec<u8>, String>;
}

/// `None` when the folder may be read; `Some(reason)` when it must be refused. The reason is
/// written into `DiagBundle.service_logs` verbatim, so it is plain words a person can act on.
pub fn refusal(fs: &dyn LogFs, dir: &Path) -> Option<String> {
    let exists = match fs.exists(dir) {
        Ok(v) => v,
        Err(e) => return Some(format!("not included: the service log folder could not be checked ({e})")),
    };
    if !exists {
        return Some("not included: the service log folder is not there".to_string());
    }
    match fs.is_reparse(dir) {
        Ok(true) => {
            return Some("not included: the service log folder is a junction or symlink".to_string())
        }
        Ok(false) => {}
        Err(e) => return Some(format!("not included: the service log folder could not be checked ({e})")),
    }
    let owner = match fs.owner(dir) {
        Ok(sid) => sid,
        Err(e) => {
            return Some(format!("not included: the owner of the service log folder could not be read ({e})"))
        }
    };
    if !owner_is_trusted(&owner) {
        return Some(format!(
            "not included: the service log folder is owned by {owner}, not SYSTEM or Administrators"
        ));
    }
    let files = match fs.files(dir) {
        Ok(v) => v,
        Err(e) => return Some(format!("not included: the service log folder could not be listed ({e})")),
    };
    for path in files {
        match fs.is_reparse(&path) {
            Ok(true) => {
                return Some("not included: a file in the service log folder is a junction or symlink".to_string())
            }
            Ok(false) => {}
            Err(e) => return Some(format!("not included: a service log could not be checked ({e})")),
        }
    }
    None
}

/// Reads the service logs after [`refusal`] passes. Never writes to the machine tree.
pub fn collect(fs: &dyn LogFs, dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    if let Some(reason) = refusal(fs, dir) {
        return Err(reason);
    }
    let mut out = Vec::new();
    for path in fs.files(dir)? {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "not included: a service log has no readable name".to_string())?;
        let bytes = fs
            .read(&path)
            .map_err(|e| format!("not included: a service log could not be read ({e})"))?;
        out.push((format!("service-logs/{name}"), bytes));
    }
    Ok(out)
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
        let attrs = std::fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_attributes();
        Ok(attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }

    fn owner(&self, path: &Path) -> Result<String, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::{
            ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
        };
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

    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| e.to_string())
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
    fn read(&self, _path: &Path) -> Result<Vec<u8>, String> {
        Err("the machine folder is a Windows-only location".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        reparse: std::collections::HashSet<PathBuf>,
        owner: String,
        logs: Vec<(PathBuf, Vec<u8>)>,
        reads: std::cell::Cell<usize>,
        folder: bool,
    }

    impl LogFs for Fake {
        fn exists(&self, _path: &Path) -> Result<bool, String> {
            Ok(self.folder)
        }
        fn is_reparse(&self, path: &Path) -> Result<bool, String> {
            Ok(self.reparse.contains(path))
        }
        fn owner(&self, _path: &Path) -> Result<String, String> {
            if self.owner.is_empty() {
                Err("the owner could not be read".to_string())
            } else {
                Ok(self.owner.clone())
            }
        }
        fn files(&self, _dir: &Path) -> Result<Vec<PathBuf>, String> {
            Ok(self.logs.iter().map(|(p, _)| p.clone()).collect())
        }
        fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
            self.reads.set(self.reads.get() + 1);
            self.logs
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| "no such log".to_string())
        }
    }

    fn fake() -> Fake {
        Fake {
            folder: true,
            owner: "S-1-5-18".to_string(),
            logs: vec![(PathBuf::from("logs/svc.log"), b"service line".to_vec())],
            ..Default::default()
        }
    }

    #[test]
    fn a_system_owned_plain_folder_is_read() {
        let fs = fake();
        let dir = Path::new("machine/logs");
        let logs = collect(&fs, dir).unwrap();
        assert_eq!(logs, vec![("service-logs/svc.log".to_string(), b"service line".to_vec())]);
        assert_eq!(fs.reads.get(), 1);
    }

    #[test]
    fn a_reparse_point_is_refused_without_a_read() {
        let mut fs = fake();
        fs.reparse.insert(PathBuf::from("machine/logs"));
        let dir = Path::new("machine/logs");
        let reason = refusal(&fs, dir).unwrap();
        assert!(reason.contains("junction or symlink"), "{reason}");
        assert!(collect(&fs, dir).is_err());
        assert_eq!(fs.reads.get(), 0, "nothing was read through the junction");
    }

    #[test]
    fn a_foreign_owner_is_refused_without_a_read() {
        let mut fs = fake();
        fs.owner = "S-1-5-21-999".to_string();
        let dir = Path::new("machine/logs");
        let reason = refusal(&fs, dir).unwrap();
        assert!(reason.contains("not SYSTEM or Administrators"), "{reason}");
        assert!(collect(&fs, dir).is_err());
        assert_eq!(fs.reads.get(), 0);
    }

    #[test]
    fn a_reparse_file_inside_is_refused_without_a_read() {
        let mut fs = fake();
        fs.reparse.insert(PathBuf::from("logs/svc.log"));
        let reason = refusal(&fs, Path::new("machine/logs")).unwrap();
        assert!(reason.contains("file in the service log folder"), "{reason}");
        assert_eq!(fs.reads.get(), 0);
    }
}
