//! Makes the machine home (`%ProgramData%\canvas-rgb`) a tree that ordinary users cannot write to.
//! `%ProgramData%` lets Users create subfolders and files, so without this a user could plant a DLL
//! where the SYSTEM-run OpenRGB loads plugins, or forge `fans.toml`, `bios-state.json` or `chip.toml`.
//!
//! The decision logic lives in [`secure_home`] over the [`FsAcl`] trait, so tests run it on a fake
//! tree. The Win32 calls are in `WinFs` and are compiled, not run, in tests. They are settled on
//! the real machine with `icacls`.
//!
//! Each start:
//! 1. Refuses a home or child that is a junction or symlink (reparse point).
//! 2. Renames aside a home whose owner is not SYSTEM or Administrators (never read), and creates a fresh one.
//! 3. Applies owner SYSTEM and the protected DACL to the whole tree (`TreeSetNamedSecurityInfoW`, reset).
//!
//! Before any file in the home is read, [`SecureDir::trusted`] checks its owner again.

use std::io;
use std::path::{Path, PathBuf};

/// Owner SYSTEM. DACL: SYSTEM full, Administrators full, Users read and execute (`0x1200a9`),
/// all inheritable, protected (`P`).
pub const SECURE_SDDL: &str = "O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

/// Subfolders the service creates inside the home, so they are secured and checked with it.
pub const SUBDIRS: [&str; 2] = ["openrgb", "logs"];

/// SID strings of the owners that are trusted: LocalSystem and BUILTIN\Administrators.
pub const TRUSTED_OWNER_SIDS: [&str; 2] = ["S-1-5-18", "S-1-5-32-544"];

/// `FILE_ATTRIBUTE_REPARSE_POINT`: a junction or symlink.
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub fn owner_is_trusted_sid(sid: &str) -> bool {
    TRUSTED_OWNER_SIDS.contains(&sid)
}

/// The filesystem operations the decision logic needs. Real: [`WinFs`]. Tests: a fake tree.
pub trait FsAcl: Send {
    fn exists(&self, p: &Path) -> io::Result<bool>;
    fn is_dir(&self, p: &Path) -> io::Result<bool>;
    /// True for junctions and symlinks. Checked without following them.
    fn is_reparse(&self, p: &Path) -> io::Result<bool>;
    /// Direct children of a directory. Never called on a reparse point.
    fn list(&self, p: &Path) -> io::Result<Vec<PathBuf>>;
    fn owner_trusted(&self, p: &Path) -> io::Result<bool>;
    fn create_dir(&mut self, p: &Path) -> io::Result<()>;
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    /// Applies owner SYSTEM and [`SECURE_SDDL`]'s protected DACL to `p` and everything under it.
    fn reset_tree(&mut self, p: &Path) -> io::Result<()>;
}

/// Secures a directory and reports whether a path can be trusted.
pub trait SecureDir: Send {
    /// Creates `dir` if needed and secures it. An `Err` means the home cannot be trusted, and
    /// nothing in it may be read or launched from.
    fn secure(&mut self, dir: &Path) -> io::Result<()>;
    /// True when `path` exists, is not a reparse point, and its owner is SYSTEM or Administrators.
    fn trusted(&self, path: &Path) -> bool;
}

fn refused(path: &Path, why: &str) -> io::Error {
    io::Error::other(format!("{}: {why}", path.display()))
}

/// Where a home whose owner is not trusted is moved: `canvas-rgb.untrusted-<now>` beside it.
pub fn aside_name(dir: &Path, now: u64) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "canvas-rgb".into());
    dir.with_file_name(format!("{name}.untrusted-{now}"))
}

/// Refuses any reparse point in the tree below `dir`.
fn refuse_reparse_tree(fs: &dyn FsAcl, dir: &Path) -> io::Result<()> {
    for child in fs.list(dir)? {
        if fs.is_reparse(&child)? {
            return Err(refused(&child, "is a junction or symlink; refusing"));
        }
        if fs.is_dir(&child)? {
            refuse_reparse_tree(fs, &child)?;
        }
    }
    Ok(())
}

