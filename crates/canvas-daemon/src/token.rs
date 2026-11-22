//! G9: the API token.
//!
//! A 256-bit random token lives at `%APPDATA%\canvas-rgb\api-token` as 64 lowercase hex
//! characters, readable only by the current user. The daemon compares it in constant time.
//!
//! The DACL is checked on every load, not only when the file is first made (#542). A token file
//! that was copied in, restored from a backup or made by an older build can carry a wider ACL,
//! and whoever its ACL allows could drive the lights and the fans. When the DACL is not the
//! locked-down one it is put back before the token is used. The token itself is never changed,
//! and the file is never deleted, renamed or replaced.

use rand::rngs::SysRng;
use rand::TryCryptoRng;
use std::path::Path;

/// How many random bytes the token holds.
pub const TOKEN_BYTES: usize = canvas_proto::consts::TOKEN_BYTES;

/// SYSTEM's SID: the second principal the token file's DACL names.
pub const SYSTEM_SID: &str = "S-1-5-18";

/// `ACCESS_ALLOWED_ACE_TYPE`: the only ACE type the locked-down DACL holds.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
/// `ACCESS_DENIED_ACE_TYPE`: the same layout as an allow ACE, so its SID can be read too.
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
/// `INHERIT_ONLY_ACE`: the ACE applies to children only, not to the file itself.
const INHERIT_ONLY_ACE_FLAG: u8 = 0x08;
/// `FILE_ALL_ACCESS`: every right on a file.
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
/// `GENERIC_ALL`: what an SDDL `FA` carries before it is mapped to a file's rights.
const GENERIC_ALL: u32 = 0x1000_0000;

/// Fills `bytes` from `rng`. The `TryCryptoRng` bound makes the compiler refuse a generator that
/// is not marked cryptographically secure, so the token can never come from a seeded test RNG.
fn fill_secret<R: TryCryptoRng>(rng: &mut R, bytes: &mut [u8]) -> Result<(), R::Error> {
    rng.try_fill_bytes(bytes)
}

/// The token as lowercase hex, drawn from the operating system's random source (`SysRng`, which
/// is `ProcessPrng` on Windows).
pub fn generate() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    fill_secret(&mut SysRng, &mut bytes).expect("OS random source must fill the API token");
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Reads the token file, creating it when missing.
///
/// The file's DACL is checked on every load of an existing token, not only when the file is
/// first made (#542): a token file that was copied in, restored from a backup or made by an
/// older build can carry a wider ACL. When it does, the locked-down DACL is put back before the
/// token is handed out, and the token value is left exactly as it was.
pub fn load_or_create(path: &Path) -> std::io::Result<String> {
    if let Some(bytes) = crate::paths::read_optional(path) {
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        if !text.is_empty() {
            lock_down_existing(path)?;
            return Ok(text);
        }
    }
    let token = generate();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::storage::write_atomic(path, token.as_bytes())?;
    restrict_to_current_user(path)?;
    Ok(token)
}

/// Puts the locked-down DACL back on a token file that is there already.
///
/// The token value is never touched, and the file is never deleted, renamed or replaced. When
/// the DACL cannot be read, or cannot be put back, the error names the path: the daemon refuses
/// to start rather than hand out a token that someone else can read.
fn lock_down_existing(path: &Path) -> std::io::Result<()> {
    let user_sid = current_user_sid()?;
    let dacl = read_dacl(path)?;
    if dacl.is_locked_down(&user_sid) {
        return Ok(());
    }
    tracing::info!(
        path = %path.display(),
        protected = dacl.protected,
        principals = ?dacl.principal_names(),
        "the API token file's permissions were not locked down to this user and SYSTEM; they are locked down now"
    );
    restrict_to_current_user(path).map_err(|e| {
        std::io::Error::other(format!(
            "the API token file {} could not be locked down to this user and SYSTEM ({e}); \
             fix its permissions by hand, or remove the file so a new one is made. \
             The token was not changed.",
            path.display()
        ))
    })?;
    // Read it back: a DACL that was not applied must not pass for a fix.
    let dacl = read_dacl(path)?;
    if dacl.is_locked_down(&user_sid) {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "the API token file {} still has wider permissions after they were locked down; \
             fix its permissions by hand, or remove the file so a new one is made. \
             The token was not changed.",
            path.display()
        )))
    }
}

