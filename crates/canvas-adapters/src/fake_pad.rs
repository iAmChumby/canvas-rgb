//! A fake 8BitDo Ultimate 2 and its vendor channel, replaying a stored profile.
//!
//! The fake is the pad's own seam (`Opener`, `VendorTransport`) and not another device's: the
//! ring probe and the ring adapter run against it in tests, and the daemon's fakes mode can list
//! it. It answers a profile read with the replies of a profile fixture, one per chunk, and logs
//! **every** report it is sent, so a test can fail on any report that is not a read request.
//!
//! The fixture is a made-up profile ([`ring_fixture_image`]): it has never been on a pad. The
//! file `tests/fixtures/ring_profile.txt` holds the replies the fake replays, in the text format
//! the ring probe saves (`ProfileCapture::to_text`), so a capture from the real pad drops in
//! with no code change.
//!
//! Nothing here touches hardware.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::eightbitdo_ultimate2::codec;
use crate::eightbitdo_ultimate2::probe::{self, ProfileCapture};
use crate::eightbitdo_ultimate2::{
    InputTransport, OpenedCollections, Opener, Personality, VendorTransport,
};

/// The made-up profile the fixture holds.
///
/// It starts from the codec's synthetic image (profile 3 active, profiles 1 and 3 in use) and adds
/// ring records: profile 3's custom theme is marked in use and holds 24 colours, stored as the
/// bytes `B, G, R, 0` (the research's `u32 0x00RRGGBB`, little-endian) so the probe's decode has
/// something to find. Ring 1 (indices 0-11) runs red, green, blue, then yellow, cyan, magenta,
/// white and four greys; ring 2 (12-23) repeats it reversed. The order is the fixture's own: it
/// is not a claim about the pad's LED order.
pub fn ring_fixture_image() -> Vec<u8> {
    let mut img = codec::synthetic_image();
    let at = codec::SINGLE + 0x64 * 2;
    img[at..at + 4].copy_from_slice(&codec::ENABLED.to_le_bytes());
    let ring: [(u8, u8, u8); 12] = [
        (255, 0, 0),
        (0, 255, 0),
        (0, 0, 255),
        (255, 255, 0),
        (0, 255, 255),
        (255, 0, 255),
        (255, 255, 255),
        (192, 192, 192),
        (128, 128, 128),
        (64, 64, 64),
        (32, 32, 32),
        (16, 16, 16),
    ];
    for (i, (r, g, b)) in ring.iter().chain(ring.iter().rev()).enumerate() {
        let s = at + 4 + 4 * i;
        img[s..s + 4].copy_from_slice(&[*b, *g, *r, 0]);
    }
    img
}

/// The replies a pad would give to a full profile read of `image`: one per chunk.
pub fn capture_of(image: &[u8], personality: Personality) -> ProfileCapture {
    let mut replies = Vec::new();
    let mut offset = 0;
    while offset < image.len() {
        let len = codec::CHUNK.min(image.len() - offset);
        replies.push(codec::reply_frame(len, true, offset as u32, &image[offset..offset + len]));
        offset += len;
    }
    ProfileCapture {
        personality: Some(personality.label().to_string()),
        replies,
        image: image.to_vec(),
        requests_sent: probe::CHUNKS,
    }
}

/// True when `data` asks the pad for a chunk (request word 2 of a class-04 config report).
///
/// The fake decides this on its own, from the header bytes, and does not use the probe's guard
/// ([`probe::is_read_request`]): a test that counts what the pad heard must not share a bug with
/// the code that is supposed to stop the report.
fn is_read_word(data: &[u8]) -> bool {
    data.len() >= codec::DATA_AT
        && data[0] == codec::OUT_ID
        && data[1] == 0x04
        && u16::from_le_bytes([data[2], data[3]]) == codec::REQ_READ
}

/// The text of the checked-in fixture file.
const FIXTURE_TEXT: &str = include_str!("../tests/fixtures/ring_profile.txt");

/// The checked-in fixture, read the way a saved probe file is read.
pub fn ring_fixture_capture() -> ProfileCapture {
    ProfileCapture::from_text(FIXTURE_TEXT).expect("the checked-in ring fixture is valid")
}

#[derive(Default)]
struct State {
    /// The reply to the read request for each chunk offset.
    replies: BTreeMap<usize, Vec<u8>>,
    /// Every report the pad was sent, in order.
    sent: Vec<Vec<u8>>,
    /// The replies ready to be read.
    queue: VecDeque<Vec<u8>>,
    /// How many read requests have been answered so far.
    answered: usize,
    /// Stop answering after this many read requests.
    silent_after: Option<usize>,
    /// Make `open_vendor` fail with this message.
    open_error: Option<String>,
}

