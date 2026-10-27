//! Who is listening on the daemon's port?
//!
//! The API token is a bearer secret: whoever holds it can drive the lights and the fans.
//! The clients send it to whatever answers on the daemon's port, so a local process that
//! binds 127.0.0.1:9430 before the daemon starts could collect the token and then talk to
//! the daemon's API with it.
//!
//! Before sending the token, a client checks who holds the port:
//!
//! 1. `GetExtendedTcpTable(AF_INET, TCP_TABLE_OWNER_PID_LISTENER)` gives the PID that owns
//!    the listening socket.
//! 2. `QueryFullProcessImageNameW` gives the image path; its file name must be
//!    `canvas-rgbd.exe`.
//! 3. `OpenProcessToken` + `GetTokenInformation(TokenUser)` give the user SID of that
//!    process; it must be the SID of the caller (`EqualSid`).
//!
//! The user check is the real guard: the image name can be faked by any process that is
//! allowed to call its executable `canvas-rgbd.exe`, but the SID cannot. Nothing here talks
//! to the network, so the check runs before the token leaves the process.
//!
//! Dev runs are fine: the daemon may run from `target\debug\` or `target\release\`, so any
//! path whose file name is `canvas-rgbd.exe` is accepted.

use std::ffi::c_void;
use std::path::Path;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Security::{
    EqualSid, GetLengthSid, GetTokenInformation, IsValidSid, TokenUser, PSID, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// The daemon's image file name. A listener with any other name is refused.
const DAEMON_EXE: &str = canvas_proto::consts::DAEMON_EXE;

/// The product name in the refusal message.
const PRODUCT: &str = canvas_proto::consts::APP_NAME;

/// The name used when the image path of the listener cannot be read.
pub const UNKNOWN_PROCESS: &str = "an unknown process";

/// `AF_INET`: the daemon's API is IPv4 loopback.
const AF_INET: u32 = 2;

/// `MIB_TCP_STATE_LISTEN`.
const TCP_STATE_LISTEN: u32 = 2;

/// A Windows security identifier: the bytes of the SID structure.
///
/// A SID has one fixed layout, so the bytes are the SID. They are compared with
/// [`EqualSid`](windows::Win32::Security::EqualSid), which is what the check is built on.
#[derive(Debug, Clone)]
pub struct Sid(Vec<u8>);

impl PartialEq for Sid {
    /// Two SIDs are the same when `EqualSid` says so.
    fn eq(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

impl Eq for Sid {}

impl Sid {
    /// The bytes of the SID structure.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// True when both SIDs are the same user.
    ///
    /// The two SID structures are put back into aligned buffers and handed to
    /// `EqualSid`, which compares them field by field.
    pub fn equals(&self, other: &Sid) -> bool {
        if !well_formed(&self.0) || !well_formed(&other.0) {
            // Not a SID structure: compare the bytes.
            return self.0 == other.0;
        }
        let left = aligned_words(&self.0);
        let right = aligned_words(&other.0);
        // SAFETY: both buffers hold a copy of a SID structure (a SID has no pointers).
        unsafe {
            EqualSid(
                PSID(left.as_ptr().cast_mut().cast()),
                PSID(right.as_ptr().cast_mut().cast()),
            )
            .is_ok()
        }
    }
}

impl std::fmt::Display for Sid {
    /// The SID in its text form, e.g. `S-1-5-21-1-2-3`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.len() < 8 {
            return write!(f, "<invalid sid>");
        }
        let revision = self.0[0];
        let count = self.0[1] as usize;
        let mut authority: u64 = 0;
        for byte in &self.0[2..8] {
            authority = (authority << 8) | u64::from(*byte);
        }
        write!(f, "S-{revision}-{authority}")?;
        for index in 0..count {
            let start = 8 + index * 4;
            let Some(chunk) = self.0.get(start..start + 4) else {
                break;
            };
            let value = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            write!(f, "-{value}")?;
        }
        Ok(())
    }
}

/// A copy of the SID bytes in a buffer that is aligned for the SID's `u32` members.
fn aligned_words(bytes: &[u8]) -> Vec<u32> {
    let mut words = vec![0u32; bytes.len().div_ceil(4)];
    if !bytes.is_empty() {
        // SAFETY: `words` holds `bytes.len()` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr().cast::<u8>(), bytes.len());
        }
    }
    words
}

