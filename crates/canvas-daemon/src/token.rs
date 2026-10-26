//! G9: the API token.
//!
//! A 256-bit random token lives at `%APPDATA%\canvas-rgb\api-token` as 64 lowercase hex
//! characters, readable only by the current user. The daemon compares it in constant time.

use rand::rngs::OsRng;
use rand::RngCore;
use std::path::Path;

/// How many random bytes the token holds.
pub const TOKEN_BYTES: usize = canvas_proto::consts::TOKEN_BYTES;

/// The token as lowercase hex.
pub fn generate() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Reads the token file, creating it when missing.
pub fn load_or_create(path: &Path) -> std::io::Result<String> {
    if let Some(bytes) = crate::paths::read_optional(path) {
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        if !text.is_empty() {
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
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, PSID, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let mut needed: u32 = 0;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            return Err(std::io::Error::other("the process token has no user SID"));
        }
        let mut buffer = vec![0u8; needed as usize];
        let mut returned: u32 = 0;
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<core::ffi::c_void>()),
            needed,
            &mut returned,
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;

        let token_user = buffer.as_ptr() as *const TOKEN_USER;
        let sid: PSID = (*token_user).User.Sid;

        let mut sid_text = windows::core::PWSTR::null();
        ConvertSidToStringSidW(sid, &mut sid_text)
            .map_err(|e| std::io::Error::other(format!("could not read the user SID: {e}")))?;
        let s = sid_text.to_string().unwrap_or_default();
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            sid_text.0.cast(),
        )));
        Ok(s)
    }
}

/// Restricts the file to the current user only (plus SYSTEM), read and write.
///
/// The daemon runs as Luke, non-elevated. The DACL names the process token's user, so the
/// file is readable by Luke and by SYSTEM (which the installer uses) and by nobody else.
#[cfg(windows)]
pub fn restrict_to_current_user(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    let sid = current_user_sid()?;
    // Protected DACL: only this user and SYSTEM, full control.
    let sddl = format!("D:P(A;;FA;;;{sid})(A;;FA;;;SY)");

    unsafe {
        let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|e| std::io::Error::other(format!("could not build the token file DACL: {e}")))?;

        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let ok = SetFileSecurityW(
            PCWSTR(path_wide.as_ptr()),
            DACL_SECURITY_INFORMATION,
            descriptor,
        );
        if !ok.as_bool() {
            return Err(std::io::Error::other("SetFileSecurityW failed on the token file"));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn restrict_to_current_user(path: &Path) -> std::io::Result<()> {
    // On other platforms the file mode is enough; Windows is the only supported target.
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