/// One access control entry of a DACL, as much of it as the check needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AceInfo {
    /// The ACE type. Only a plain allow ACE matches the locked-down DACL.
    ace_type: u8,
    /// The ACE flags. An inherit-only ACE does not apply to the file itself.
    flags: u8,
    /// The rights the ACE grants.
    mask: u32,
    /// The principal the ACE names, when the ACE's layout carries a SID.
    sid: Option<String>,
}

/// What a file's DACL holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Dacl {
    /// True when the DACL is protected from inheritance: it holds no inherited ACE.
    protected: bool,
    /// The ACEs, in the order the DACL has them.
    aces: Vec<AceInfo>,
}

impl Dacl {
    /// True when this is exactly the DACL the token file must carry: protected, and nothing
    /// but two plain allow ACEs with full control, one for the calling user and one for SYSTEM.
    fn is_locked_down(&self, user_sid: &str) -> bool {
        if !self.protected || self.aces.len() != 2 {
            return false;
        }
        let mut user = false;
        let mut system = false;
        for ace in &self.aces {
            if ace.ace_type != ACCESS_ALLOWED_ACE_TYPE
                || ace.flags & INHERIT_ONLY_ACE_FLAG != 0
                || !grants_full_control(ace.mask)
            {
                return false;
            }
            match ace.sid.as_deref() {
                Some(sid) if sid == user_sid => user = true,
                Some(sid) if sid == SYSTEM_SID => system = true,
                _ => return false,
            }
        }
        user && system
    }

    /// The principals the DACL names, for the log.
    fn principal_names(&self) -> Vec<&str> {
        self.aces.iter().filter_map(|ace| ace.sid.as_deref()).collect()
    }
}

/// True when the ACE's mask grants every right on the file.
fn grants_full_control(mask: u32) -> bool {
    mask & GENERIC_ALL != 0 || mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS
}

/// Compares two tokens in constant time. Length differences are not secret.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The `Authorization: Bearer <token>` header value, or `None`.
pub fn bearer(header: &str) -> Option<&str> {
    let rest = header.strip_prefix("Bearer ").or_else(|| header.strip_prefix("bearer "))?;
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// The SID of the user the daemon runs as, as an SDDL SID string (`S-1-5-21-...`).
#[cfg(windows)]
fn current_user_sid() -> std::io::Result<String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenUser, PSID, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE::default();
    // SAFETY: the out-pointer is a valid local; the handle is closed once, below.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut needed: u32 = 0;
    // SAFETY: a null buffer asks for the size the call needs.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    if needed == 0 {
        // SAFETY: the token was opened above and is closed exactly once.
        let _ = unsafe { CloseHandle(token) };
        return Err(std::io::Error::other("the process token has no user SID"));
    }
    let mut buffer = vec![0u8; needed as usize];
    let mut returned: u32 = 0;
    // SAFETY: the buffer holds `needed` bytes, which is what the call asked for.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<core::ffi::c_void>()),
            needed,
            &mut returned,
        )
    };
    // SAFETY: the token was opened above and is closed exactly once.
    let _ = unsafe { CloseHandle(token) };
    read.map_err(|e| std::io::Error::other(e.to_string()))?;

    let token_user = buffer.as_ptr() as *const TOKEN_USER;
    // SAFETY: the buffer holds a TOKEN_USER that was just read.
    let sid: PSID = unsafe { (*token_user).User.Sid };
    sid_to_string(sid).map_err(|e| std::io::Error::other(format!("could not read the user SID: {e}")))
}

