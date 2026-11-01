//! Makes the machine home (`%ProgramData%\canvas-rgb`) a tree that ordinary users cannot write to.
//! `%ProgramData%` lets Users create subfolders and files, so without this a user could plant a DLL
//! where the SYSTEM-run OpenRGB loads plugins, or forge `fans.toml`, `bios-state.json` or `chip.toml`.
//!
//! The decision logic lives in [`secure_home`] over the [`FsAcl`] trait, so tests run it on a fake
//! tree. The Win32 calls are in `WinFs` and are compiled, not run, in tests. They are settled on
//! the real machine with `icacls`.
//!
//! This step never deletes, renames or takes ownership of anything. Deleting by path inside a
//! tree that a user may write to is a junction race: a swapped-in junction would redirect the
//! delete anywhere on disk. So every existing entry is checked, and any problem fails closed.
//!
//! Each start, in this order:
//! 1. A home that exists must be a real directory (not a junction or symlink) owned by SYSTEM or
//!    Administrators. Otherwise the step fails closed. A home that does not exist is created with
//!    `CreateDirectoryW` and [`SECURE_SDDL`], so it is born protected and has no children.
//! 2. The home's file ID is recorded.
//! 3. A read-only walk checks every entry under the home. The first junction, symlink, or entry
//!    whose owner is not SYSTEM or Administrators fails the step. The error is logged at ERROR with
//!    the offending path, and the message tells the user to check it and remove it by hand. Nothing is
//!    changed by this walk.
//! 4. The protected DACL is applied to the home itself. Children keep their own ACLs, which are
//!    trusted, so nothing else is changed.
//! 5. The `openrgb` and `logs` subfolders are created with [`SECURE_SDDL`] if they are missing.
//! 6. The home is opened again. If its file ID differs from step 2, the step fails closed.
//!
//! Any `Err` means the service treats the home as untrusted: every header stays on BIOS, OpenRGB
//! is not started, and no config is read. Before any file in the home is read, [`SecureDir::trusted`]
//! checks its owner again.

use std::io;
use std::path::{Path, PathBuf};

/// Owner SYSTEM. DACL: SYSTEM full, Administrators full, Users read and execute (`0x1200a9`),
/// all inheritable, protected (`P`).
pub const SECURE_SDDL: &str = "O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

/// Owner SYSTEM. DACL: SYSTEM full, Administrators full, nobody else, inheritable, protected. For
/// folders no ordinary user may even read, such as the update staging folder (#43).
pub const PRIVATE_SDDL: &str = "O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// The program the service runs as SYSTEM, in the install folder.
pub const SERVICE_EXE: &str = "canvas-rgb-svc.exe";

/// Subfolders the service creates inside the home, so they are secured and checked with it.
pub const SUBDIRS: [&str; 2] = ["openrgb", "logs"];

/// SID strings of the owners that are trusted: LocalSystem and BUILTIN\Administrators.
pub const TRUSTED_OWNER_SIDS: [&str; 2] = ["S-1-5-18", "S-1-5-32-544"];

/// `FILE_ATTRIBUTE_REPARSE_POINT`: a junction or symlink.
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub fn owner_is_trusted_sid(sid: &str) -> bool {
    TRUSTED_OWNER_SIDS.contains(&sid)
}

/// Identity of a file on disk: volume serial and file index. Stable across renames within a volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileId {
    pub volume: u32,
    pub index: u64,
}

