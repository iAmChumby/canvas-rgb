//! The live preview feed: the daemon's WebSocket, read in the background.
//!
//! The daemon sends binary messages: `u16 LE width, u16 LE height`, then
//! `width * height * 3` RGB bytes (`GET /api/v1/preview`, the same feed the
//! React editor reads in `lib/preview.ts`). The socket takes the API token as
//! `?token=`, because a WebSocket handshake cannot carry a header.
//!
//! The client here is a small one of its own: it speaks the RFC 6455
//! handshake and reads the frames it needs. It needs no crate for that, and
//! the dependency ledger stays as it is. The handshake's `Sec-WebSocket-Accept`
//! is not checked: the daemon is on the loopback, the token is in the query,
//! and the frames that arrive are checked by the daemon's own token check.
//!
//! Round 1 draws the desk at rest; the feed's error and liveness helpers are
//! for the preview states round 2 names, so this module allows dead code.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One frame of the feed: the picture, in RGB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    pub width: u16,
    pub height: u16,
    pub rgb: Vec<u8>,
}

impl PreviewFrame {
    /// The colour at a point of the picture, in picture pixels. `None` when
    /// the point is outside the frame.
    pub fn at(&self, x: i64, y: i64) -> Option<[u8; 3]> {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 3;
        Some([self.rgb[at], self.rgb[at + 1], self.rgb[at + 2]])
    }
}

/// The feed's state, shared between the reader thread and the page.
#[derive(Debug, Clone, Default)]
pub struct PreviewFeed {
    latest: Arc<Mutex<Option<PreviewFrame>>>,
    /// True once a frame has arrived: the page then draws the picture rather
    /// than the editor's own guess at it.
    live: Arc<Mutex<bool>>,
    /// How many frames have arrived: the page watches this number to know
    /// when to draw again.
    count: Arc<AtomicU64>,
    /// Why the socket is not there, in the daemon's own words.
    error: Arc<Mutex<Option<String>>>,
}

impl PreviewFeed {
    pub fn new() -> Self {
        Self::default()
    }

    /// The newest frame, if one has arrived.
    pub fn frame(&self) -> Option<PreviewFrame> {
        self.latest.lock().ok().and_then(|g| g.clone())
    }

    /// True when frames are arriving.
    pub fn is_live(&self) -> bool {
        self.live.lock().map(|g| *g).unwrap_or(false)
    }

    /// Why the feed is not there.
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|g| g.clone())
    }

    fn put(&self, frame: PreviewFrame) {
        if let Ok(mut g) = self.latest.lock() {
            *g = Some(frame);
        }
        if let Ok(mut g) = self.live.lock() {
            *g = true;
        }
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// How many frames have arrived. The page watches this number to know
    /// when the desk has to be drawn again.
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    fn fail(&self, message: String) {
        if let Ok(mut g) = self.error.lock() {
            *g = Some(message);
        }
    }
}