/// The SID's string form (`S-1-5-21-...`).
#[cfg(windows)]
fn sid_to_string(sid: windows::Win32::Security::PSID) -> std::io::Result<String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;

    let mut text = windows::core::PWSTR::null();
    // SAFETY: the SID is a valid SID; the out-pointer is a valid local.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }
        .map_err(|e| std::io::Error::other(format!("could not read a SID: {e}")))?;
    // SAFETY: the string is NUL-terminated and was just made by the call above.
    let s = unsafe { text.to_string() }.unwrap_or_default();
    // SAFETY: the string came from ConvertSidToStringSidW and is freed once.
    unsafe {
        LocalFree(Some(HLOCAL(text.0.cast())));
    }
    Ok(s)
}

/// The SID of the user the daemon runs as. Windows is the only supported target; elsewhere the
/// file mode is the whole protection, so the owner stands in for the user's SID.
#[cfg(not(windows))]
fn current_user_sid() -> std::io::Result<String> {
    Ok("owner".to_string())
}

/// Restricts the file to the current user only (plus SYSTEM), read and write.
///
/// The daemon runs as the logged-on user, non-elevated. The DACL names the process token's user, so the
/// file is readable by the daemon's user and by SYSTEM (which the installer uses) and by nobody else.
#[cfg(windows)]
pub fn restrict_to_current_user(path: &Path) -> std::io::Result<()> {
    let sid = current_user_sid()?;
    // Protected DACL: only this user and SYSTEM, full control.
    apply_dacl(path, &format!("D:P(A;;FA;;;{sid})(A;;FA;;;{SYSTEM_SID})"))
}

#[cfg(not(windows))]
pub fn restrict_to_current_user(path: &Path) -> std::io::Result<()> {
    // On other platforms the file mode is enough; Windows is the only supported target.
    apply_dacl(path, "")
}

/// Puts the DACL `sddl` describes on `path`, protected from inheritance.
///
/// The owner is not changed: the file stays the user's own.
#[cfg(windows)]
fn apply_dacl(path: &Path, sddl: &str) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };

    let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the SDDL is NUL-terminated; the out-pointer is a valid local.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| std::io::Error::other(format!("could not build the token file DACL: {e}")))?;

    let result = (|| -> std::io::Result<()> {
        let mut present = windows::core::BOOL::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut defaulted = windows::core::BOOL::default();
        // SAFETY: the descriptor is alive until it is freed below; the out-pointers are locals.
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            .map_err(|e| std::io::Error::other(format!("could not read the DACL: {e}")))?;
        if !present.as_bool() || dacl.is_null() {
            return Err(std::io::Error::other("the security descriptor carries no DACL"));
        }
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // Only the DACL is set, and it is protected: it inherits nothing from the folder it is
        // in. The owner and the group are left as they are.
        // SAFETY: the path is NUL-terminated; the DACL points into the descriptor, alive here.
        let status = unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(path_wide.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(dacl.cast_const()),
                None,
            )
        };
        if status.0 != 0 {
            return Err(std::io::Error::from_raw_os_error(status.0 as i32));
        }
        Ok(())
    })();

    // SAFETY: the descriptor came from the conversion above and is freed once.
    unsafe {
        LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

#[cfg(not(windows))]
fn apply_dacl(path: &Path, _sddl: &str) -> std::io::Result<()> {
    // On other platforms the file mode is enough; Windows is the only supported target.
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
}

/// Reads a file's DACL through the real Win32 path.
#[cfg(windows)]
fn read_dacl(path: &Path) -> std::io::Result<Dacl> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

    let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the path is NUL-terminated; the out-pointers are valid locals.
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status.0 != 0 {
        return Err(std::io::Error::from_raw_os_error(status.0 as i32));
    }
    let result = dacl_from_descriptor(descriptor);
    // SAFETY: the descriptor came from GetNamedSecurityInfoW and is freed once.
    unsafe {
        LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

/// Reads a file's DACL. A Unix file mode has no ACEs: the owner and SYSTEM stand in for the two
/// principals the Windows DACL names.
#[cfg(not(windows))]
fn read_dacl(path: &Path) -> std::io::Result<Dacl> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path)?.permissions().mode();
    let mask = if mode & 0o077 == 0 { FILE_ALL_ACCESS } else { 0o444 };
    Ok(Dacl {
        protected: true,
        aces: vec![
            AceInfo {
                ace_type: ACCESS_ALLOWED_ACE_TYPE,
                flags: 0,
                mask,
                sid: Some(current_user_sid()?),
            },
            AceInfo {
                ace_type: ACCESS_ALLOWED_ACE_TYPE,
                flags: 0,
                mask,
                sid: Some(SYSTEM_SID.to_string()),
            },
        ],
    })
}