/// The filesystem operations the decision logic needs. Real: [`WinFs`]. Tests: a fake tree.
/// There is deliberately no delete, no rename and no ownership change.
pub trait FsAcl: Send {
    fn exists(&self, p: &Path) -> io::Result<bool>;
    /// True for a directory, including a directory junction (`FILE_ATTRIBUTE_DIRECTORY`).
    fn is_dir(&self, p: &Path) -> io::Result<bool>;
    /// True for junctions and symlinks. Checked without following them.
    fn is_reparse(&self, p: &Path) -> io::Result<bool>;
    /// Direct children of a directory. Never called on a reparse point.
    fn list(&self, p: &Path) -> io::Result<Vec<PathBuf>>;
    fn owner_trusted(&self, p: &Path) -> io::Result<bool>;
    /// The file ID of `p`, opened without following a reparse point.
    fn file_id(&self, p: &Path) -> io::Result<FileId>;
    /// Creates a directory whose owner and DACL come from [`SECURE_SDDL`] at birth.
    fn create_dir_protected(&mut self, p: &Path) -> io::Result<()>;
    /// Creates a directory whose owner and DACL come from [`PRIVATE_SDDL`] at birth: SYSTEM and
    /// Administrators only. The update staging folder is made this way (#43).
    fn create_dir_private(&mut self, _p: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    /// Replaces the DACL of `p` with [`SECURE_SDDL`]'s protected DACL. The owner is not changed.
    fn apply_protected_dacl(&mut self, p: &Path) -> io::Result<()>;
    /// True if the entry's DACL has any ACCESS_ALLOWED ACE granting a write-class right to any SID other
    /// than SYSTEM (`S-1-5-18`) and Administrators (`S-1-5-32-544`). A NULL DACL counts as true.
    fn has_nonadmin_write(&self, p: &Path) -> io::Result<bool>;
    /// True if the owner of a PARENT folder is SYSTEM, Administrators or TrustedInstaller. Windows
    /// itself owns `C:\` and `C:\Program Files` as TrustedInstaller, so the strict owner rule of
    /// [`FsAcl::owner_trusted`] would refuse the default install path (#46).
    fn owner_trusted_parent(&self, p: &Path) -> io::Result<bool>;
    /// True if the entry's DACL has an ACCESS_ALLOWED ACE (not inherit-only) that gives a principal
    /// other than SYSTEM, Administrators or TrustedInstaller WRITE_DAC, WRITE_OWNER or FILE_DELETE_CHILD.
    /// Those three rights on a parent folder let its holder swap the install folder out from under a
    /// check. DELETE counts too, except on a drive root (which cannot be renamed): a Modify ACE includes
    /// it, and with it a user can rename the parent away and rebuild the path with their own files in
    /// it. The bits are read, not the ACE name (#46). A NULL DACL counts as true.
    fn has_nonadmin_control(&self, p: &Path, is_drive_root: bool) -> io::Result<bool>;
}

/// Secures a directory and reports whether a path can be trusted.
pub trait SecureDir: Send {
    /// Creates `dir` if needed and secures it. An `Err` means the home cannot be trusted, and
    /// nothing in it may be read or launched from.
    fn secure(&mut self, dir: &Path) -> io::Result<()>;
    /// True when `path` exists, is not a reparse point, and its owner is SYSTEM or Administrators.
    fn trusted(&self, path: &Path) -> bool;
}

pub(crate) fn refused(path: &Path, why: &str) -> io::Error {
    io::Error::other(format!("{}: {why}", path.display()))
}

/// Logs the offending path at ERROR and returns the failure. Nothing is deleted or renamed.
pub(crate) fn offending(path: &Path, what: &str) -> io::Error {
    tracing::error!(
        path = %path.display(),
        "machine home: {what}. canvas-rgb will not read or launch from the home until it is removed. \
         Check the path, then remove it by hand from an elevated prompt (rd /s). Nothing was deleted or renamed."
    );
    refused(path, &format!("{what}; check it and remove it by hand (elevated rd /s)"))
}

/// Read-only check of everything under `dir`. Fails on the first junction, symlink, or entry whose
/// owner is not trusted. A junction is never followed: it is refused before its children are read.
pub(crate) fn check_tree(fs: &dyn FsAcl, dir: &Path) -> io::Result<()> {
    for child in fs.list(dir)? {
        if fs.is_reparse(&child)? {
            return Err(offending(&child, "a junction or symlink is in the home"));
        }
        match fs.owner_trusted(&child) {
            Ok(true) => {}
            Ok(false) => return Err(offending(&child, "an entry is owned by someone other than SYSTEM or Administrators")),
            Err(e) => return Err(offending(&child, &format!("its owner cannot be read ({e})"))),
        }
        // Check for non-admin write permissions
        match fs.has_nonadmin_write(&child) {
            Ok(true) => return Err(offending(&child, "an entry has write access granted to a non-admin principal")),
            Ok(false) => {}
            Err(e) => return Err(offending(&child, &format!("cannot check write permissions ({e})"))),
        }
        if fs.is_dir(&child)? {
            check_tree(fs, &child)?;
        }
    }
    Ok(())
}

/// Names the install-folder failure: logs it at ERROR with the path and returns it. Nothing is deleted,
/// renamed or adopted; the person fixes the folder, or installs into the default one.
pub(crate) fn install_offending(path: &Path, what: &str) -> io::Error {
    tracing::error!(
        path = %path.display(),
        "install folder: {what}. canvas-rgb runs programs from its install folder as the system, so it will not \
         start an update from here. Nothing was deleted or renamed. Install canvas-rgb into a folder only \
         administrators can change (the default is under Program Files)."
    );
    refused(path, &format!("{what}; install canvas-rgb into a folder only administrators can change"))
}

/// Checks one path inside the install folder: it exists, is the kind it should be, is no junction or
/// symlink, is owned by SYSTEM or Administrators, and no one else can write it.
fn check_install_entry(fs: &dyn FsAcl, path: &Path, is_folder: bool) -> io::Result<()> {
    if !fs.exists(path)? {
        return Err(install_offending(path, "an install path canvas-rgb needs is missing"));
    }
    if fs.is_reparse(path)? {
        return Err(install_offending(path, "an install path is a junction or symlink"));
    }
    if fs.is_dir(path)? != is_folder {
        return Err(refused(path, if is_folder { "is not a folder" } else { "is not a file" }));
    }
    if !fs.owner_trusted(path)? {
        return Err(install_offending(path, "an install path is owned by someone other than SYSTEM or Administrators"));
    }
    if fs.has_nonadmin_write(path)? {
        return Err(install_offending(path, "an install path has write access granted to a non-admin principal"));
    }
    Ok(())
}

/// Walks every entry under `dir` (a junction is never followed): none may be a junction or symlink,
/// owned by anyone but SYSTEM or Administrators, or writable by anyone else.
fn check_install_tree(fs: &dyn FsAcl, dir: &Path) -> io::Result<()> {
    for child in fs.list(dir)? {
        let folder = fs.is_dir(&child)? && !fs.is_reparse(&child)?;
        check_install_entry(fs, &child, fs.is_dir(&child)?)?;
        if folder {
            check_install_tree(fs, &child)?;
        }
    }
    Ok(())
}

/// Every folder above `app`, up to and including the drive root: no junction or symlink, owned by
/// SYSTEM, Administrators or TrustedInstaller, and no one else holds WRITE_DAC, WRITE_OWNER or
/// FILE_DELETE_CHILD on it. Someone who holds one of those on a parent can rename the install folder
/// away and put their own in its place, whatever the install folder's own ACL says.
pub(crate) fn check_install_parents(fs: &dyn FsAcl, app: &Path) -> io::Result<()> {
    for parent in app.ancestors().skip(1) {
        if !fs.exists(parent)? {
            continue;
        }
        // A drive root has no reparse attribute worth reading.
        if parent.parent().is_some() && fs.is_reparse(parent)? {
            return Err(install_offending(parent, "the install path goes through a junction or symlink"));
        }
        if !fs.owner_trusted_parent(parent)? {
            return Err(install_offending(
                parent,
                "a folder above the install folder is owned by someone other than SYSTEM, Administrators or TrustedInstaller",
            ));
        }
        if fs.has_nonadmin_control(parent, parent.parent().is_none())? {
            return Err(install_offending(
                parent,
                "a folder above the install folder lets a non-admin change its permissions, rename it or delete what is in it; choose a folder under Program Files or directly under a drive",
            ));
        }
    }
    Ok(())
}

/// The one install-folder check (#46). The update path and the service start use it. It changes
/// nothing and names the first path that fails:
/// - every folder above `app` ([`check_install_parents`]),
/// - `app`, `installer\`, `canvas-rgb.exe`, `canvas-rgb-svc.exe` (the exe the service runs as SYSTEM),
///   the scripts the finish step runs, and a left-over `update-result.json`,
/// - the whole `openrgb\` tree when there is one (OpenRGB runs as SYSTEM too).
pub fn check_install_folder(fs: &dyn FsAcl, app: &Path) -> io::Result<()> {
    check_install_parents(fs, app)?;
    let installer = app.join("installer");
    // setup-user.ps1 dot-sources lib.ps1, so it is run as part of the script.
    let mut all: Vec<(PathBuf, bool)> = vec![
        (app.to_path_buf(), true),
        (installer.clone(), true),
        (installer.join("setup-user.ps1"), false),
        (installer.join("lib.ps1"), false),
        (app.join("canvas-rgb.exe"), false),
        (app.join(SERVICE_EXE), false),
    ];
    // A result file left by an earlier run is read too, so it is checked when it is there.
    let result = app.join("update-result.json");
    if fs.exists(&result)? {
        all.push((result, false));
    }
    for (path, is_folder) in all {
        check_install_entry(fs, &path, is_folder)?;
    }
    let openrgb = app.join("openrgb");
    if fs.exists(&openrgb)? {
        check_install_entry(fs, &openrgb, true)?;
        check_install_tree(fs, &openrgb)?;
    }
    Ok(())
}

/// The decision logic. See the module docs for the order.
pub fn secure_home(fs: &mut dyn FsAcl, dir: &Path) -> io::Result<()> {
    if fs.exists(dir)? {
        if fs.is_reparse(dir)? {
            return Err(offending(dir, "the home is a junction or symlink"));
        }
        if !fs.is_dir(dir)? {
            return Err(refused(dir, "exists and is not a directory"));
        }
        if !fs.owner_trusted(dir)? {
            return Err(offending(dir, "the home is owned by someone other than SYSTEM or Administrators"));
        }
    } else {
        fs.create_dir_protected(dir)?;
    }
    let first = fs.file_id(dir)?;
    // Read-only. Nothing is changed unless every entry under the home passes.
    check_tree(&*fs, dir)?;
    fs.apply_protected_dacl(dir)?;
    for sub in SUBDIRS {
        let path = dir.join(sub);
        if fs.exists(&path)? {
            if !fs.is_dir(&path)? {
                return Err(refused(&path, "exists and is not a directory"));
            }
        } else {
            fs.create_dir_protected(&path)?;
        }
    }
    let again = fs.file_id(dir)?;
    if again != first {
        tracing::error!(home = %dir.display(), "machine home changed identity while it was being secured; failing closed");
        return Err(refused(dir, "file ID changed while securing; refusing"));
    }
    Ok(())
}

/// [`SecureDir`] over any [`FsAcl`].
pub struct Secured<F: FsAcl> {
    fs: F,
}

impl<F: FsAcl> Secured<F> {
    pub fn new(fs: F) -> Self {
        Self { fs }
    }
}

impl<F: FsAcl> SecureDir for Secured<F> {
    fn secure(&mut self, dir: &Path) -> io::Result<()> {
        secure_home(&mut self.fs, dir)
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
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

#[cfg(windows)]
fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().chain(Some(0)).collect()
}

/// [`SECURE_SDDL`] parsed into a self-relative security descriptor. Freed on drop.
#[cfg(windows)]
struct SddlDescriptor(*mut std::ffi::c_void);

#[cfg(windows)]
impl SddlDescriptor {
    fn parse() -> io::Result<Self> {
        Self::parse_sddl(SECURE_SDDL)
    }