/// The decision logic. See the module docs for the order.
pub fn secure_home(fs: &mut dyn FsAcl, dir: &Path, now: u64) -> io::Result<()> {
    if fs.exists(dir)? {
        if fs.is_reparse(dir)? {
            return Err(refused(dir, "is a junction or symlink; refusing"));
        }
        if !fs.is_dir(dir)? {
            return Err(refused(dir, "exists and is not a directory"));
        }
        if !fs.owner_trusted(dir)? {
            let aside = aside_name(dir, now);
            tracing::error!(
                home = %dir.display(),
                aside = %aside.display(),
                "machine home owner is not SYSTEM or Administrators; renaming it aside (it will never be read) and creating a fresh one"
            );
            fs.rename(dir, &aside)?;
        }
    }
    if !fs.exists(dir)? {
        fs.create_dir(dir)?;
    }
    for sub in SUBDIRS {
        let path = dir.join(sub);
        if !fs.exists(&path)? {
            fs.create_dir(&path)?;
        }
    }
    refuse_reparse_tree(fs, dir)?;
    fs.reset_tree(dir)
}

/// Unix seconds, for the aside name.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// [`SecureDir`] over any [`FsAcl`].
pub struct Secured<F: FsAcl> {
    fs: F,
    clock: Box<dyn Fn() -> u64 + Send>,
}

impl<F: FsAcl> Secured<F> {
    pub fn new(fs: F) -> Self {
        Self { fs, clock: Box::new(unix_now) }
    }

    pub fn with_clock(fs: F, clock: Box<dyn Fn() -> u64 + Send>) -> Self {
        Self { fs, clock }
    }
}

impl<F: FsAcl> SecureDir for Secured<F> {
    fn secure(&mut self, dir: &Path) -> io::Result<()> {
        let now = (self.clock)();
        secure_home(&mut self.fs, dir, now)
    }

    fn trusted(&self, path: &Path) -> bool {
        let not_reparse = matches!(self.fs.exists(path), Ok(true))
            && matches!(self.fs.is_reparse(path), Ok(false));
        not_reparse && self.fs.owner_trusted(path).unwrap_or(false)
    }
}

/// The production [`SecureDir`]: Win32 over the real filesystem.
#[cfg(windows)]
pub fn windows_acl() -> Box<dyn SecureDir> {
    Box::new(Secured::new(WinFs))
}

/// The real filesystem, with the Win32 security calls.
#[cfg(windows)]
pub struct WinFs;

#[cfg(windows)]
fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().chain(Some(0)).collect()
}

/// The owner SID of `path`, as its string form (`S-1-5-18`, ...).
#[cfg(windows)]
fn owner_sid_string(path: &Path) -> io::Result<String> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::OWNER_SECURITY_INFORMATION;

    let target = wide(path.as_os_str());
    let mut owner: *mut c_void = std::ptr::null_mut();
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    // SAFETY: `target` is NUL-terminated; the out pointers receive the owner (inside `descriptor`)
    // and the descriptor, which is freed below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            target.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `owner` points into the descriptor, still allocated; `text` receives a LocalAlloc'd string.
    let converted = unsafe { ConvertSidToStringSidW(owner, &mut text) };
    let result = if converted == 0 {
        Err(io::Error::last_os_error())
    } else {
        let mut len = 0usize;
        // SAFETY: the converted string is NUL-terminated.
        unsafe {
            while *text.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(text, len);
            Ok(String::from_utf16_lossy(slice))
        }
    };
    // SAFETY: both buffers came from the calls above and are freed once.
    unsafe {
        if !text.is_null() {
            LocalFree(text as *mut c_void);
        }
        LocalFree(descriptor);
    }
    result
}