/// The ACEs of a security descriptor's DACL, read one by one.
#[cfg(windows)]
fn dacl_from_descriptor(
    descriptor: windows::Win32::Security::PSECURITY_DESCRIPTOR,
) -> std::io::Result<Dacl> {
    use windows::Win32::Security::{
        GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, ACCESS_ALLOWED_ACE, ACE_HEADER,
        ACL, SE_DACL_PROTECTED,
    };

    let mut control: u16 = 0;
    let mut revision: u32 = 0;
    // SAFETY: the descriptor came from GetNamedSecurityInfoW and is alive here.
    unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
        .map_err(|e| std::io::Error::other(format!("could not read the DACL's control bits: {e}")))?;
    let protected = control & SE_DACL_PROTECTED.0 != 0;

    let mut present = windows::core::BOOL::default();
    let mut defaulted = windows::core::BOOL::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: the descriptor is alive here; the out-pointers are valid locals.
    unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        .map_err(|e| std::io::Error::other(format!("could not read the DACL: {e}")))?;
    if !present.as_bool() || dacl.is_null() {
        // No DACL at all grants everyone everything, so it is never the locked-down one.
        return Ok(Dacl { protected, aces: Vec::new() });
    }

    let mut aces = Vec::new();
    let count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(count) {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: the DACL is alive here; the out-pointer is a valid local.
        unsafe { GetAce(dacl, index, &mut ace) }
            .map_err(|e| std::io::Error::other(format!("could not read ACE {index} of the DACL: {e}")))?;
        let header = ace as *const ACE_HEADER;
        // SAFETY: GetAce returned a valid ACE, so its header is readable.
        let (ace_type, flags) = unsafe { ((*header).AceType, (*header).AceFlags) };
        // A plain allow or deny ACE carries its mask and its SID in the same layout. The other
        // types (callback, object, audit) do not, so their SID is left unread.
        let (mask, sid) = if ace_type == ACCESS_ALLOWED_ACE_TYPE || ace_type == ACCESS_DENIED_ACE_TYPE {
            let allowed = ace as *const ACCESS_ALLOWED_ACE;
            // SAFETY: the ACE is a plain allow or deny ACE, so this is the right layout.
            let mask = unsafe { (*allowed).Mask };
            // SAFETY: the SID starts at SidStart and lives inside the ACE.
            let sid_ptr = windows::Win32::Security::PSID(
                unsafe { std::ptr::addr_of!((*allowed).SidStart) }.cast_mut().cast::<core::ffi::c_void>(),
            );
            (mask, Some(sid_to_string(sid_ptr)?))
        } else {
            (0, None)
        };
        aces.push(AceInfo { ace_type, flags, mask, sid });
    }
    Ok(Dacl { protected, aces })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The token the tests write: 64 hex characters, so it looks like a real one.
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A temp directory that is removed when it is dropped. The token file in it may carry the
    /// locked-down DACL, which its owner can always change back.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("canvas-rgb-token-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the temp dir is made");
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn generated_token_is_64_hex_chars() {
        let token = generate();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn generated_tokens_differ() {
        assert_ne!(generate(), generate());
    }

    #[test]
    fn constant_time_eq_matches_equality() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn bearer_parses_the_header() {
        assert_eq!(bearer("Bearer abc"), Some("abc"));
        assert_eq!(bearer("bearer abc"), Some("abc"));
        assert_eq!(bearer("Basic abc"), None);
        assert_eq!(bearer("Bearer   "), None);
        assert_eq!(bearer(""), None);
    }

    /// A token file with a wider ACL than the daemon's own: BUILTIN\Users may read it too.
    #[cfg(windows)]
    fn widen(path: &Path) {
        let user = current_user_sid().expect("the caller's own SID");
        let sddl = format!("D:P(A;;FA;;;{user})(A;;FA;;;{SYSTEM_SID})(A;;FR;;;BU)");
        apply_dacl(path, &sddl).expect("the wider DACL is applied");
    }

    /// #542: a token file that is already there is checked on every load, and a wider ACL is
    /// put back to the locked-down one. The token itself is not changed.
    #[cfg(windows)]
    #[test]
    fn a_token_file_with_a_wider_acl_is_locked_down_on_load() {
        let dir = TempDir::new("wider");
        let path = dir.path().join("api-token");
        std::fs::write(&path, TOKEN).expect("the token file is written");
        widen(&path);

        // The file starts wider than the daemon's own: this user, SYSTEM and BUILTIN\Users.
        let user_sid = current_user_sid().expect("the caller's own SID");
        let before = read_dacl(&path).expect("the DACL is read");
        assert!(
            !before.is_locked_down(&user_sid),
            "the file starts wider than the locked-down one: {:?}",
            before.principal_names()
        );
        assert!(
            before.principal_names().contains(&"S-1-5-32-545"),
            "BUILTIN\\Users may read the file to start with: {:?}",
            before.principal_names()
        );

        let token = load_or_create(&path).expect("the token is loaded");

        assert_eq!(token, TOKEN, "the token value is not changed");
        let after = read_dacl(&path).expect("the DACL is read");
        assert!(after.protected, "the DACL is protected from inheritance");
        let sids = after.principal_names();
        assert_eq!(sids.len(), 2, "only two principals are named: {sids:?}");
        assert!(sids.contains(&user_sid.as_str()), "this user is named: {sids:?}");
        assert!(sids.contains(&SYSTEM_SID), "SYSTEM is named: {sids:?}");
        assert!(after.is_locked_down(&user_sid), "the DACL is the locked-down one");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the token file is read"),
            TOKEN,
            "the file still holds the same token"
        );
    }

    /// A token file the daemon makes itself is locked down too.
    #[cfg(windows)]
    #[test]
    fn a_token_file_the_daemon_makes_is_locked_down() {
        let dir = TempDir::new("made");
        let path = dir.path().join("api-token");
        let token = load_or_create(&path).expect("the token is made");
        assert_eq!(token.len(), 64);
        let user_sid = current_user_sid().expect("the caller's own SID");
        let dacl = read_dacl(&path).expect("the DACL is read");
        assert!(dacl.protected, "the DACL is protected from inheritance");
        assert!(dacl.is_locked_down(&user_sid), "the DACL is the locked-down one");
    }

    /// A token file that is already locked down is left as it is, and the token is not changed.
    #[cfg(windows)]
    #[test]
    fn a_locked_down_token_file_is_left_alone() {
        let dir = TempDir::new("locked");
        let path = dir.path().join("api-token");
        std::fs::write(&path, TOKEN).expect("the token file is written");
        restrict_to_current_user(&path).expect("the DACL is applied");

        let token = load_or_create(&path).expect("the token is loaded");
        assert_eq!(token, TOKEN, "the token value is not changed");
        let user_sid = current_user_sid().expect("the caller's own SID");
        let dacl = read_dacl(&path).expect("the DACL is read");
        assert!(dacl.is_locked_down(&user_sid), "the DACL is the locked-down one");
    }
}
