//! The builder's live preview feed, after `app/web/src/lib/stackPreview.ts`.
//!
//! The daemon's second engine streams the stack's frames over
//! `WS /api/v1/preview?token=…&stack=1`: binary messages of `u16 LE width`,
//! `u16 LE height`, then RGB. The daemon opens one stack preview at a time
//! and refuses a second socket with close code 1013 and a reason; the feed
//! says so and asks again at a walking pace.
//!
//! React opens the socket from the browser. The GPUI editor has no WebSocket
//! client in its dependency list, so this is the same protocol spoken
//! directly over a loopback socket: the handshake, the frame reader, and the
//! reconnect rules, ported one for one.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// How long the feed waits before it asks again after a refused socket.
pub const REFUSED_RETRY: Duration = Duration::from_millis(5_000);
/// How long the feed waits before it asks again after any other close.
pub const CLOSED_RETRY: Duration = Duration::from_millis(1_200);
/// The close code the daemon uses when it refuses a second stack preview.
pub const CLOSE_REFUSED: u16 = 1013;
/// The reason the feed says when the daemon gives none.
pub const REFUSED_FALLBACK: &str = "another window is showing a stack preview; close it first";

/// One frame of the stack: `width` x `height` RGB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub width: u16,
    pub height: u16,
    pub rgb: Vec<u8>,
}

/// What the feed says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedEvent {
    /// A frame arrived.
    Frame(Box<StackFrame>),
    /// The daemon refused the socket and said why.
    Refused(String),
    /// The socket closed for another reason.
    Closed,
}

/// Reads a frame out of a binary message (`parseFrame`).
///
/// The message is `u16 LE width`, `u16 LE height`, then `width * height * 3`
/// RGB bytes. Anything else is not a frame.
pub fn parse_frame(buf: &[u8]) -> Option<StackFrame> {
    if buf.len() < 4 {
        return None;
    }
    let width = u16::from_le_bytes([buf[0], buf[1]]);
    let height = u16::from_le_bytes([buf[2], buf[3]]);
    let expected = width as usize * height as usize * 3;
    if buf.len() - 4 != expected {
        return None;
    }
    Some(StackFrame { width, height, rgb: buf[4..4 + expected].to_vec() })
}

/// The feed's own last word, so a builder that subscribes late still sees it.
#[derive(Debug, Clone, Default)]
pub struct FeedState {
    /// The last frame that arrived.
    pub last: Option<StackFrame>,
    /// The daemon's reason, when it refused the socket.
    pub problem: Option<String>,
}