#[cfg(windows)]
impl FsAcl for WinFs {
    fn exists(&self, p: &Path) -> io::Result<bool> {
        match std::fs::symlink_metadata(p) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn is_dir(&self, p: &Path) -> io::Result<bool> {
        Ok(std::fs::symlink_metadata(p)?.is_dir())
    }

    fn is_reparse(&self, p: &Path) -> io::Result<bool> {
        use std::os::windows::fs::MetadataExt;
        // symlink_metadata opens with FILE_FLAG_OPEN_REPARSE_POINT, so a junction is reported as itself.
        Ok(std::fs::symlink_metadata(p)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }

    fn list(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(p)?.map(|entry| entry.map(|e| e.path())).collect()
    }

    fn owner_trusted(&self, p: &Path) -> io::Result<bool> {
        Ok(owner_is_trusted_sid(&owner_sid_string(p)?))
    }

    fn create_dir(&mut self, p: &Path) -> io::Result<()> {
        std::fs::create_dir(p)
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn reset_tree(&mut self, p: &Path) -> io::Result<()> {
        use std::ffi::c_void;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::core::BOOL;
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, TreeSetNamedSecurityInfoW, SE_FILE_OBJECT,
            TREE_SEC_INFO_RESET,
        };
        use windows_sys::Win32::Security::{
            GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, ACL, DACL_SECURITY_INFORMATION,
            OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        };

        let sddl: Vec<u16> = SECURE_SDDL.encode_utf16().chain(Some(0)).collect();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a LocalAlloc'd buffer freed below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut())
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| -> io::Result<()> {
            let mut owner: *mut c_void = std::ptr::null_mut();
            let mut owner_defaulted: BOOL = 0;
            // SAFETY: `descriptor` is valid until LocalFree below.
            if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut present: BOOL = 0;
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut dacl_defaulted: BOOL = 0;
            // SAFETY: as above.
            if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut dacl_defaulted) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let target = wide(p.as_os_str());
            let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
            // SAFETY: `target` is NUL-terminated; `owner` and `dacl` point into `descriptor`, which
            // outlives the call. No progress callback. A failure on any child fails the whole call.
            let status = unsafe {
                TreeSetNamedSecurityInfoW(
                    target.as_ptr(),
                    SE_FILE_OBJECT,
                    info,
                    owner,
                    std::ptr::null_mut(),
                    dacl,
                    std::ptr::null(),
                    TREE_SEC_INFO_RESET,
                    None,
                    windows_sys::Win32::Security::Authorization::ProgressInvokeNever,
                    std::ptr::null(),
                )
            };
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            Ok(())
        })();
        // SAFETY: `descriptor` came from ConvertStringSecurityDescriptor... and is freed once.
        unsafe { LocalFree(descriptor) };
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug)]
    struct Node {
        dir: bool,
        reparse: bool,
        trusted: bool,
        /// Makes `reset_tree` fail for this node.
        stuck: bool,
    }

    /// A tree in memory. Records every call, so tests can see what was read.
    #[derive(Default)]
    struct FakeFs {
        nodes: BTreeMap<PathBuf, Node>,
        /// Paths whose children were listed (i.e. read).
        listed: Arc<Mutex<Vec<PathBuf>>>,
        reset: Vec<PathBuf>,
        rename_fails: bool,
    }

    impl FakeFs {
        fn with(mut self, path: &str, node: Node) -> Self {
            self.nodes.insert(PathBuf::from(path), node);
            self
        }

        fn descendants(&self, p: &Path) -> Vec<PathBuf> {
            self.nodes.keys().filter(|k| k.starts_with(p) && k.as_path() != p).cloned().collect()
        }
    }

    fn dir(trusted: bool) -> Node {
        Node { dir: true, reparse: false, trusted, stuck: false }
    }

    fn file(trusted: bool) -> Node {
        Node { dir: false, reparse: false, trusted, stuck: false }
    }

    impl FsAcl for FakeFs {
        fn exists(&self, p: &Path) -> io::Result<bool> {
            Ok(self.nodes.contains_key(p))
        }
        fn is_dir(&self, p: &Path) -> io::Result<bool> {
            Ok(self.nodes.get(p).is_some_and(|n| n.dir))
        }
        fn is_reparse(&self, p: &Path) -> io::Result<bool> {
            Ok(self.nodes.get(p).is_some_and(|n| n.reparse))
        }
        fn list(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
            self.listed.lock().unwrap().push(p.to_path_buf());
            let mut out: Vec<PathBuf> = self
                .nodes
                .keys()
                .filter(|k| k.parent() == Some(p))
                .cloned()
                .collect();
            out.sort();
            Ok(out)
        }
        fn owner_trusted(&self, p: &Path) -> io::Result<bool> {
            self.nodes.get(p).map(|n| n.trusted).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn create_dir(&mut self, p: &Path) -> io::Result<()> {
            self.nodes.insert(p.to_path_buf(), dir(true));
            Ok(())
        }
        fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            if self.rename_fails {
                return Err(io::Error::other("rename refused"));
            }
            let moved: Vec<(PathBuf, Node)> = std::iter::once(from.to_path_buf())
                .chain(self.descendants(from))
                .filter_map(|p| self.nodes.remove(&p).map(|n| (p, n)))
                .collect();
            for (p, n) in moved {
                let rel = p.strip_prefix(from).unwrap().to_path_buf();
                self.nodes.insert(to.join(rel), n);
            }
            Ok(())
        }
        fn reset_tree(&mut self, p: &Path) -> io::Result<()> {
            self.reset.push(p.to_path_buf());
            let under: BTreeSet<PathBuf> = std::iter::once(p.to_path_buf()).chain(self.descendants(p)).collect();
            for path in &under {
                if self.nodes.get(path).is_some_and(|n| n.stuck) {
                    return Err(io::Error::other(format!("cannot reset {}", path.display())));
                }
            }
            for path in under {
                if let Some(n) = self.nodes.get_mut(&path) {
                    n.trusted = true;
                }
            }
            Ok(())
        }
    }

    const HOME: &str = r"C:\ProgramData\canvas-rgb";

    #[test]
    fn a_fresh_home_is_created_with_its_subfolders_and_reset() {
        let mut fs = FakeFs::default();
        secure_home(&mut fs, Path::new(HOME), 7).unwrap();
        assert!(fs.nodes.contains_key(Path::new(HOME)));
        assert!(fs.nodes.contains_key(&PathBuf::from(HOME).join("openrgb")));
        assert!(fs.nodes.contains_key(&PathBuf::from(HOME).join("logs")));
        assert_eq!(fs.reset, vec![PathBuf::from(HOME)]);
    }

    #[test]
    fn an_untrusted_owner_is_renamed_aside_and_never_read() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(false))
            .with(&format!(r"{HOME}\fans.toml"), file(false))
            .with(&format!(r"{HOME}\openrgb"), dir(false));
        secure_home(&mut fs, Path::new(HOME), 1234).unwrap();

        let aside = PathBuf::from(r"C:\ProgramData\canvas-rgb.untrusted-1234");
        assert!(fs.nodes.contains_key(&aside), "old home renamed aside");
        assert!(fs.nodes.contains_key(&aside.join("fans.toml")), "old contents moved, not read");
        // The fresh home is created, and its children are the new ones, all trusted after reset.
        assert!(fs.nodes.get(Path::new(HOME)).unwrap().trusted);
        assert!(fs.nodes.get(&PathBuf::from(HOME).join("openrgb")).unwrap().trusted);
        assert!(!fs.nodes.contains_key(&PathBuf::from(HOME).join("fans.toml")), "no forged file survives");
        // Nothing under the renamed-aside folder was listed.
        let listed = fs.listed.lock().unwrap();
        assert!(!listed.iter().any(|p| p.starts_with(&aside)), "the aside folder was never read");
    }

    #[test]
    fn a_reparse_point_home_is_refused_and_nothing_is_moved() {
        let mut fs = FakeFs::default().with(HOME, Node { dir: true, reparse: true, trusted: true, stuck: false });
        let err = secure_home(&mut fs, Path::new(HOME), 1).unwrap_err();
        assert!(err.to_string().contains("junction or symlink"), "{err}");
        assert!(fs.nodes.contains_key(Path::new(HOME)), "not renamed");
        assert!(fs.reset.is_empty(), "no reset happened");
    }

    #[test]
    fn a_reparse_point_child_is_refused_before_the_reset() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&format!(r"{HOME}\openrgb"), Node { dir: true, reparse: true, trusted: false, stuck: false });
        let err = secure_home(&mut fs, Path::new(HOME), 1).unwrap_err();
        assert!(err.to_string().contains("openrgb"), "{err}");
        assert!(fs.reset.is_empty(), "the reset never ran");
    }

    #[test]
    fn an_untrusted_child_is_reset_to_trusted() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&format!(r"{HOME}\openrgb"), dir(true))
            .with(&format!(r"{HOME}\openrgb\plugins"), dir(false))
            .with(&format!(r"{HOME}\openrgb\plugins\evil.dll"), file(false));
        secure_home(&mut fs, Path::new(HOME), 1).unwrap();
        assert!(fs.nodes.get(&PathBuf::from(HOME).join("openrgb").join("plugins")).unwrap().trusted);
        assert!(fs.nodes.get(&PathBuf::from(HOME).join("openrgb").join("plugins").join("evil.dll")).unwrap().trusted);
    }

    #[test]
    fn a_child_that_cannot_be_reset_fails_the_whole_step() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&format!(r"{HOME}\logs"), Node { dir: true, reparse: false, trusted: false, stuck: true });
        assert!(secure_home(&mut fs, Path::new(HOME), 1).is_err());
    }

    #[test]
    fn a_failed_rename_fails_without_creating_a_fresh_home() {
        let mut fs = FakeFs { rename_fails: true, ..FakeFs::default() }.with(HOME, dir(false));
        assert!(secure_home(&mut fs, Path::new(HOME), 1).is_err());
        assert!(fs.reset.is_empty());
    }

    #[test]
    fn the_owner_check_accepts_only_system_and_administrators() {
        assert!(owner_is_trusted_sid("S-1-5-18"));
        assert!(owner_is_trusted_sid("S-1-5-32-544"));
        assert!(!owner_is_trusted_sid("S-1-5-32-545"), "Users");
        assert!(!owner_is_trusted_sid("S-1-1-0"), "Everyone");
    }

    #[test]
    fn trusted_refuses_missing_reparse_and_untrusted_paths() {
        let fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&format!(r"{HOME}\fans.toml"), file(false))
            .with(&format!(r"{HOME}\link"), Node { dir: false, reparse: true, trusted: true, stuck: false });
        let secured = Secured::with_clock(fs, Box::new(|| 0));
        assert!(secured.trusted(Path::new(HOME)));
        assert!(!secured.trusted(&PathBuf::from(HOME).join("fans.toml")), "untrusted owner");
        assert!(!secured.trusted(&PathBuf::from(HOME).join("link")), "reparse point");
        assert!(!secured.trusted(&PathBuf::from(HOME).join("missing.toml")), "missing");
    }

    #[test]
    fn the_sddl_protects_the_dacl_and_gives_users_read_and_execute_only() {
        assert!(SECURE_SDDL.starts_with("O:SY"));
        assert!(SECURE_SDDL.contains("D:P("), "protected DACL");
        assert!(SECURE_SDDL.contains("(A;OICI;FA;;;SY)"));
        assert!(SECURE_SDDL.contains("(A;OICI;FA;;;BA)"));
        assert!(SECURE_SDDL.contains("(A;OICI;0x1200a9;;;BU)"));
        assert!(!SECURE_SDDL.contains(";;;WD)"), "no Everyone grant");
    }
}