/// True when the bytes can be a SID structure: 8 bytes plus 4 per sub authority.
fn well_formed(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    bytes.len() >= 8 + 4 * usize::from(bytes[1])
}

/// What we know about the process holding the port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    /// The PID that owns the listening socket.
    pub pid: u32,
    /// The port the socket listens on.
    pub port: u16,
    /// The file name of the process image, e.g. `canvas-rgbd.exe`. `None` when the path
    /// could not be read.
    pub image_name: Option<String>,
    /// The user the process runs as. `None` when the token could not be read.
    pub sid: Option<Sid>,
}

/// Why the listener is not trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Nothing listens on the port.
    NothingListening,
    /// The port is held by a process that is not our daemon, or by our daemon running as
    /// another user.
    HeldBy {
        /// The file name of the holder, or [`UNKNOWN_PROCESS`].
        name: String,
        /// The port that was checked.
        port: u16,
    },
}

impl Refusal {
    /// The message the clients show.
    pub fn message(&self) -> String {
        match self {
            Refusal::NothingListening => format!("{PRODUCT} daemon not running"),
            Refusal::HeldBy { name, port } => {
                format!("{PRODUCT} daemon not running (port {port} is held by {name})")
            }
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

/// The decision: is this listener our daemon, running as this user?
///
/// `listener` is `None` when nothing listens on the port.
pub fn trusted(listener: Option<PeerInfo>, me: &Sid) -> Result<(), Refusal> {
    let Some(peer) = listener else {
        return Err(Refusal::NothingListening);
    };
    let name = peer
        .image_name
        .clone()
        .unwrap_or_else(|| UNKNOWN_PROCESS.to_string());
    let is_daemon = peer
        .image_name
        .as_deref()
        .is_some_and(|image| image.eq_ignore_ascii_case(DAEMON_EXE));
    if !is_daemon {
        return Err(Refusal::HeldBy { name, port: peer.port });
    }
    // The image name can be faked; the user cannot.
    match peer.sid.as_ref() {
        Some(sid) if sid.equals(me) => Ok(()),
        _ => Err(Refusal::HeldBy { name, port: peer.port }),
    }
}

/// Checks that the listener on `port` is the daemon, running as this user.
pub fn check(port: u16) -> Result<(), Refusal> {
    let Some(pid) = listener_pid(port) else {
        return Err(Refusal::NothingListening);
    };
    let peer = PeerInfo {
        pid,
        port,
        image_name: image_name(pid),
        sid: process_sid(pid),
    };
    let Some(me) = caller_sid() else {
        // We cannot say who we are, so we cannot trust the holder either.
        return Err(Refusal::HeldBy {
            name: peer
                .image_name
                .clone()
                .unwrap_or_else(|| UNKNOWN_PROCESS.to_string()),
            port,
        });
    };
    trusted(Some(peer), &me)
}

/// Checks the listener of an API base URL, e.g. `http://127.0.0.1:9430`.
///
/// Only a loopback address is checked: the daemon's API is loopback-only, so any other
/// address is not the daemon and the check does not apply to it.
pub fn check_base(base: &str) -> Result<(), Refusal> {
    match loopback_port(base) {
        Some(port) => check(port),
        None => Ok(()),
    }
}

/// The port of a loopback API URL, or `None` when the URL is not on the loopback interface.
fn loopback_port(base: &str) -> Option<u16> {
    let rest = base
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (host, port) = authority.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if !host.eq_ignore_ascii_case("localhost") && host != "127.0.0.1" && host != "::1" {
        return None;
    }
    port.parse::<u16>().ok()
}

/// The PID that owns the listening socket on 127.0.0.1:`port`.
///
/// `None` when nothing listens there.
pub fn listener_pid(port: u16) -> Option<u32> {
    tcp_rows()?.into_iter().find_map(|row| {
        if row.dwState == TCP_STATE_LISTEN && row_port(row.dwLocalPort) == port && is_local(row.dwLocalAddr)
        {
            Some(row.dwOwningPid)
        } else {
            None
        }
    })
}

/// The file name of a process's image, e.g. `canvas-rgbd.exe`.
///
/// `None` when the path cannot be read (the process is gone, or it may not be opened).
pub fn image_name(pid: u32) -> Option<String> {
    // SAFETY: every call below is made with a handle this function owns.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let _handle = Handle::new(handle);
        let mut size: u32 = 1024;
        let mut buffer = vec![0u16; size as usize];
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
        .ok()?;
        let path = String::from_utf16_lossy(&buffer[..(size as usize).min(buffer.len())]);
        Some(Path::new(&path).file_name()?.to_string_lossy().into_owned())
    }
}

/// The user SID of the calling process.
pub fn caller_sid() -> Option<Sid> {
    // SAFETY: the pseudo-handle of the current process is never closed.
    unsafe { sid_of_process(GetCurrentProcess()) }
}

/// The user SID of another process.
pub fn process_sid(pid: u32) -> Option<Sid> {
    // SAFETY: the handle is closed by the guard.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let _handle = Handle::new(handle);
        sid_of_process(handle)
    }
}

/// The user SID of the owner of a process handle.
unsafe fn sid_of_process(process: HANDLE) -> Option<Sid> {
    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
    let _token = Handle::new(token);
    let mut needed: u32 = 0;
    // The first call asks how many bytes the TOKEN_USER needs.
    let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
    if needed == 0 {
        return None;
    }
    let mut buffer = vec![0u8; needed as usize];
    GetTokenInformation(
        token,
        TokenUser,
        Some(buffer.as_mut_ptr() as *mut c_void),
        needed,
        &mut needed,
    )
    .ok()?;
    let token_user = &*(buffer.as_ptr() as *const TOKEN_USER);
    sid_bytes(token_user.User.Sid)
}

/// The bytes of a SID structure.
unsafe fn sid_bytes(sid: PSID) -> Option<Sid> {
    if sid.0.is_null() || !IsValidSid(sid).as_bool() {
        return None;
    }
    let length = GetLengthSid(sid);
    if length == 0 {
        return None;
    }
    let mut bytes = vec![0u8; length as usize];
    std::ptr::copy_nonoverlapping(sid.0.cast::<u8>(), bytes.as_mut_ptr(), length as usize);
    Some(Sid(bytes))
}

/// Every listening row of the IPv4 TCP table.
fn tcp_rows() -> Option<Vec<MIB_TCPROW_OWNER_PID>> {
    // SAFETY: the table is read into a buffer of the size Windows asks for.
    unsafe {
        let mut size: u32 = 0;
        // The first call asks how big the table is.
        GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        let code = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut c_void),
            &mut size,
            false,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if code != 0 {
            // The table grew while we were reading it: try again next time.
            return None;
        }
        let count = u32::from_ne_bytes(buffer.get(0..4)?.try_into().ok()?);
        let row_size = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
        // The count comes from the kernel, but a table that changed under us could name
        // more rows than the buffer holds: the cap keeps the allocation sane.
        let rows_in_buffer = buffer.len().saturating_sub(4) / row_size;
        let mut rows = Vec::with_capacity((count as usize).min(rows_in_buffer));
        for chunk in buffer.get(4..).unwrap_or_default().chunks_exact(row_size) {
            let row: MIB_TCPROW_OWNER_PID =
                std::ptr::read_unaligned(chunk.as_ptr() as *const MIB_TCPROW_OWNER_PID);
            rows.push(row);
        }
        Some(rows)
    }
}