/// Starts the reader thread. It reconnects until the process ends, so a
/// daemon that starts later is still picked up.
///
/// `port` is the daemon's API port; `token` its API token.
pub fn spawn(port: u16, token: String, feed: PreviewFeed) {
    std::thread::spawn(move || {
        loop {
            match connect(port, &token) {
                Ok(mut stream) => {
                    let _ = stream.set_read_timeout(None);
                    if let Err(e) = read_frames(&mut stream, &feed) {
                        feed.fail(format!("the preview feed ended: {e}"));
                    }
                }
                Err(e) => feed.fail(format!("the preview feed could not start: {e}")),
            }
            // The daemon may be restarting: try again in a second.
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Opens the socket and says hello.
fn connect(port: u16, token: &str) -> std::io::Result<TcpStream> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_nodelay(true).ok();
    let key = base64(&random_key());
    let request = format!(
        "GET /api/v1/preview?token={token} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    // The answer's head: the status line and the headers, up to the blank line.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte)?;
        if n == 0 {
            return Err(std::io::Error::other("the daemon closed the socket"));
        }
        head.push(byte[0]);
        if head.len() >= 4 && &head[head.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if head.len() > 8192 {
            return Err(std::io::Error::other("the daemon's answer was too long"));
        }
    }
    let text = String::from_utf8_lossy(&head);
    if !text.starts_with("HTTP/1.1 101") {
        return Err(std::io::Error::other(format!(
            "the daemon refused the preview socket: {}",
            text.lines().next().unwrap_or("no status line")
        )));
    }
    Ok(stream)
}

/// Reads the frames until the socket ends. Binary messages are the picture.
fn read_frames(stream: &mut TcpStream, feed: &PreviewFeed) -> std::io::Result<()> {
    let mut pending: Vec<u8> = Vec::new();
    let mut message_opcode: u8 = 0;
    loop {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header)?;
        let fin = header[0] & 0x80 != 0;
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let short_len = header[1] & 0x7f;
        let mut len = short_len as u64;
        if short_len == 126 {
            let mut ext = [0u8; 2];
            stream.read_exact(&mut ext)?;
            // The WebSocket lengths are big-endian.
            len = u16::from_be_bytes(ext) as u64;
        } else if short_len == 127 {
            let mut ext = [0u8; 8];
            stream.read_exact(&mut ext)?;
            len = u64::from_be_bytes(ext);
        }
        if len > 64 * 1024 * 1024 {
            return Err(std::io::Error::other("the preview frame was too long"));
        }
        let mask = if masked {
            let mut m = [0u8; 4];
            stream.read_exact(&mut m)?;
            Some(m)
        } else {
            None
        };
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload)?;
        if let Some(m) = mask {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= m[i % 4];
            }
        }
        match opcode {
            // A continuation: the rest of the message the first frame began.
            0x0 => {
                pending.extend_from_slice(&payload);
                if fin {
                    let message = std::mem::take(&mut pending);
                    if message_opcode == 0x2 {
                        if let Some(frame) = parse_frame(&message) {
                            feed.put(frame);
                        }
                    }
                }
            }
            0x1 | 0x2 => {
                if fin {
                    if opcode == 0x2 {
                        if let Some(frame) = parse_frame(&payload) {
                            feed.put(frame);
                        }
                    }
                } else {
                    pending = payload;
                    message_opcode = opcode;
                }
            }
            // The socket is closing.
            0x8 => return Ok(()),
            // A ping: the answer is a pong, which this client does not need.
            0x9 => {
                let mut pong = vec![0x8a, payload.len().min(125) as u8];
                pong.extend_from_slice(&payload[..payload.len().min(125)]);
                stream.write_all(&pong).ok();
            }
            _ => {}
        }
    }
}

/// A binary message as a picture: `u16 LE width, u16 LE height`, then RGB.
fn parse_frame(payload: &[u8]) -> Option<PreviewFrame> {
    if payload.len() < 4 {
        return None;
    }
    let width = u16::from_le_bytes([payload[0], payload[1]]);
    let height = u16::from_le_bytes([payload[2], payload[3]]);
    let expected = width as usize * height as usize * 3;
    if payload.len() - 4 != expected {
        return None;
    }
    Some(PreviewFrame {
        width,
        height,
        rgb: payload[4..].to_vec(),
    })
}

/// Sixteen bytes for the handshake's key. The key only has to be different
/// from the last one; it is not a secret.
fn random_key() -> [u8; 16] {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15);
    seed ^= std::process::id() as u64;
    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut out = [0u8; 16];
    let mut state = seed | 1;
    for slot in out.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *slot = (state >> 24) as u8;
    }
    out
}

/// Base64 for the handshake's key.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_read_from_its_header_and_its_bytes() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&2u16.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let frame = parse_frame(&payload).expect("a frame");
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 1);
        assert_eq!(frame.rgb, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(frame.at(0, 0), Some([1, 2, 3]));
        assert_eq!(frame.at(1, 0), Some([4, 5, 6]));
        assert_eq!(frame.at(2, 0), None);
    }

    #[test]
    fn a_frame_that_is_not_the_size_it_says_is_dropped() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&4u16.to_le_bytes());
        payload.extend_from_slice(&4u16.to_le_bytes());
        payload.extend_from_slice(&[1, 2, 3]);
        assert!(parse_frame(&payload).is_none());
        assert!(parse_frame(&[1, 2]).is_none());
    }

    #[test]
    fn the_handshake_key_is_base64_of_sixteen_bytes() {
        let key = base64(&[0u8; 16]);
        assert_eq!(key.len(), 24);
        assert!(key.ends_with("=="));
        assert!(key.starts_with("AAAAAAAA"));
    }
}
