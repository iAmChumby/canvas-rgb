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

/// The request word of a class-04 config report, in the no-length framing, when it is one.
fn config_word(data: &[u8]) -> Option<u16> {
    (data.len() >= 6 && data[0] == codec::OUT_ID && data[1] == 0x04)
        .then(|| u16::from_le_bytes([data[2], data[3]]))
}

/// The text of the checked-in fixture file.
const FIXTURE_TEXT: &str = include_str!("../tests/fixtures/ring_profile.txt");

/// The profile image the S2 probe read from the real pad (2026-11-18): profile 1 active, its
/// light-tracing, fire-ring and custom ring records marked in use, all 24 custom words
/// `ff 00 8b 00`. The S3 tool's tests run on it, so the layout they check is the pad's own.
const REAL_TEXT: &str = include_str!("../tests/fixtures/pad_profile_real.txt");

/// The real pad's profile, read the way a saved probe file is read.
pub fn real_profile_capture() -> ProfileCapture {
    ProfileCapture::from_text(REAL_TEXT).expect("the checked-in real profile is valid")
}

/// The checked-in fixture, read the way a saved probe file is read.
pub fn ring_fixture_capture() -> ProfileCapture {
    ProfileCapture::from_text(FIXTURE_TEXT).expect("the checked-in ring fixture is valid")
}

#[derive(Default)]
struct State {
    /// The image the pad holds. A write request changes it; a read after it shows the change.
    image: Vec<u8>,
    /// The reply to the read request for each chunk offset. Replies of a stored capture are
    /// replayed byte for byte until a write touches their chunk; then they are rebuilt from the
    /// image.
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
    /// The pad does not answer this many more commits.
    silent_commits: usize,
    /// The pad applies but does not acknowledge this many more write chunks.
    silent_write_acks: usize,
    /// The pad acknowledges write chunks and does not apply them.
    ignore_writes: bool,
    /// On the next commit the pad also changes this byte of the image (a pad that scribbles
    /// outside the block it was given).
    scribble_on_commit: Option<usize>,
    /// On the next commit the pad drops the write: the image goes back to what it was.
    drop_next_commit: bool,
    /// The image as it was before the writes since the last commit.
    before_write: Option<Vec<u8>>,
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
            state: Arc::new(Mutex::new(State {
                image: capture.image.clone(),
                replies,
                ..Default::default()
            })),
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

    /// The pad never answers a commit (the write chunks are acknowledged).
    pub fn silence_commits(&self) {
        self.lock().silent_commits = usize::MAX;
    }

    /// The pad does not answer the next `n` commits (the bytes are applied, as a pad that did the
    /// work and lost the reply would).
    pub fn silence_next_commits(&self, n: usize) {
        self.lock().silent_commits = n;
    }

    /// The pad applies the next `n` write chunks and does not acknowledge them.
    pub fn silence_next_write_acks(&self, n: usize) {
        self.lock().silent_write_acks = n;
    }

    /// The pad acknowledges every write chunk and every commit and keeps none of the bytes.
    pub fn ignore_writes(&self) {
        self.lock().ignore_writes = true;
    }

    /// The pad takes writes again.
    pub fn take_writes(&self) {
        self.lock().ignore_writes = false;
    }

    /// Overwrites the pad's image directly (a test setting a state, not a report).
    pub fn set_image(&self, image: Vec<u8>) {
        let mut s = self.lock();
        s.image = image;
        s.rebuild_replies(0, codec::IMAGE_LEN);
    }

    /// At the next commit the pad also changes the byte at `offset` of its image.
    pub fn scribble_on_next_commit(&self, offset: usize) {
        self.lock().scribble_on_commit = Some(offset);
    }

    /// The next commit is acknowledged but the pad drops what was written since the last one.
    pub fn drop_next_commit(&self) {
        self.lock().drop_next_commit = true;
    }

    /// The image the pad holds now.
    pub fn image(&self) -> Vec<u8> {
        self.lock().image.clone()
    }