/// The port of a row: the low 16 bits of `dwLocalPort` are in network byte order.
fn row_port(dw_local_port: u32) -> u16 {
    ((dw_local_port & 0xFFFF) as u16).swap_bytes()
}

/// True when the address of a row is 127.0.0.1 or 0.0.0.0 (any address).
fn is_local(dw_local_addr: u32) -> bool {
    let bytes = dw_local_addr.to_ne_bytes();
    bytes == [127, 0, 0, 1] || bytes == [0, 0, 0, 0]
}

/// Closes a handle when it goes out of scope.
struct Handle(HANDLE);

impl Handle {
    fn new(handle: HANDLE) -> Self {
        Self(handle)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by this process and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SID for the tests: `S-1-5-<id>`. A real SID structure, so `EqualSid` accepts it.
    fn fake_sid(id: u32) -> Sid {
        let mut bytes = vec![1u8, 1, 0, 0, 0, 0, 0, 5];
        bytes.extend_from_slice(&id.to_le_bytes());
        Sid(bytes)
    }

    fn peer(image: Option<&str>, sid: Option<Sid>) -> PeerInfo {
        PeerInfo {
            pid: 4242,
            port: 9430,
            image_name: image.map(str::to_string),
            sid,
        }
    }

    #[test]
    fn nothing_listening_is_refused() {
        let refusal = trusted(None, &fake_sid(1)).unwrap_err();
        assert_eq!(refusal.message(), "canvas-rgb daemon not running");
    }

    #[test]
    fn another_program_is_refused_and_named() {
        let refusal = trusted(Some(peer(Some("evil.exe"), Some(fake_sid(1)))), &fake_sid(1)).unwrap_err();
        assert_eq!(
            refusal.message(),
            "canvas-rgb daemon not running (port 9430 is held by evil.exe)"
        );
    }

    #[test]
    fn an_unreadable_image_is_refused_as_an_unknown_process() {
        let refusal = trusted(Some(peer(None, Some(fake_sid(1)))), &fake_sid(1)).unwrap_err();
        assert_eq!(
            refusal.message(),
            "canvas-rgb daemon not running (port 9430 is held by an unknown process)"
        );
    }

    #[test]
    fn the_daemon_of_another_user_is_refused() {
        let refusal = trusted(
            Some(peer(Some("canvas-rgbd.exe"), Some(fake_sid(2)))),
            &fake_sid(1),
        )
        .unwrap_err();
        assert_eq!(
            refusal.message(),
            "canvas-rgb daemon not running (port 9430 is held by canvas-rgbd.exe)"
        );
    }

    #[test]
    fn a_daemon_with_an_unreadable_user_is_refused() {
        let refusal = trusted(Some(peer(Some("canvas-rgbd.exe"), None)), &fake_sid(1)).unwrap_err();
        assert_eq!(
            refusal.message(),
            "canvas-rgb daemon not running (port 9430 is held by canvas-rgbd.exe)"
        );
    }

    #[test]
    fn our_own_daemon_is_trusted() {
        assert!(trusted(
            Some(peer(Some("canvas-rgbd.exe"), Some(fake_sid(1)))),
            &fake_sid(1)
        )
        .is_ok());
    }

    #[test]
    fn the_image_name_is_matched_without_case() {
        assert!(trusted(
            Some(peer(Some("CANVAS-RGBD.EXE"), Some(fake_sid(1)))),
            &fake_sid(1)
        )
        .is_ok());
    }

    #[test]
    fn the_sid_of_the_caller_is_read() {
        let sid = caller_sid().expect("the caller's own SID");
        assert!(sid.as_bytes().len() >= 8);
        assert!(sid.to_string().starts_with("S-1-5-"));
    }

    #[test]
    fn the_table_names_this_process_as_the_owner_of_a_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the local address").port();
        assert_eq!(listener_pid(port), Some(std::process::id()));
    }

    #[test]
    fn a_port_nobody_listens_on_has_no_owner() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the local address").port();
        drop(listener);
        assert_eq!(listener_pid(port), None);
    }
}