    fn parse_sddl(text: &str) -> io::Result<Self> {
        use std::ffi::c_void;
        use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;

        let sddl: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; `raw` receives a LocalAlloc'd buffer freed in Drop.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut raw, std::ptr::null_mut())
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(raw))
    }
}

#[cfg(windows)]
impl Drop for SddlDescriptor {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::LocalFree;
        // SAFETY: `self.0` came from ConvertStringSecurityDescriptor... and is freed once.
        unsafe {
            LocalFree(self.0);
        }
    }
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
        use std::os::windows::fs::MetadataExt;
        // Read the attribute directly: std reports a directory junction as a symlink, not a directory.
        Ok(std::fs::symlink_metadata(p)?.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0)
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

    fn file_id(&self, p: &Path) -> io::Result<FileId> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING,
        };

        let target = wide(p.as_os_str());
        // SAFETY: `target` is NUL-terminated; attributes-only access, no security attributes, no template.
        let handle = unsafe {
            CreateFileW(
                target.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is a valid out-buffer for the call below.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `handle` is open and valid; `info` is a valid out-buffer.
        let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
        let result = if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(FileId {
                volume: info.dwVolumeSerialNumber,
                index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            })
        };
        // SAFETY: `handle` was opened above and is closed exactly once.
        unsafe {
            CloseHandle(handle);
        }
        result
    }

    fn create_dir_protected(&mut self, p: &Path) -> io::Result<()> {
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

        let descriptor = SddlDescriptor::parse()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let target = wide(p.as_os_str());
        // SAFETY: `target` is NUL-terminated; `attributes` points at the descriptor, alive for the call.
        let ok = unsafe { CreateDirectoryW(target.as_ptr(), &attributes) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn create_dir_private(&mut self, p: &Path) -> io::Result<()> {
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

        let descriptor = SddlDescriptor::parse_sddl(PRIVATE_SDDL)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let target = wide(p.as_os_str());
        // SAFETY: `target` is NUL-terminated; `attributes` points at the descriptor, alive for the call.
        let ok = unsafe { CreateDirectoryW(target.as_ptr(), &attributes) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn apply_protected_dacl(&mut self, p: &Path) -> io::Result<()> {
        use windows_sys::core::BOOL;
        use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        };

        let descriptor = SddlDescriptor::parse()?;
        let mut present: BOOL = 0;
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut defaulted: BOOL = 0;
        // SAFETY: `descriptor` is valid until Drop; the out pointers are valid locals.
        if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let target = wide(p.as_os_str());
        // Only the DACL is set. Null owner and group leave them as they are.
        // SAFETY: `target` is NUL-terminated; `dacl` points into `descriptor`, alive for the call.
        let status = unsafe {
            SetNamedSecurityInfoW(
                target.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null(),
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    fn has_nonadmin_write(&self, p: &Path) -> io::Result<bool> {
        dacl_grants_nonadmin(p, ENTRY_WRITE_MASK)
    }

    fn owner_trusted_parent(&self, p: &Path) -> io::Result<bool> {
        Ok(writer_is_trusted_sid(&owner_sid_string(p)?))
    }

    fn has_nonadmin_control(&self, p: &Path, is_drive_root: bool) -> io::Result<bool> {
        dacl_grants_nonadmin(p, if is_drive_root { PARENT_CONTROL_MASK } else { PARENT_NONROOT_MASK })
    }
}

/// `ACCESS_ALLOWED_ACE_TYPE`: the only ACE type that grants rights.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
/// `INHERIT_ONLY_ACE`: the ACE applies to children only, not to the object itself.
const INHERIT_ONLY_ACE_FLAG: u8 = 0x08;
const GENERIC_WRITE_BIT: u32 = 0x4000_0000;
const GENERIC_ALL_BIT: u32 = 0x1000_0000;
/// Write-class rights: FILE_WRITE_DATA (= FILE_ADD_FILE), FILE_APPEND_DATA (= FILE_ADD_SUBDIRECTORY),
/// FILE_WRITE_EA, FILE_DELETE_CHILD, FILE_WRITE_ATTRIBUTES, DELETE, WRITE_DAC, WRITE_OWNER.
const WRITE_CLASS_RIGHTS: u32 = 0x0000_0002
    | 0x0000_0004
    | 0x0000_0010
    | 0x0000_0040
    | 0x0000_0100
    | 0x0001_0000
    | 0x0004_0000
    | 0x0008_0000;

/// What counts as write access on an entry inside the install folder.
pub(crate) const ENTRY_WRITE_MASK: u32 = WRITE_CLASS_RIGHTS | GENERIC_WRITE_BIT | GENERIC_ALL_BIT;

/// What counts as control over a PARENT folder: FILE_DELETE_CHILD (0x40), WRITE_DAC (0x40000) and
/// WRITE_OWNER (0x80000), or GENERIC_ALL. A plain Modify ACE (0x1301bf) has none of them.
pub(crate) const PARENT_CONTROL_MASK: u32 = 0x0000_0040 | 0x0004_0000 | 0x0008_0000 | GENERIC_ALL_BIT;

/// The same for a parent that is not a drive root: DELETE (0x10000) too, because a user holding it can
/// rename that parent away and rebuild the install path (#46).
pub(crate) const PARENT_NONROOT_MASK: u32 = PARENT_CONTROL_MASK | 0x0001_0000;

/// The string form of a SID (`S-1-5-32-545`, ...).
#[cfg(windows)]
fn sid_string(sid: *mut std::ffi::c_void) -> io::Result<String> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` points at a valid SID inside a descriptor the caller keeps alive; `text` receives a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut len = 0usize;
    // SAFETY: the converted string is NUL-terminated.
    let result = unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    // SAFETY: `text` came from ConvertSidToStringSidW and is freed once.
    unsafe {
        LocalFree(text as *mut c_void);
    }
    Ok(result)
}

/// True if the ACL has an ALLOWED ACE that is not inherit-only, grants a right in `mask`, and names a
/// SID other than SYSTEM, Administrators or TrustedInstaller.
#[cfg(windows)]
fn acl_grants_nonadmin(dacl: *mut windows_sys::Win32::Security::ACL, mask: u32) -> io::Result<bool> {
    use std::ffi::c_void;
    use windows_sys::Win32::Security::{GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER};

    // SAFETY: `dacl` is a valid ACL from GetNamedSecurityInfoW, alive while the caller holds its descriptor.
    let count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(count) {
        let mut ace: *mut c_void = std::ptr::null_mut();
        // SAFETY: `ace` receives a pointer into the ACL, which is alive.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let header = ace as *const ACE_HEADER;
        // SAFETY: GetAce returned a valid ACE, so its header is readable.
        let (ace_type, ace_flags) = unsafe { ((*header).AceType, (*header).AceFlags) };
        let allowed = ace as *const ACCESS_ALLOWED_ACE;
        // SAFETY: an ACCESS_ALLOWED ACE has this layout; the mask is read only when the type says so.
        let ace_mask = if ace_type == ACCESS_ALLOWED_ACE_TYPE { unsafe { (*allowed).Mask } } else { 0 };
        let grants = ace_grants_untrusted(ace_type, ace_flags, ace_mask, mask, || {
            // SAFETY: `SidStart` is inside the ACE, which is inside the ACL.
            let sid = unsafe { std::ptr::addr_of!((*allowed).SidStart) } as *mut c_void;
            sid_string(sid)
        })?;
        if grants {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The decision for one ACE, apart from reading it: true when it gives a principal other than SYSTEM,
/// Administrators or TrustedInstaller a right in `mask`. `sid` is read only when the mask matches.
/// Inherit-only ACEs do not apply to the object itself. An allow-type ACE this check cannot parse, or a
/// type it does not know, counts as granting (fail closed).
pub(crate) fn ace_grants_untrusted(
    ace_type: u8,
    ace_flags: u8,
    ace_mask: u32,
    mask: u32,
    sid: impl FnOnce() -> io::Result<String>,
) -> io::Result<bool> {
    if ace_flags & INHERIT_ONLY_ACE_FLAG != 0 {
        return Ok(false);
    }
    if ace_type != ACCESS_ALLOWED_ACE_TYPE {
        return Ok(!ace_type_grants_nothing(ace_type));
    }
    if ace_mask & mask == 0 {
        return Ok(false);
    }
    Ok(!writer_is_trusted_sid(&sid()?))
}

/// `NT SERVICE\TrustedInstaller`. Every folder under `C:\Program Files` carries a full-control ACE
/// for it (the install folder included), and it can already replace System32.
pub const TRUSTED_INSTALLER_SID: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// Who may hold write access on a trusted path: the trusted owners, plus TrustedInstaller. The
/// owner rule stays [`TRUSTED_OWNER_SIDS`] only.
pub fn writer_is_trusted_sid(sid: &str) -> bool {
    owner_is_trusted_sid(sid) || sid == TRUSTED_INSTALLER_SID
}

/// ACE types that cannot grant access: the deny, audit, alarm and label families. Every other type
/// other than plain ACCESS_ALLOWED (0) is an allow-type this check does not parse, or unknown.
pub(crate) fn ace_type_grants_nothing(ace_type: u8) -> bool {
    matches!(ace_type, 1 | 2 | 3 | 6 | 7 | 8 | 10 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19)
}

/// The DACL check for `p`. A NULL DACL grants everyone everything, so it counts as true.
#[cfg(windows)]
fn dacl_grants_nonadmin(p: &Path, mask: u32) -> io::Result<bool> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, ACL};

    let target = wide(p.as_os_str());
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    // SAFETY: `target` is NUL-terminated; only the DACL is requested; `dacl` points into `descriptor`,
    // which is freed below after the DACL has been read.
    let status = unsafe {
        GetNamedSecurityInfoW(
            target.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let result = if dacl.is_null() { Ok(true) } else { acl_grants_nonadmin(dacl, mask) };
    // SAFETY: `descriptor` came from GetNamedSecurityInfoW and is freed once; `dacl` is not used after this.
    unsafe {
        LocalFree(descriptor);
    }
    result
}

#[cfg(all(test, windows))]
mod win_tests {
    use super::*;
    use std::ffi::c_void;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };

    /// Puts the protected DACL described by `sddl` (`D:P(...)`) on `path`. The owner is not changed.
    fn set_dacl(path: &Path, sddl: &str) -> io::Result<()> {
        let text: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        // SAFETY: `text` is NUL-terminated; `descriptor` receives a LocalAlloc'd buffer freed below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(), 1, &mut descriptor, std::ptr::null_mut())
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| -> io::Result<()> {
            let mut present: BOOL = 0;
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut defaulted: BOOL = 0;
            // SAFETY: `descriptor` is valid until LocalFree below; the out-pointers are valid locals.
            if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let target = wide(path.as_os_str());
            // SAFETY: `target` is NUL-terminated; `dacl` points into `descriptor`, alive for the call.
            let status = unsafe {
                SetNamedSecurityInfoW(
                    target.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    dacl,
                    std::ptr::null(),
                )
            };
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            Ok(())
        })();
        // SAFETY: `descriptor` came from the conversion above and is freed once.
        unsafe {
            LocalFree(descriptor);
        }
        result
    }

    /// Restores a DACL that gives the owner full control, so the temp directory can be deleted.
    /// Declared after the TempDir in each test, so it runs first.
    struct Restore<'a>(&'a Path);

    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let _ = set_dacl(self.0, "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;OW)");
        }
    }

    /// A temp directory with a DACL set from `sddl`, checked with the real Win32 path.
    fn check(sddl: &str) -> io::Result<bool> {
        let dir = tempfile::tempdir()?;
        let _restore = Restore(dir.path());
        set_dacl(dir.path(), sddl)?;
        has_nonadmin_write_on(dir.path())
    }

    fn has_nonadmin_write_on(p: &Path) -> io::Result<bool> {
        dacl_grants_nonadmin(p, ENTRY_WRITE_MASK)
    }

    #[test]
    fn a_dacl_of_only_system_and_administrators_has_no_nonadmin_write() {
        assert!(!check("D:P(A;;FA;;;SY)(A;;FA;;;BA)").unwrap());
    }

    #[test]
    fn an_allowed_add_file_ace_for_users_is_nonadmin_write() {
        // FILE_ADD_FILE (0x2) for BUILTIN\Users (S-1-5-32-545).
        assert!(check("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x2;;;BU)").unwrap());
    }

    #[test]
    fn a_callback_allow_ace_for_users_fails_closed_on_a_real_dacl() {
        // A conditional (callback) allow ACE is not plain ACCESS_ALLOWED; the walk must not skip it.
        assert!(check("D:P(A;;FA;;;SY)(A;;FA;;;BA)(XA;;FA;;;BU;(1==1))").unwrap());
        // A callback ACE for a trusted SID is still an allow-type the check does not parse: closed too.
        assert!(check("D:P(A;;FA;;;SY)(XA;;FA;;;BA;(1==1))").unwrap());
    }

    #[test]
    fn allow_type_aces_other_than_plain_allowed_fail_closed() {
        for t in [4u8, 5, 9, 11] {
            assert!(!ace_type_grants_nothing(t), "type {t} is an allow type and must count as write");
        }
        assert!(!ace_type_grants_nothing(0x20), "an unknown type must count as write");
        for t in [1u8, 2, 3, 6, 7, 8, 10, 12, 13, 14, 15, 16, 17, 18, 19] {
            assert!(ace_type_grants_nothing(t), "type {t} grants nothing");
        }
    }

    #[test]
    fn an_inherit_only_users_write_ace_does_not_count() {
        // IO = INHERIT_ONLY_ACE: it applies to children, not to this directory.
        assert!(!check("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;OIIO;FW;;;BU)").unwrap());
    }

    #[test]
    fn a_program_files_style_dacl_with_trustedinstaller_has_no_nonadmin_write() {
        // The DACL Inno leaves on C:\Program Files\canvas-rgb (read with Get-Acl, 2026-10-31).
        let sddl = format!(
            "D:P(A;;FA;;;{TRUSTED_INSTALLER_SID})(A;CIIO;GA;;;{TRUSTED_INSTALLER_SID})(A;;FA;;;SY)(A;OICIIO;GA;;;SY)\
             (A;;FA;;;BA)(A;OICIIO;GA;;;BA)(A;;0x1200a9;;;BU)(A;OICIIO;GXGR;;;BU)(A;OICIIO;GA;;;CO)\
             (A;;0x1200a9;;;AC)(A;OICIIO;GXGR;;;AC)"
        );
        assert!(!check(&sddl).unwrap());
        // Any other service SID with write is still refused.
        assert!(check("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;S-1-5-80-1-2-3-4-5)").unwrap());
    }

    #[test]
    fn the_real_system32_folder_has_no_nonadmin_write() {
        let root = std::env::var_os("SystemRoot").expect("SystemRoot");
        assert!(!has_nonadmin_write_on(&Path::new(&root).join("System32")).unwrap());
    }

    #[test]
    fn trustedinstaller_may_write_but_never_owns() {
        assert!(writer_is_trusted_sid(TRUSTED_INSTALLER_SID));
        assert!(!owner_is_trusted_sid(TRUSTED_INSTALLER_SID));
        assert!(!writer_is_trusted_sid("S-1-5-32-545"), "Users");
    }

    #[test]
    fn a_users_read_only_ace_is_not_write() {
        assert!(!check("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;BU)").unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq)]
    struct Node {
        dir: bool,
        reparse: bool,
        trusted: bool,
        id: u64,
    }

    /// A tree in memory. It never deletes or renames. It records what was listed, created and
    /// locked down, so tests can check that a failed step changed nothing.
    #[derive(Default)]
    struct FakeFs {
        nodes: BTreeMap<PathBuf, Node>,
        /// Paths whose children were listed (i.e. read).
        listed: Arc<Mutex<Vec<PathBuf>>>,
        created: Vec<PathBuf>,
        dacl: Vec<PathBuf>,
        next_id: u64,
        /// When set, the node at this path gets a new file ID the moment its DACL is applied,
        /// as if the home had been swapped for another directory during the step.
        swap_id_on: Option<PathBuf>,
        /// Tracks which paths have non-admin write permissions (for testing)
        nonadmin_write_on: Vec<PathBuf>,
        /// Paths that give a non-admin WRITE_DAC, WRITE_OWNER or FILE_DELETE_CHILD.
        nonadmin_control_on: Vec<PathBuf>,
        /// Paths that give a non-admin DELETE only (a plain Modify ACE).
        nonadmin_delete_on: Vec<PathBuf>,
        /// Paths owned by TrustedInstaller: a parent may be, an entry in the install folder may not.
        installer_owned: Vec<PathBuf>,
    }

    impl FakeFs {
        fn with(mut self, path: &str, node: Node) -> Self {
            self.nodes.insert(PathBuf::from(path), node);
            self
        }

        fn fresh_id(&mut self) -> u64 {
            self.next_id += 1;
            self.next_id
        }

        fn snapshot(&self) -> BTreeMap<PathBuf, Node> {
            self.nodes.clone()
        }

        fn touched(&self) -> bool {
            !self.created.is_empty() || !self.dacl.is_empty()
        }
    }

    fn dir(trusted: bool) -> Node {
        Node { dir: true, reparse: false, trusted, id: 0 }
    }

    fn file(trusted: bool) -> Node {
        Node { dir: false, reparse: false, trusted, id: 0 }
    }

    fn link(dir: bool, trusted: bool) -> Node {
        Node { dir, reparse: true, trusted, id: 0 }
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
            let mut out: Vec<PathBuf> = self.nodes.keys().filter(|k| k.parent() == Some(p)).cloned().collect();
            out.sort();
            Ok(out)
        }
        fn owner_trusted(&self, p: &Path) -> io::Result<bool> {
            self.nodes.get(p).map(|n| n.trusted).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn file_id(&self, p: &Path) -> io::Result<FileId> {
            self.nodes
                .get(p)
                .map(|n| FileId { volume: 1, index: n.id })
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn create_dir_protected(&mut self, p: &Path) -> io::Result<()> {
            if self.nodes.contains_key(p) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            let id = self.fresh_id();
            // Born protected: owned by a trusted SID.
            self.nodes.insert(p.to_path_buf(), Node { dir: true, reparse: false, trusted: true, id });
            self.created.push(p.to_path_buf());
            Ok(())
        }
        fn apply_protected_dacl(&mut self, p: &Path) -> io::Result<()> {
            if !self.nodes.contains_key(p) {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            self.dacl.push(p.to_path_buf());
            if self.swap_id_on.as_deref() == Some(p) {
                let id = self.fresh_id();
                if let Some(n) = self.nodes.get_mut(p) {
                    n.id = id;
                }
            }
            Ok(())
        }

        fn has_nonadmin_write(&self, p: &Path) -> io::Result<bool> {
            // Check if this path has non-admin write permissions (for testing)
            if self.nonadmin_write_on.contains(&p.to_path_buf()) {
                Ok(true)
            } else {
                Ok(false)
            }
        }

        fn owner_trusted_parent(&self, p: &Path) -> io::Result<bool> {
            if self.installer_owned.iter().any(|i| i == p) {
                return Ok(true);
            }
            self.owner_trusted(p)
        }

        fn has_nonadmin_control(&self, p: &Path, is_drive_root: bool) -> io::Result<bool> {
            // A DELETE grant (a plain Modify) counts for every parent except a drive root.
            let deletable = !is_drive_root && self.nonadmin_delete_on.iter().any(|c| c == p);
            Ok(deletable || self.nonadmin_control_on.iter().any(|c| c == p))
        }
    }

    const HOME: &str = r"C:\ProgramData\canvas-rgb";

    fn under_home(rest: &str) -> String {
        format!(r"{HOME}\{rest}")
    }

    #[test]
    fn a_fresh_home_is_born_protected_with_its_subfolders() {
        let mut fs = FakeFs::default();
        secure_home(&mut fs, Path::new(HOME)).unwrap();
        assert!(fs.created.contains(&PathBuf::from(HOME)), "home created");
        assert!(fs.created.contains(&PathBuf::from(under_home("openrgb"))), "openrgb created");
        assert!(fs.created.contains(&PathBuf::from(under_home("logs"))), "logs created");
        assert_eq!(fs.dacl, vec![PathBuf::from(HOME)], "protected DACL on the home only");
    }

    #[test]
    fn an_untrusted_home_fails_closed_with_nothing_changed() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(false))
            .with(&under_home("fans.toml"), file(false));
        let before = fs.snapshot();
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains("owned by someone other than SYSTEM or Administrators"), "{err}");
        assert!(err.to_string().contains("remove it by hand"), "{err}");
        assert_eq!(fs.snapshot(), before, "the tree is unchanged");
        assert!(!fs.touched(), "nothing created or locked down");
        assert!(fs.listed.lock().unwrap().is_empty(), "the home was never listed");
    }

    #[test]
    fn a_reparse_home_fails_closed_with_nothing_changed() {
        let mut fs = FakeFs::default().with(HOME, link(true, true));
        let before = fs.snapshot();
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains("junction or symlink"), "{err}");
        assert_eq!(fs.snapshot(), before, "the tree is unchanged");
        assert!(!fs.touched());
    }

    #[test]
    fn an_untrusted_child_fails_closed_and_nothing_is_deleted() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("openrgb"), dir(false))
            .with(&under_home(r"openrgb\evil.dll"), file(false));
        let before = fs.snapshot();
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains(&under_home("openrgb")), "names the path: {err}");
        assert!(err.to_string().contains("remove it by hand"), "{err}");
        assert_eq!(fs.snapshot(), before, "no entry deleted, renamed or re-owned");
        assert!(!fs.touched(), "no DACL applied and nothing created");
    }

    #[test]
    fn an_untrusted_grandchild_fails_closed_and_nothing_is_deleted() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("openrgb"), dir(true))
            .with(&under_home(r"openrgb\plugins"), dir(false))
            .with(&under_home(r"openrgb\plugins\evil.dll"), file(false));
        let before = fs.snapshot();
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains(&under_home(r"openrgb\plugins")), "names the path: {err}");
        assert_eq!(fs.snapshot(), before, "nothing deleted");
        assert!(!fs.touched());
    }

    #[test]
    fn a_reparse_child_fails_closed_and_its_target_is_never_read() {
        let victim = r"C:\Windows\victim.dll";
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("logs"), link(true, true))
            .with(victim, file(true));
        let before = fs.snapshot();
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains(&under_home("logs")), "names the link: {err}");
        assert!(err.to_string().contains("junction or symlink"), "{err}");
        let listed = fs.listed.lock().unwrap();
        assert!(!listed.iter().any(|p| p.ends_with("logs")), "the link is never listed");
        assert!(!listed.iter().any(|p| p.starts_with(r"C:\Windows")), "the target is never listed");
        drop(listed);
        assert_eq!(fs.snapshot(), before, "the link and its target are untouched");
    }

    #[test]
    fn a_reparse_grandchild_fails_closed() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("openrgb"), dir(true))
            .with(&under_home(r"openrgb\link"), link(true, true));
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains(&under_home(r"openrgb\link")), "{err}");
        let listed = fs.listed.lock().unwrap();
        assert!(!listed.iter().any(|p| p.ends_with("link")), "the link is never listed");
    }

    #[test]
    fn a_trusted_existing_home_keeps_its_children_and_gets_the_dacl_on_the_home_only() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("openrgb"), dir(true))
            .with(&under_home("logs"), dir(true))
            .with(&under_home("fans.toml"), file(true));
        let before = fs.snapshot();
        secure_home(&mut fs, Path::new(HOME)).unwrap();
        assert!(fs.created.is_empty(), "every subfolder already exists and is kept");
        assert_eq!(fs.dacl, vec![PathBuf::from(HOME)], "the DACL goes on the home only");
        assert_eq!(fs.snapshot(), before, "children are kept as they are");
    }

    #[test]
    fn a_missing_subfolder_is_created_protected() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("openrgb"), dir(true));
        secure_home(&mut fs, Path::new(HOME)).unwrap();
        assert_eq!(fs.created, vec![PathBuf::from(under_home("logs"))]);
    }

    #[test]
    fn a_file_where_a_subfolder_belongs_fails_closed() {
        let mut fs = FakeFs::default().with(HOME, dir(true)).with(&under_home("openrgb"), file(true));
        let before = fs.snapshot();
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");
        assert_eq!(fs.snapshot(), before);
    }

    #[test]
    fn a_file_id_that_changes_while_securing_fails_closed() {
        let mut fs = FakeFs { swap_id_on: Some(PathBuf::from(HOME)), ..FakeFs::default() }.with(HOME, dir(true));
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains("file ID changed"), "{err}");
    }

    #[test]
    fn a_stable_file_id_passes() {
        let mut fs = FakeFs::default().with(HOME, dir(true));
        secure_home(&mut fs, Path::new(HOME)).unwrap();
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
            .with(&under_home("fans.toml"), file(false))
            .with(&under_home("link"), link(false, true));
        let secured = Secured::new(fs);
        assert!(secured.trusted(Path::new(HOME)));
        assert!(!secured.trusted(&PathBuf::from(under_home("fans.toml"))), "untrusted owner");
        assert!(!secured.trusted(&PathBuf::from(under_home("link"))), "reparse point");
        assert!(!secured.trusted(&PathBuf::from(under_home("missing.toml"))), "missing");
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

    #[test]
    fn a_trusted_owner_child_with_nonadmin_write_fails_closed_unchanged() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("trusted.txt"), file(true));
        // Mark the child as having non-admin write permissions
        fs.nonadmin_write_on.push(PathBuf::from(under_home("trusted.txt")));
        let before = fs.snapshot();
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains(&under_home("trusted.txt")), "{err}");
        assert!(err.to_string().contains("non-admin principal"), "{err}");
        assert_eq!(fs.snapshot(), before, "the tree is unchanged");
        assert!(!fs.touched(), "nothing created or locked down");
    }

    #[test]
    fn a_grandchild_with_nonadmin_write_fails_closed() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("subdir"), dir(true))
            .with(&under_home(r"subdir\file.txt"), file(true));
        // Mark the grandchild as having non-admin write permissions
        fs.nonadmin_write_on.push(PathBuf::from(under_home(r"subdir\file.txt")));
        let err = secure_home(&mut fs, Path::new(HOME)).unwrap_err();
        assert!(err.to_string().contains(&under_home(r"subdir\file.txt")), "{err}");
        assert!(err.to_string().contains("non-admin principal"), "{err}");
    }

    #[test]
    fn a_trusted_tree_with_no_nonadmin_write_passes() {
        let mut fs = FakeFs::default()
            .with(HOME, dir(true))
            .with(&under_home("file.txt"), file(true));
        // Ensure no non-admin write permissions
        let path_str = under_home("file.txt");
        let path = Path::new(&path_str);
        let has_write = fs.has_nonadmin_write(path).unwrap();
        assert!(!has_write, "no non-admin write permissions");
        secure_home(&mut fs, Path::new(HOME)).unwrap();
        assert!(fs.touched(), "the tree was secured");
    }

    // ---- the shared install-folder check (#46) ----

    const APP: &str = r"C:\Program Files\canvas-rgb";

    fn app_path(rest: &str) -> String {
        format!(r"{APP}\{rest}")
    }

    /// The default install as it is on a real PC: Windows' TrustedInstaller owns `C:\` and
    /// `C:\Program Files`, Administrators own the install folder, all of it admin-only.
    fn default_install() -> FakeFs {
        let mut fs = FakeFs::default()
            .with(r"C:\", dir(false))
            .with(r"C:\Program Files", dir(false))
            .with(APP, dir(true))
            .with(&app_path("installer"), dir(true))
            .with(&app_path(r"installer\setup-user.ps1"), file(true))
            .with(&app_path(r"installer\lib.ps1"), file(true))
            .with(&app_path("canvas-rgb.exe"), file(true))
            .with(&app_path("canvas-rgb-svc.exe"), file(true))
            .with(&app_path("openrgb"), dir(true))
            .with(&app_path(r"openrgb\OpenRGB.exe"), file(true))
            .with(&app_path(r"openrgb\plugins"), dir(true))
            .with(&app_path(r"openrgb\plugins\a.dll"), file(true));
        fs.installer_owned = vec![PathBuf::from(r"C:\"), PathBuf::from(r"C:\Program Files")];
        fs
    }

    fn check_default(fs: &FakeFs) -> io::Result<()> {
        check_install_folder(fs, Path::new(APP))
    }

    #[test]
    fn the_default_install_folder_passes_even_though_trustedinstaller_owns_the_parents() {
        check_default(&default_install()).unwrap();
    }

    #[test]
    fn a_trustedinstaller_owner_is_fine_on_a_parent_but_not_on_an_entry_inside_the_install_folder() {
        let mut fs = default_install();
        // The entry rule stays SYSTEM and Administrators only.
        fs.nodes.insert(PathBuf::from(app_path("canvas-rgb-svc.exe")), file(false));
        let err = check_default(&fs).unwrap_err().to_string();
        assert!(err.contains("canvas-rgb-svc.exe") && err.contains("owned by someone other"), "{err}");
    }

    /// Every way the install folder can be bad, each with the path the refusal must name.
    #[test]
    fn each_install_folder_failure_is_refused_and_names_its_path() {
        type Bad = fn(&mut FakeFs) -> String;
        let table: Vec<(&str, Bad, &str)> = vec![
            ("the service exe is writable by a non-admin", |f| { let p = app_path("canvas-rgb-svc.exe"); f.nonadmin_write_on.push(p.clone().into()); p }, "write access"),
            ("the service exe is owned by a user", |f| { let p = app_path("canvas-rgb-svc.exe"); f.nodes.insert(p.clone().into(), file(false)); p }, "owned by someone other"),
            ("the service exe is missing", |f| { let p = app_path("canvas-rgb-svc.exe"); f.nodes.remove(Path::new(&p)); p }, "missing"),
            ("a file deep in openrgb is writable", |f| { let p = app_path(r"openrgb\plugins\a.dll"); f.nonadmin_write_on.push(p.clone().into()); p }, "write access"),
            ("a folder in openrgb is owned by a user", |f| { let p = app_path(r"openrgb\plugins"); f.nodes.insert(p.clone().into(), dir(false)); p }, "owned by someone other"),
            ("openrgb holds a junction", |f| { let p = app_path(r"openrgb\plugins"); f.nodes.insert(p.clone().into(), link(true, true)); p }, "junction"),
            ("the install folder is a junction", |f| { f.nodes.insert(APP.into(), link(true, true)); APP.to_string() }, "junction"),
            ("a parent is a junction", |f| { let p = r"C:\Program Files".to_string(); f.nodes.insert(p.clone().into(), link(true, true)); p }, "junction"),
            ("a parent is owned by a user", |f| { let p = r"C:\Program Files".to_string(); f.installer_owned.clear(); f.nodes.insert(p.clone().into(), dir(false)); p }, "owned by someone other"),
            ("a parent lets a non-admin delete children or change its ACL", |f| { let p = r"C:\Program Files".to_string(); f.nonadmin_control_on.push(p.clone().into()); p }, "permissions, rename it or delete"),
            ("the drive root lets a non-admin change its ACL", |f| { f.nonadmin_control_on.push(PathBuf::from(r"C:\")); r"C:\".to_string() }, "permissions, rename it or delete"),
            ("a parent that is not a drive root gives a non-admin DELETE (a plain Modify ACE)", |f| { let p = r"C:\Program Files".to_string(); f.nonadmin_delete_on.push(p.clone().into()); p }, "choose a folder under Program Files or directly under a drive"),
            ("installer\\ is writable", |f| { let p = app_path("installer"); f.nonadmin_write_on.push(p.clone().into()); p }, "write access"),
            ("lib.ps1 is writable", |f| { let p = app_path(r"installer\lib.ps1"); f.nonadmin_write_on.push(p.clone().into()); p }, "write access"),
            ("setup-user.ps1 is writable", |f| { let p = app_path(r"installer\setup-user.ps1"); f.nodes.insert(p.clone().into(), file(true)); f.nonadmin_write_on.push(p.clone().into()); p }, "write access"),
            ("canvas-rgb.exe is writable", |f| { let p = app_path("canvas-rgb.exe"); f.nonadmin_write_on.push(p.clone().into()); p }, "write access"),
            ("update-result.json is writable", |f| { let p = app_path("update-result.json"); f.nodes.insert(p.clone().into(), file(true)); f.nonadmin_write_on.push(p.clone().into()); p }, "write access"),
            ("update-result.json is owned by a user", |f| { let p = app_path("update-result.json"); f.nodes.insert(p.clone().into(), file(false)); p }, "owned by someone other"),
        ];
        for (label, bad, expect) in table {
            let mut fs = default_install();
            let path = bad(&mut fs);
            let err = check_default(&fs).expect_err(label).to_string();
            assert!(err.contains(&path), "{label}: must name {path}: {err}");
            assert!(err.contains(expect), "{label}: must say {expect:?}: {err}");
        }
    }

    #[test]
    fn a_parent_that_is_missing_is_skipped_and_the_rest_still_checked() {
        let mut fs = default_install();
        fs.nodes.remove(Path::new(r"C:\Program Files"));
        check_default(&fs).unwrap();
    }

    #[test]
    fn the_install_check_changes_nothing() {
        let fs = default_install();
        let before = fs.snapshot();
        check_default(&fs).unwrap();
        assert!(!fs.touched() && fs.snapshot() == before);
    }

    /// The ACE table: (type, flags, ace mask, sid, mask asked, expected). The bits are read, not the ACE name.
    #[test]
    fn ace_table_decides_on_the_bits_and_the_sid() {
        const USERS: &str = "S-1-5-32-545";
        const AU: &str = "S-1-5-11";
        const SYSTEM: &str = "S-1-5-18";
        const BA: &str = "S-1-5-32-544";
        const MODIFY: u32 = 0x1301bf; // a plain Modify ACE: has DELETE, no FILE_DELETE_CHILD
        const READ_EXEC: u32 = 0x1200a9;
        const INHERIT_ONLY: u8 = 0x08;
        let rows: Vec<(u8, u8, u32, &str, u32, bool, &str)> = vec![
            (0, 0, MODIFY, AU, PARENT_CONTROL_MASK, false, "Modify has no delete-child, WRITE_DAC or WRITE_OWNER (a drive root, which cannot be renamed)"),
            (0, 0, MODIFY, AU, PARENT_NONROOT_MASK, true, "Modify has DELETE: a non-root parent could be renamed away"),
            (0, 0, 0x10000, USERS, PARENT_NONROOT_MASK, true, "DELETE alone on a non-root parent"),
            (0, 0, 0x10000, USERS, PARENT_CONTROL_MASK, false, "DELETE alone on a drive root"),
            (0, 0, READ_EXEC, USERS, PARENT_NONROOT_MASK, false, "read and execute on a non-root parent"),
            (0, 0, GENERIC_WRITE_BIT, AU, ENTRY_WRITE_MASK, true, "GENERIC_WRITE on an entry"),
            (0, 0, 0x10000, AU, ENTRY_WRITE_MASK, true, "DELETE on an entry"),
            (0, 0, MODIFY | 0x40, AU, PARENT_CONTROL_MASK, true, "Modify plus FILE_DELETE_CHILD"),
            (0, 0, 0x40000, USERS, PARENT_CONTROL_MASK, true, "WRITE_DAC"),
            (0, 0, 0x80000, USERS, PARENT_CONTROL_MASK, true, "WRITE_OWNER"),
            (0, 0, GENERIC_ALL_BIT, AU, PARENT_CONTROL_MASK, true, "GENERIC_ALL"),
            (0, 0, READ_EXEC, USERS, PARENT_CONTROL_MASK, false, "read and execute"),
            (0, INHERIT_ONLY, 0x1f01ff, USERS, PARENT_CONTROL_MASK, false, "inherit-only never applies to the folder itself"),
            (0, 0, 0x1f01ff, SYSTEM, PARENT_CONTROL_MASK, false, "SYSTEM"),
            (0, 0, 0x1f01ff, BA, PARENT_CONTROL_MASK, false, "Administrators"),
            (0, 0, 0x1f01ff, TRUSTED_INSTALLER_SID, PARENT_CONTROL_MASK, false, "TrustedInstaller"),
            (0, 0, MODIFY, AU, ENTRY_WRITE_MASK, true, "Modify is write on an entry"),
            (0, 0, READ_EXEC, USERS, ENTRY_WRITE_MASK, false, "read and execute is no write"),
            (0, 0, 0x4, AU, ENTRY_WRITE_MASK, true, "FILE_ADD_SUBDIRECTORY is write on an entry"),
            (1, 0, MODIFY, AU, ENTRY_WRITE_MASK, false, "a deny ACE grants nothing"),
            (9, 0, 0, AU, ENTRY_WRITE_MASK, true, "an allow-type ACE this check cannot parse fails closed"),
            (200, 0, 0, AU, ENTRY_WRITE_MASK, true, "an unknown ACE type fails closed"),
        ];
        for (ty, flags, ace_mask, sid, mask, want, why) in rows {
            let got = ace_grants_untrusted(ty, flags, ace_mask, mask, || Ok(sid.to_string())).unwrap();
            assert_eq!(got, want, "{why}");
        }
    }

    #[test]
    fn the_sid_is_read_only_when_the_mask_matches() {
        let got = ace_grants_untrusted(0, 0, 0x1200a9, PARENT_CONTROL_MASK, || panic!("the SID must not be read"));
        assert!(!got.unwrap());
    }

    #[test]
    fn a_folder_directly_under_a_drive_passes_even_when_the_root_grants_modify() {
        // D:\ grants Authenticated Users Modify (DELETE included); a drive root cannot be renamed.
        let mut fs = FakeFs::default().with(r"D:\", dir(true)).with(r"D:\canvas-rgb", dir(true));
        for (rel, is_dir) in [("installer", true), ("installer\\setup-user.ps1", false), ("installer\\lib.ps1", false), ("canvas-rgb.exe", false), ("canvas-rgb-svc.exe", false)] {
            let path = format!(r"D:\canvas-rgb\{rel}");
            fs.nodes.insert(PathBuf::from(path), if is_dir { dir(true) } else { file(true) });
        }
        fs.nonadmin_delete_on.push(PathBuf::from(r"D:\"));
        check_install_folder(&fs, Path::new(r"D:\canvas-rgb")).unwrap();
        // The same grant on a non-root parent is refused.
        let mut under = FakeFs::default().with(r"D:\", dir(true)).with(r"D:\Apps", dir(true)).with(r"D:\Apps\canvas-rgb", dir(true));
        under.nonadmin_delete_on.push(PathBuf::from(r"D:\Apps"));
        let err = check_install_folder(&under, Path::new(r"D:\Apps\canvas-rgb")).unwrap_err().to_string();
        assert!(err.contains(r"D:\Apps") && err.contains("rename it"), "{err}");
    }
}