/// A running feed. Dropping it stops the thread.
pub struct FeedHandle {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FeedHandle {
    /// Stops the feed and waits for its thread.
    pub fn stop(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FeedHandle {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts the feed: a thread that connects, reads frames and asks again when
/// the socket closes.
///
/// `token` is the API token the handshake carries; `port` is the daemon's
/// API port. The feed is silent until the daemon answers.
pub fn start_feed(port: u16, token: String, events: Sender<FeedEvent>) -> FeedHandle {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread = std::thread::spawn(move || {
        while !thread_stop.load(std::sync::atomic::Ordering::SeqCst) {
            let refused = match connect(port, &token, &events) {
                Ok(Close::Refused(reason)) => Some(reason),
                Ok(Close::Closed) => None,
                Err(_) => None,
            };
            if thread_stop.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            match refused {
                Some(reason) => {
                    if events.send(FeedEvent::Refused(reason)).is_err() {
                        return;
                    }
                    if sleep_until(&thread_stop, REFUSED_RETRY) {
                        return;
                    }
                }
                None => {
                    if events.send(FeedEvent::Closed).is_err() {
                        return;
                    }
                    if sleep_until(&thread_stop, CLOSED_RETRY) {
                        return;
                    }
                }
            }
        }
    });
    FeedHandle { stop, thread: Some(thread) }
}

/// Waits `wait`, or until the feed is stopped. True when it was stopped.
fn sleep_until(stop: &std::sync::Arc<std::sync::atomic::AtomicBool>, wait: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut left = wait;
    while left > Duration::ZERO {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(step.min(left));
        left -= step;
    }
    stop.load(std::sync::atomic::Ordering::SeqCst)
}

/// How the socket ended.
enum Close {
    /// The daemon refused it (close code 1013) with a reason.
    Refused(String),
    /// It closed for any other reason.
    Closed,
}

/// The WebSocket handshake and the frame reader.
fn connect(port: u16, token: &str, events: &Sender<FeedEvent>) -> Result<Close, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).map_err(|e| e.to_string())?;
    let key = base64(&random_bytes());
    let request = format!(
        "GET /api/v1/preview?token={}&stack=1 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        percent_encode(token),
        port,
        key
    );
    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    // The answer's head, up to the blank line.
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("the handshake closed before its answer".into());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        head.push_str(&line);
    }
    if !head.starts_with("HTTP/1.1 101") {
        return Err(format!("the handshake was refused: {}", head.lines().next().unwrap_or("")));
    }
    let mut stream = reader.into_inner();
    loop {
        match read_message(&mut stream)? {
            Some(Message::Binary(payload)) => {
                if let Some(frame) = parse_frame(&payload) {
                    // The channel is closed when the builder is gone.
                    if events.send(FeedEvent::Frame(Box::new(frame))).is_err() {
                        return Ok(Close::Closed);
                    }
                }
            }
            Some(Message::Close(code, reason)) => {
                return Ok(if code == CLOSE_REFUSED {
                    Close::Refused(if reason.is_empty() { REFUSED_FALLBACK.to_string() } else { reason })
                } else {
                    Close::Closed
                });
            }
            Some(Message::Other) => continue,
            Some(Message::Timeout) => continue,
            None => return Ok(Close::Closed),
        }
    }
}

enum Message {
    Binary(Vec<u8>),
    Close(u16, String),
    Other,
    /// Nothing arrived before the read deadline: the socket is still open.
    Timeout,
}

/// Reads one WebSocket message (a client never masks, so no mask is expected).
fn read_message(stream: &mut TcpStream) -> Result<Option<Message>, String> {
    let mut header = [0u8; 2];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
            return Ok(Some(Message::Timeout));
        }
        Err(_) => return Ok(None),
    }
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let mut len = (header[1] & 0x7f) as usize;
    if len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext).map_err(|e| e.to_string())?;
        len = u16::from_be_bytes(ext) as usize;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext).map_err(|e| e.to_string())?;
        len = u64::from_be_bytes(ext) as usize;
    }
    if len > 32 * 1024 * 1024 {
        return Err("the frame is too large".into());
    }
    let mut mask = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask).map_err(|e| e.to_string())?;
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).map_err(|e| e.to_string())?;
    if masked {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
    }
    Ok(Some(match opcode {
        0x2 => Message::Binary(payload),
        0x8 => {
            let code = if payload.len() >= 2 {
                u16::from_be_bytes([payload[0], payload[1]])
            } else {
                0
            };
            let reason = if payload.len() > 2 {
                String::from_utf8_lossy(&payload[2..]).into_owned()
            } else {
                String::new()
            };
            Message::Close(code, reason)
        }
        _ => Message::Other,
    }))
}

/// Sixteen bytes for the handshake's key.
fn random_bytes() -> [u8; 16] {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
        ^ (std::process::id() as u64) << 32;
    let mut out = [0u8; 16];
    for byte in out.iter_mut() {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed = seed.wrapping_mul(0x2545F4914F6CDD1D);
        *byte = (seed >> 32) as u8;
    }
    out
}

/// Base64, for the handshake's key.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

/// `encodeURIComponent` for the token in the query.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')') {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_a_width_a_height_and_the_pixels() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&[255, 0, 0, 0, 255, 0]);
        let frame = parse_frame(&buf).expect("a frame parses");
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 1);
        assert_eq!(frame.rgb, vec![255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn a_message_that_is_not_a_frame_is_dropped() {
        assert!(parse_frame(&[]).is_none());
        assert!(parse_frame(&[1, 0, 1]).is_none());
        // The body is one byte short.
        let mut buf = Vec::new();
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert!(parse_frame(&buf).is_none());
    }

    #[test]
    fn the_base64_of_the_key_is_the_standard_one() {
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn a_token_is_percent_encoded_in_the_query() {
        assert_eq!(percent_encode("abc123-_."), "abc123-_.");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("a+b/c"), "a%2Bb%2Fc");
    }

    #[test]
    fn the_retry_waits_are_the_daemons_own() {
        assert_eq!(REFUSED_RETRY, Duration::from_millis(5_000));
        assert_eq!(CLOSED_RETRY, Duration::from_millis(1_200));
        assert_eq!(CLOSE_REFUSED, 1013);
    }
}