/// A fake pad: an [`Opener`] whose vendor channel replays a profile.
#[derive(Clone)]
pub struct FakePad {
    state: Arc<Mutex<State>>,
    personality: Personality,
}

impl FakePad {
    /// A pad that answers read requests from the replies of `capture`.
    pub fn from_capture(capture: &ProfileCapture, personality: Personality) -> Self {
        let mut replies = BTreeMap::new();
        for (n, reply) in capture.replies.iter().enumerate() {
            replies.insert(n * codec::CHUNK, reply.clone());
        }
        Self {
            state: Arc::new(Mutex::new(State { replies, ..Default::default() })),
            personality,
        }
    }

    /// A pad holding `image`.
    pub fn with_image(image: Vec<u8>, personality: Personality) -> Self {
        Self::from_capture(&capture_of(&image, personality), personality)
    }

    /// The vendor channel of this pad. Every handle shares the one log.
    pub fn vendor(&self) -> FakePadVendor {
        FakePadVendor { state: self.state.clone() }
    }

    /// Every report the pad was sent, in order.
    pub fn sent_reports(&self) -> Vec<Vec<u8>> {
        self.lock().sent.clone()
    }

    /// How many of the reports the pad was sent are not read requests.
    pub fn non_read_reports(&self) -> usize {
        self.lock().sent.iter().filter(|r| !is_read_word(r)).count()
    }

    /// The pad stops answering after `n` read requests.
    pub fn go_silent_after(&self, n: usize) {
        self.lock().silent_after = Some(n);
    }

    /// Opening the vendor channel fails with `message`, as it does when another program holds it.
    pub fn refuse_open(&self, message: &str) {
        self.lock().open_error = Some(message.to_string());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The fake's vendor channel.
pub struct FakePadVendor {
    state: Arc<Mutex<State>>,
}

impl InputTransport for FakePadVendor {
    fn read_input(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(reply) = s.queue.pop_front() else {
            return Ok(0);
        };
        let n = reply.len().min(buf.len());
        buf[..n].copy_from_slice(&reply[..n]);
        Ok(n)
    }
}

impl VendorTransport for FakePadVendor {
    fn send_report(&mut self, data: &[u8]) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.sent.push(data.to_vec());
        if !is_read_word(data) {
            return Ok(());
        }
        if s.silent_after.is_some_and(|n| s.answered >= n) {
            return Ok(());
        }
        s.answered += 1;
        let offset = u32::from_le_bytes([data[14], data[15], data[16], data[17]]) as usize;
        if let Some(reply) = s.replies.get(&offset).cloned() {
            s.queue.push_back(reply);
        }
        Ok(())
    }
}

impl Opener for FakePad {
    fn personalities(&self) -> Vec<Personality> {
        vec![self.personality]
    }

    fn open_vendor(
        &self,
        personality: Personality,
    ) -> Result<Option<Box<dyn VendorTransport>>, String> {
        if let Some(why) = self.lock().open_error.clone() {
            return Err(why);
        }
        if personality.vendor_usage_page().is_none() {
            return Ok(None);
        }
        Ok(Some(Box::new(self.vendor())))
    }

    fn open_gamepad(
        &self,
        _personality: Personality,
    ) -> Result<Option<Box<dyn InputTransport>>, String> {
        Ok(None)
    }

    fn open_collections(&self) -> Result<OpenedCollections, String> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run once with `CANVAS_WRITE_FIXTURE=1` to rewrite the fixture file from the generator.
    #[test]
    fn the_fixture_file_is_what_the_generator_makes() {
        let text = capture_of(&ring_fixture_image(), Personality::XInput).to_text();
        if std::env::var_os("CANVAS_WRITE_FIXTURE").is_some() {
            std::fs::write(
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ring_profile.txt"),
                &text,
            )
            .unwrap();
            return;
        }
        assert_eq!(FIXTURE_TEXT.replace("\r\n", "\n"), text, "run once with CANVAS_WRITE_FIXTURE=1");
    }

    #[test]
    fn the_fake_replays_a_chunk_by_the_offset_asked_for() {
        let pad = FakePad::with_image(ring_fixture_image(), Personality::XInput);
        let mut v = pad.vendor();
        v.send_report(&codec::read_request(45).unwrap()).unwrap();
        let mut buf = [0u8; 64];
        assert_eq!(v.read_input(&mut buf, 1).unwrap(), 64);
        let data = codec::parse_read_reply(&buf, 45, 45).unwrap();
        assert_eq!(data, &ring_fixture_image()[45..90]);
        // Nothing queued: nothing read.
        assert_eq!(v.read_input(&mut buf, 1).unwrap(), 0);
    }
}