    /// How many reports of each kind the pad was sent: `(read, write chunk, commit, reportstate)`.
    pub fn report_counts(&self) -> (usize, usize, usize, usize) {
        let s = self.lock();
        let mut counts = (0, 0, 0, 0);
        for r in &s.sent {
            if is_read_word(r) {
                counts.0 += 1;
            } else if let Some(word) = config_word(r) {
                match word {
                    codec::REQ_WRITE => counts.1 += 1,
                    codec::REQ_COMMIT => counts.2 += 1,
                    codec::REQ_REPORT_STATE => counts.3 += 1,
                    _ => {}
                }
            }
        }
        counts
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
        let Some(word) = config_word(data) else {
            return Ok(());
        };
        match word {
            codec::REQ_READ if is_read_word(data) => {
                if s.silent_after.is_some_and(|n| s.answered >= n) {
                    return Ok(());
                }
                s.answered += 1;
                let offset = u32::from_le_bytes([data[14], data[15], data[16], data[17]]) as usize;
                if let Some(reply) = s.replies.get(&offset).cloned() {
                    s.queue.push_back(reply);
                }
            }
            codec::REQ_WRITE if data.len() >= codec::DATA_AT => {
                // A write request carries bytes into the image at its offset; the fake is a small
                // model of the pad, so a re-read shows what was written once it is committed.
                let word = u32::from_le_bytes([data[6], data[7], data[8], data[9]]);
                let len = (word & 0xFFFF) as usize;
                let offset = u32::from_le_bytes([data[14], data[15], data[16], data[17]]) as usize;
                if !s.ignore_writes
                    && codec::DATA_AT + len <= data.len()
                    && offset + len <= s.image.len()
                {
                    if s.before_write.is_none() {
                        s.before_write = Some(s.image.clone());
                    }
                    s.image[offset..offset + len]
                        .copy_from_slice(&data[codec::DATA_AT..codec::DATA_AT + len]);
                    s.rebuild_replies(offset, len);
                }
                if s.silent_write_acks > 0 {
                    s.silent_write_acks -= 1;
                } else {
                    s.queue.push_back(codec::ack_frame(codec::REQ_WRITE));
                }
            }
            codec::REQ_COMMIT => {
                if s.drop_next_commit {
                    s.drop_next_commit = false;
                    if let Some(before) = s.before_write.take() {
                        s.image = before;
                        s.rebuild_replies(0, codec::IMAGE_LEN);
                    }
                }
                if let Some(at) = s.scribble_on_commit.take() {
                    if at < s.image.len() {
                        s.image[at] ^= 0xFF;
                        s.rebuild_replies(at, 1);
                    }
                }
                s.before_write = None;
                if s.silent_commits > 0 {
                    s.silent_commits = s.silent_commits.saturating_sub(1);
                } else {
                    s.queue.push_back(codec::ack_frame(codec::REQ_COMMIT));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl State {
    /// Rebuilds the stored reply of every chunk that overlaps `len` bytes at `offset`.
    fn rebuild_replies(&mut self, offset: usize, len: usize) {
        let mut at = 0;
        while at < self.image.len() {
            let chunk = codec::CHUNK.min(self.image.len() - at);
            if at < offset + len && offset < at + chunk {
                let reply = codec::reply_frame(chunk, true, at as u32, &self.image[at..at + chunk]);
                self.replies.insert(at, reply);
            }
            at += chunk;
        }
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
        personality: Personality,
    ) -> Result<Option<Box<dyn InputTransport>>, String> {
        // A silent gamepad collection where the real pad has one: over Bluetooth it is the one
        // collection there is, so the adapter connects through it.
        Ok(personality
            .reads_gamepad_collection()
            .then(|| Box::new(self.vendor()) as Box<dyn InputTransport>))
    }

    fn open_collections(&self) -> Result<OpenedCollections, String> {
        Ok(Vec::new())
    }
}

/// Gamepad reports of the real pad over Bluetooth LE (#566): 34 bytes each, report id `0x01`,
/// read from its HID-over-GATT collection on 2026-11-26 while Luke pressed each button in turn.
/// Bytes 15..26 are the motion sensors and move in every report; byte 14 is the battery (90 %,
/// then 83 %, not charging).
pub mod ble_capture {
    /// Hands off: the hat centred (`0x0f`), the sticks at `0x7f`, nothing pressed, 90 %.
    pub const REST: [u8; 34] = hex34("010f7f7f7f7f00000000000012605a6308dffece0d0f00faff180000000000000000");
    /// A pressed (byte 8, `0x01`), 83 %.
    pub const A: [u8; 34] = hex34("010f7f7f7f7f000001000000126053d8094e000b0d62003300220000000000000000");
    /// The left back paddle, PL (byte 8, `0x20`).
    pub const LEFT_PADDLE: [u8; 34] = hex34("010f7f7f7f7f000020000000126053b8096ffff50c2e00b8012e0000000000000000");
    /// The fast bumper L4 (byte 10, `0x01`).
    pub const L4: [u8; 34] = hex34("010f7f7f7f7f0000000001001260532c100b015402780046ff940000000000000000");
    /// LT fully pulled: the left trigger axis (byte 7) at `0xff` and the LT bit (byte 9, `0x01`).
    pub const LT: [u8; 34] = hex34("010f7f7f7f7f00ff00010000126053ac0a2a001c0d3601e4ffe2ff00000000000000");

    const fn hex34(text: &str) -> [u8; 34] {
        const fn nibble(c: u8) -> u8 {
            match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                _ => panic!("not a hex digit"),
            }
        }
        let b = text.as_bytes();
        assert!(b.len() == 68, "a capture row is 34 bytes");
        let mut out = [0u8; 34];
        let mut i = 0;
        while i < 34 {
            out[i] = nibble(b[2 * i]) << 4 | nibble(b[2 * i + 1]);
            i += 1;
        }
        out
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
