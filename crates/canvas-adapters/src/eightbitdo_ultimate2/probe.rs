//! The read-only ring probe (#376, stage S2): read the pad's stored profile and decode its ring
//! block, sending the pad nothing but read requests.
//!
//! What it does:
//! - [`capture_from_hardware`] opens the pad's vendor channel and asks for the profile image the
//!   way the adapter's own settings read does: 36 read requests (request word 2, 45 bytes each,
//!   the last 17), one per chunk of the 1592-byte image. A chunk the pad does not answer is asked
//!   for again (up to [`super::ATTEMPTS`] times), so the worst case is more than 36 reports; every
//!   one of them is a read request.
//! - [`ReadOnlyVendor`] sits between the read loop and the real channel and lets through only a
//!   byte-exact read request ([`is_read_request`]). A write chunk, a commit, a `Reportstate`, an
//!   identify, or anything else is refused **before** it reaches the transport and is counted in
//!   [`ReadOnlyVendor::refused`]. The read loop cannot send one by mistake, and the tests prove it.
//! - [`ProfileCapture`] keeps the raw replies (printed and saved as hex by [`ProfileCapture::to_text`]
//!   and read back by [`ProfileCapture::from_text`]) and the image they build.
//! - [`RingDump`] decodes the ring block of every profile: the three theme records
//!   (`dynamic` at `0x4B8`, `hotwheel` at `0x4DC`, `single` at `0x50C`), which of them the pad
//!   marks in use, the 24 custom colour words, and a guess at the colour byte order with the
//!   evidence for it ([`guess_byte_order`]).
//!
//! **Coexistence with the daemon.** The daemon holds the pad's vendor handle open while the
//! pad's peripheral reads are on, and reads it only after 10 s of keyboard and mouse idle. The
//! probe opens its own handle. Every reply is checked against its request (the offset it echoes,
//! the length, the CRC), so a reply that belongs to another reader's request is dropped and the
//! chunk is asked again. Whether Windows lets a second process open the vendor collection while
//! the daemon has it is UNKNOWN until it runs on the pad; if the open is refused the probe stops
//! and says so, and says how to free the channel (turn the pad's reads off in the editor, close
//! the vendor app).
//!
//! **Nothing in this file touches the pad except the reads above.** Decoding is pure bytes.

use std::fmt::Write as _;

use canvas_proto::traits::AdapterError;

use super::{
    codec, read_image_raw, refuse, HidOpener, InputTransport, Opener, Pace, Personality,
    VendorTransport,
};

/// How many chunks the profile read asks for: 1592 bytes in chunks of 45 (35 full, one of 17).
pub const CHUNKS: usize = codec::IMAGE_LEN.div_ceil(codec::CHUNK);

/// True when `report` is exactly the read request the adapter sends for some chunk of the image:
/// byte for byte what [`codec::read_request`] builds for the offset the report names.
pub fn is_read_request(report: &[u8]) -> bool {
    if report.len() != codec::REPORT_LEN || report[0] != codec::OUT_ID {
        return false;
    }
    let offset = u32::from_le_bytes([report[14], report[15], report[16], report[17]]) as usize;
    codec::read_request(offset).is_ok_and(|expected| expected == report)
}

/// A vendor channel that can only read: it lets through the profile read's own requests and
/// refuses everything else.
pub struct ReadOnlyVendor<'a> {
    inner: &'a mut dyn VendorTransport,
    /// How many read requests went through to the channel.
    pub sent: usize,
    /// Every report that was refused, in order. It stays empty on a correct run.
    pub refused: Vec<Vec<u8>>,
}

impl<'a> ReadOnlyVendor<'a> {
    /// Wraps a channel.
    pub fn new(inner: &'a mut dyn VendorTransport) -> Self {
        Self { inner, sent: 0, refused: Vec::new() }
    }
}

impl InputTransport for ReadOnlyVendor<'_> {
    fn read_input(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        self.inner.read_input(buf, timeout_ms)
    }
}

impl VendorTransport for ReadOnlyVendor<'_> {
    fn send_report(&mut self, data: &[u8]) -> Result<(), String> {
        if !is_read_request(data) {
            self.refused.push(data.to_vec());
            return Err(format!(
                "the ring probe is read-only: refused to send {}",
                codec::hex(&data[..data.len().min(8)])
            ));
        }
        self.sent += 1;
        self.inner.send_report(data)
    }
}

// ------------------------------------------------------------------------------------- the capture

/// The profile as the pad answered it: the raw reply of every chunk and the image they build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCapture {
    /// The personality the pad was showing, as a label (`XInput`), when known.
    pub personality: Option<String>,
    /// The accepted reply of every chunk, in order (64 bytes each).
    pub replies: Vec<Vec<u8>>,
    /// The image the replies carry, all 1592 bytes.
    pub image: Vec<u8>,
    /// How many read requests the probe sent to get it (36 when every chunk answered at once).
    pub requests_sent: usize,
}

/// Reads the profile from an open vendor channel, read-only. Blocking.
///
/// Fails when the read fails or when it does not bring back exactly [`CHUNKS`] replies and the
/// whole image. Fails, too, if the read loop ever tried to send anything but a read request
/// (it cannot today; the check is the guard against a future change).
pub(super) fn capture_profile(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    personality: Option<&str>,
) -> Result<ProfileCapture, AdapterError> {
    let mut guard = ReadOnlyVendor::new(vendor);
    let read = read_image_raw(&mut guard, pace);
    if !guard.refused.is_empty() {
        return Err(refuse(format!(
            "the probe's read loop tried to send {} report(s) that are not read requests; none were sent",
            guard.refused.len()
        )));
    }
    let requests_sent = guard.sent;
    let (image, replies) = read?;
    if image.len() != codec::IMAGE_LEN || replies.len() != CHUNKS {
        return Err(refuse(format!(
            "the read brought back {} bytes in {} replies, expected {} in {CHUNKS}",
            image.len(),
            replies.len(),
            codec::IMAGE_LEN
        )));
    }
    Ok(ProfileCapture { personality: personality.map(str::to_string), replies, image, requests_sent })
}

/// Opens the pad's vendor channel through `opener` and reads the profile. Blocking.
///
/// Refuses, with a message that says what to do, when no pad is found, when the receiver is idle,
/// when the personality has no vendor channel, and when the channel will not open.
pub fn capture_from_opener(opener: &dyn Opener) -> Result<ProfileCapture, AdapterError> {
    capture_from_opener_paced(opener, Pace::default())
}

/// [`capture_from_opener`] with the pauses between chunks set by the caller (the tests use none).
pub(super) fn capture_from_opener_paced(
    opener: &dyn Opener,
    pace: Pace,
) -> Result<ProfileCapture, AdapterError> {
    let (mut vendor, personality) = open_pad_vendor(opener)?;
    capture_profile(vendor.as_mut(), pace, Some(personality.label()))
}

/// Opens the pad's vendor channel through `opener`. Blocking.
///
/// Refuses, with a message that says what to do, when no pad is found, when the receiver is idle,
/// when the personality has no vendor channel, and when the channel will not open. Nothing is
/// sent.
pub(super) fn open_pad_vendor(
    opener: &dyn Opener,
) -> Result<(Box<dyn VendorTransport>, Personality), AdapterError> {
    let personalities = opener.personalities();
    let Some(personality) = personalities.first().copied() else {
        return Err(AdapterError::NotConnected);
    };
    match personality {
        Personality::Idle => {
            return Err(refuse(
                "the receiver is idle: no pad is linked to it, so nothing answers the profile read.                  Turn the pad on (press a button on it) and run again",
            ));
        }
        Personality::DInput => {
            return Err(refuse(
                "the pad is in its DInput personality, which has no vendor channel to read the                  profile through",
            ));
        }
        Personality::Bluetooth => {
            return Err(refuse(super::BLUETOOTH_NO_SETTINGS));
        }
        Personality::XInput => {}
    }
    match opener.open_vendor(personality) {
        Ok(Some(v)) => Ok((v, personality)),
        Ok(None) => Err(refuse("the pad shows no vendor channel to read the profile through")),
        Err(why) => Err(refuse(format!(
            "the pad's vendor channel would not open: {why}. Another program may hold it: the              canvas-rgb daemon (turn the pad's reads off on the Peripherals page, then run              again) or the 8BitDo app (close it). Nothing was sent"
        ))),
    }
}

/// Reads the profile from the real pad with hidapi. Blocking.
pub fn capture_from_hardware() -> Result<ProfileCapture, AdapterError> {
    capture_from_opener(&HidOpener::<hidapi::HidApi>::default())
}

/// What `--check` shows: the personalities the pad is showing, with nothing opened.
pub fn enumerate_personalities() -> Vec<Personality> {
    HidOpener::<hidapi::HidApi>::default().personalities()
}

// ------------------------------------------------------------------------------------ the text file

const TEXT_HEADER: &str = "# canvas-rgb ring probe: the pad's profile image, read-only (#376 S2)";

impl ProfileCapture {
    /// The capture as text: one `reply NN offset 0xXXXX: <hex>` line per chunk, then the image.
    /// [`ProfileCapture::from_text`] reads it back.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{TEXT_HEADER}");
        let _ = writeln!(out, "# personality: {}", self.personality.as_deref().unwrap_or("unknown"));
        let _ = writeln!(out, "# image_len: {}", self.image.len());
        let _ = writeln!(out, "# requests_sent: {}", self.requests_sent);
        for (n, reply) in self.replies.iter().enumerate() {
            let offset = n * codec::CHUNK;
            let _ = writeln!(out, "reply {n:02} offset {offset:#06x}: {}", codec::hex(reply));
        }
        for (n, row) in self.image.chunks(32).enumerate() {
            let _ = writeln!(out, "image {:#06x}: {}", n * 32, codec::hex(row));
        }
        out
    }

    /// Reads a capture back from [`ProfileCapture::to_text`]'s text, checking every reply the way
    /// the live read does (echoed offset, length, CRC) and rebuilding the image from them.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut personality = None;
        let mut requests_sent = CHUNKS;
        let mut replies: Vec<Vec<u8>> = Vec::new();
        for line in text.lines() {
            if let Some(p) = line.strip_prefix("# personality: ") {
                personality = (p != "unknown").then(|| p.to_string());
            } else if let Some(n) = line.strip_prefix("# requests_sent: ") {
                requests_sent = n.trim().parse().map_err(|_| format!("bad requests_sent: {n:?}"))?;
            } else if line.starts_with("reply ") {
                let (_, hex) = line.split_once(": ").ok_or_else(|| format!("bad reply line: {line:?}"))?;
                replies.push(parse_hex(hex)?);
            }
        }
        if replies.len() != CHUNKS {
            return Err(format!("the file holds {} replies, expected {CHUNKS}", replies.len()));
        }
        let mut image = Vec::with_capacity(codec::IMAGE_LEN);
        for reply in &replies {
            let offset = image.len();
            let len = codec::CHUNK.min(codec::IMAGE_LEN - offset);
            let data = codec::parse_read_reply(reply, len, offset)?;
            image.extend_from_slice(data);
        }
        Ok(Self { personality, replies, image, requests_sent })
    }
}

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).map_err(|_| format!("bad hex byte {b:?}")))
        .collect()
}

// ------------------------------------------------------------------------------------- the decode

/// One theme record as the image holds it, undecoded: the flag word and the bytes after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeRecord {
    /// The record's flag word. [`codec::ENABLED`] marks it in use.
    pub flag: u32,
    /// The colour words after the flag, 4 raw bytes each, in file order.
    pub words: Vec<[u8; 4]>,
    /// Bytes after the colour words (the fire ring's speed byte and its padding).
    pub tail: Vec<u8>,
}

impl ThemeRecord {
    /// True when the pad marks the record in use.
    pub fn in_use(&self) -> bool {
        self.flag == codec::ENABLED
    }
}

/// The three ring themes of one profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRings {
    /// The light-tracing theme: foreground and background colour words.
    pub dynamic: ThemeRecord,
    /// The fire-ring theme: foreground and background colour words, then a speed byte.
    pub hotwheel: ThemeRecord,
    /// The custom theme: 24 colour words, 12 per ring (which index is which LED is UNKNOWN).
    pub single: ThemeRecord,
}

/// The ring block of the whole image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingDump {
    /// The mode the image stores, when it stores one the research names.
    pub stored_mode: Option<&'static str>,
    /// The active profile (0-based).
    pub active_profile: Option<usize>,
    /// The three profiles.
    pub profiles: [ProfileRings; codec::PROFILES],
}

/// Size of one `dynamic` record: flag, colour, background colour.
const DYNAMIC_LEN: usize = 12;
/// Size of one `hotwheel` record: flag, colour, background colour, speed byte and padding.
const HOTWHEEL_LEN: usize = 16;
/// Size of one `single` record: flag and 24 colour words.
const SINGLE_LEN: usize = 0x64;
/// The custom theme's colours, 12 per ring.
pub const CUSTOM_COLOURS: usize = 24;

fn record(img: &[u8], at: usize, len: usize, words: usize) -> ThemeRecord {
    let flag = u32::from_le_bytes([img[at], img[at + 1], img[at + 2], img[at + 3]]);
    let words_at = at + 4;
    let mut out = Vec::with_capacity(words);
    for w in 0..words {
        let s = words_at + 4 * w;
        out.push([img[s], img[s + 1], img[s + 2], img[s + 3]]);
    }
    let tail = img[words_at + 4 * words..at + len].to_vec();
    ThemeRecord { flag, words: out, tail }
}

impl RingDump {
    /// Decodes the ring block of an image.
    pub fn decode(img: &[u8]) -> Result<Self, String> {
        if img.len() != codec::IMAGE_LEN {
            return Err(format!("the image is {} bytes, not {}", img.len(), codec::IMAGE_LEN));
        }
        let profile = |n: usize| ProfileRings {
            dynamic: record(img, codec::DYNAMIC + DYNAMIC_LEN * n, DYNAMIC_LEN, 2),
            hotwheel: record(img, codec::HOTWHEEL + HOTWHEEL_LEN * n, HOTWHEEL_LEN, 2),
            single: record(img, codec::SINGLE + SINGLE_LEN * n, SINGLE_LEN, CUSTOM_COLOURS),
        };
        Ok(Self {
            stored_mode: codec::stored_mode(img),
            active_profile: codec::active_profile(img),
            profiles: [profile(0), profile(1), profile(2)],
        })
    }

    /// Every colour word of every record the pad marks in use, in file order.
    pub fn colour_words_in_use(&self) -> Vec<[u8; 4]> {
        let mut out = Vec::new();
        for p in &self.profiles {
            for r in [&p.dynamic, &p.hotwheel, &p.single] {
                if r.in_use() {
                    out.extend(r.words.iter().copied());
                }
            }
        }
        out
    }
}

/// A colour word as 8 hex digits in file order.
fn word_hex(w: &[u8; 4]) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", w[0], w[1], w[2], w[3])
}

// ------------------------------------------------------------------------------------ byte order

/// A guess at the colour byte order, with what it rests on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteOrderGuess {
    /// The guess, in one line.
    pub verdict: String,
    /// What the guess rests on, one fact per line.
    pub evidence: Vec<String>,
}

/// Guesses how a colour word's four bytes are laid out, from words the pad holds.
///
/// What the words can show: which byte position never varies (a pad byte) and which three carry
/// the colour. What they cannot show: which of the three is red. Stored words say nothing about
/// that until one of them is a colour whose name is known; the vendor's own defaults are not in
/// the research. So the verdict names the research's reading (`u32 RGB`, a word `0x00RRGGBB`
/// stored little-endian, which puts the bytes in file order as blue, green, red, pad) as the
/// guess and marks it unconfirmed. Stage S3's pure-red write settles it.
pub fn guess_byte_order(words: &[[u8; 4]]) -> ByteOrderGuess {
    let mut evidence = Vec::new();
    if words.is_empty() {
        return ByteOrderGuess {
            verdict: "UNKNOWN: no ring record is marked in use, so there are no colour words to look at"
                .into(),
            evidence,
        };
    }
    evidence.push(format!("{} colour word(s) from records the pad marks in use", words.len()));
    let mut constant_at = Vec::new();
    for pos in 0..4 {
        let mut values: Vec<u8> = words.iter().map(|w| w[pos]).collect();
        values.sort_unstable();
        values.dedup();
        let (lo, hi) = (values[0], values[values.len() - 1]);
        evidence.push(format!(
            "byte {pos}: {} distinct value(s), range {lo:#04x}..{hi:#04x}",
            values.len()
        ));
        if values.len() == 1 {
            constant_at.push((pos, lo));
        }
    }
    let pure: Vec<usize> = words
        .iter()
        .filter_map(|w| {
            let lit: Vec<usize> = (0..3).filter(|&p| w[p] != 0).collect();
            (lit.len() == 1).then(|| lit[0])
        })
        .collect();
    if !pure.is_empty() {
        let mut tally = [0usize; 3];
        for p in &pure {
            tally[*p] += 1;
        }
        evidence.push(format!(
            "pure-primary words (one of bytes 0-2 lit): {} at byte 0, {} at byte 1, {} at byte 2",
            tally[0], tally[1], tally[2]
        ));
    }
    let verdict = match constant_at.as_slice() {
        [(3, v)] => {
            evidence.push(format!("byte 3 is {v:#04x} in every word: a pad byte, not colour"));
            "bytes 0..2 carry the colour and byte 3 is a pad. Best reading: a u32 0x00RRGGBB stored \
             little-endian, so file order is B, G, R, pad. UNCONFIRMED: the words cannot say which \
             byte is red; the S3 pure-red write settles it"
        }
        [] => "UNKNOWN: no byte position is constant, so the words are not 3 colour bytes plus a pad",
        _ => "UNKNOWN: a constant byte sits at an unexpected position; the layout is not settled",
    };
    ByteOrderGuess { verdict: verdict.to_string(), evidence }
}

// --------------------------------------------------------------------------------------- the report

impl ProfileCapture {
    /// The raw replies as hex, one line each, for the console.
    pub fn raw_listing(&self) -> String {
        let mut out = String::new();
        for (n, reply) in self.replies.iter().enumerate() {
            let _ = writeln!(out, "reply {n:02} offset {:#06x}: {}", n * codec::CHUNK, codec::hex(reply));
        }
        out
    }

    /// What the probe can decode, in words: the ring block of every profile and the byte-order
    /// guess. Every statement is either a thing read from the bytes or marked UNKNOWN.
    pub fn decode_report(&self) -> Result<String, String> {
        let dump = RingDump::decode(&self.image)?;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "stored mode: {}; active profile: {}",
            dump.stored_mode.unwrap_or("UNKNOWN"),
            dump.active_profile.map_or("UNKNOWN".to_string(), |p| (p + 1).to_string())
        );
        for (n, p) in dump.profiles.iter().enumerate() {
            let marked: Vec<&str> = [
                ("light-tracing (dynamic)", &p.dynamic),
                ("fire ring (hotwheel)", &p.hotwheel),
                ("custom (single)", &p.single),
            ]
            .iter()
            .filter(|(_, r)| r.in_use())
            .map(|(name, _)| *name)
            .collect();
            let _ = writeln!(
                out,
                "profile {}{}: ring records marked in use: {}",
                n + 1,
                if dump.active_profile == Some(n) { " (active)" } else { "" },
                if marked.is_empty() { "none".to_string() } else { marked.join(", ") }
            );
            let _ = writeln!(
                out,
                "  ring mode: {}",
                match marked.as_slice() {
                    [] => "UNKNOWN (no theme record is marked; the ring may be off)".to_string(),
                    [one] => format!("{one} is the only record marked; the image does not say which \
                                      theme is shown, so this is a reading, not a fact"),
                    _ => "UNKNOWN (several records are marked; the image does not store which theme is \
                          shown)"
                        .to_string(),
                }
            );
            let _ = writeln!(
                out,
                "  light-tracing: flag {:#010x} colour {} background {}",
                p.dynamic.flag,
                word_hex(&p.dynamic.words[0]),
                word_hex(&p.dynamic.words[1])
            );
            let _ = writeln!(
                out,
                "  fire ring: flag {:#010x} colour {} background {} speed byte {:#04x}",
                p.hotwheel.flag,
                word_hex(&p.hotwheel.words[0]),
                word_hex(&p.hotwheel.words[1]),
                p.hotwheel.tail.first().copied().unwrap_or(0)
            );
            let _ = writeln!(out, "  custom: flag {:#010x}", p.single.flag);
            for (ring, half) in p.single.words.chunks(12).enumerate() {
                let words: Vec<String> = half.iter().map(word_hex).collect();
                let _ = writeln!(
                    out,
                    "    colours {:>2}-{:>2} (ring {}, a split by index; which LED is which is UNKNOWN): {}",
                    ring * 12,
                    ring * 12 + 11,
                    ring + 1,
                    words.join(" ")
                );
            }
        }
        let guess = guess_byte_order(&dump.colour_words_in_use());
        let _ = writeln!(out, "byte order: {}", guess.verdict);
        for line in &guess.evidence {
            let _ = writeln!(out, "  evidence: {line}");
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::fake_pad::{self, FakePad};

    fn pace() -> Pace {
        Pace {
            poll_ms: 1,
            chunk_pause: Duration::ZERO,
            write_pause: Duration::ZERO,
            session_pause: Duration::ZERO,
        }
    }

    #[test]
    fn the_probe_reads_exactly_thirty_six_chunks_and_every_report_it_sends_is_a_read_request() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let capture = capture_from_opener_paced(&pad, pace()).expect("the fake pad answers");
        let sent = pad.sent_reports();
        assert_eq!(CHUNKS, 36);
        assert_eq!(sent.len(), 36, "one request per chunk, none repeated");
        assert_eq!(capture.requests_sent, 36);
        assert!(sent.iter().all(|r| is_read_request(r)), "a report that is not a read request went out");
        assert_eq!(pad.non_read_reports(), 0, "the pad was sent something that is not a read");
        assert_eq!(capture.image, fake_pad::ring_fixture_image());
        assert_eq!(capture.replies.len(), 36);
    }

    #[test]
    fn the_fake_counts_a_write_report_so_the_zero_above_is_not_vacuous() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let mut vendor = pad.vendor();
        vendor.send_report(&codec::commit_request()).unwrap();
        vendor.send_report(&codec::write_request(codec::SINGLE, &[1, 2, 3]).unwrap()).unwrap();
        assert_eq!(pad.non_read_reports(), 2);
    }

    #[test]
    fn a_write_a_commit_a_report_state_and_an_identify_never_reach_the_channel() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let mut vendor = pad.vendor();
        let mut guard = ReadOnlyVendor::new(&mut vendor);
        let attempts = [
            codec::write_request(codec::SINGLE, &[0xFF; 8]).unwrap(),
            codec::commit_request(),
            codec::report_state_request(0),
            codec::report_state_request(1),
            codec::report_state_connect_request(1),
            codec::identify_request(),
        ];
        for report in &attempts {
            assert!(guard.send_report(report).is_err(), "{} went through", codec::hex(&report[..6]));
        }
        assert_eq!(guard.sent, 0);
        assert_eq!(guard.refused.len(), attempts.len());
        assert!(pad.sent_reports().is_empty(), "the fake pad heard a report the guard should have stopped");
        // A read request that is one byte off the real thing is refused too.
        let mut off = codec::read_request(0).unwrap();
        off[20] ^= 1;
        assert!(guard.send_report(&off).is_err());
        assert!(pad.sent_reports().is_empty());
        // The genuine read request goes through.
        assert!(guard.send_report(&codec::read_request(0).unwrap()).is_ok());
        assert_eq!(pad.sent_reports().len(), 1);
    }

    #[test]
    fn a_pad_that_goes_quiet_ends_the_read_with_reads_only_on_the_wire() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        pad.go_silent_after(10);
        let err = capture_from_opener_paced(&pad, pace()).expect_err("a silent pad is a failed read");
        assert!(err.to_string().contains("did not answer"), "{err}");
        assert_eq!(pad.non_read_reports(), 0);
        assert!(!pad.sent_reports().is_empty());
    }

    #[test]
    fn an_idle_receiver_and_a_dinput_pad_are_refused_with_nothing_sent() {
        for personality in [Personality::Idle, Personality::DInput] {
            let pad = FakePad::with_image(fake_pad::ring_fixture_image(), personality);
            let err = capture_from_opener_paced(&pad, pace()).expect_err("no vendor channel to read");
            assert!(!err.to_string().is_empty());
            assert!(pad.sent_reports().is_empty());
        }
    }

    #[test]
    fn a_channel_that_will_not_open_says_who_may_hold_it() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        pad.refuse_open("access denied");
        let err = capture_from_opener_paced(&pad, pace()).expect_err("the open was refused");
        let text = err.to_string();
        assert!(text.contains("daemon") && text.contains("Nothing was sent"), "{text}");
        assert!(pad.sent_reports().is_empty());
    }

    #[test]
    fn the_saved_text_reads_back_to_the_same_capture() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let capture = capture_from_opener_paced(&pad, pace()).unwrap();
        let back = ProfileCapture::from_text(&capture.to_text()).unwrap();
        assert_eq!(back, capture);
        // A corrupted reply is refused the way the live read refuses it.
        let text = capture.to_text();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let at = lines.iter().position(|l| l.starts_with("reply 03")).unwrap();
        lines[at] = lines[at].replacen(": 02 04 04 00", ": 02 04 04 01", 1);
        assert!(ProfileCapture::from_text(&lines.join("\n")).is_err());
    }

    #[test]
    fn the_checked_in_fixture_is_what_the_fake_pad_replays() {
        let from_file = fake_pad::ring_fixture_capture();
        assert_eq!(from_file.image, fake_pad::ring_fixture_image());
        let pad = FakePad::from_capture(&from_file, Personality::XInput);
        let again = capture_from_opener_paced(&pad, pace()).unwrap();
        assert_eq!(again.replies, from_file.replies, "the fake did not replay the file byte for byte");
    }

    #[test]
    fn the_decode_names_the_marked_records_the_colours_and_the_byte_order_guess() {
        let report = fake_pad::ring_fixture_capture().decode_report().unwrap();
        assert!(report.contains("stored mode: XInput; active profile: 3"), "{report}");
        assert!(report.contains("profile 3 (active): ring records marked in use: custom (single)"), "{report}");
        assert!(report.contains("profile 1: ring records marked in use: none"), "{report}");
        // Profile 3's first custom colour is the fixture's 0x00FF0000-style red word: B G R pad.
        assert!(report.contains("colours  0-11 (ring 1"), "{report}");
        assert!(report.contains("0000ff00"), "{report}");
        assert!(report.contains("byte order: bytes 0..2 carry the colour and byte 3 is a pad"), "{report}");
        assert!(report.contains("evidence: byte 3: 1 distinct value(s)"), "{report}");
        assert!(report.contains("UNCONFIRMED"), "{report}");
    }

    #[test]
    fn the_byte_order_guess_is_unknown_when_nothing_is_marked_or_the_layout_is_odd() {
        let none = guess_byte_order(&[]);
        assert!(none.verdict.starts_with("UNKNOWN"));
        // Every position varies: not three colour bytes and a pad.
        let busy = guess_byte_order(&[[1, 2, 3, 4], [5, 6, 7, 8]]);
        assert!(busy.verdict.starts_with("UNKNOWN"), "{}", busy.verdict);
        // The constant byte is at position 0, not 3.
        let odd = guess_byte_order(&[[9, 1, 2, 3], [9, 4, 5, 6]]);
        assert!(odd.verdict.starts_with("UNKNOWN"), "{}", odd.verdict);
    }

    #[test]
    fn a_capture_of_the_wrong_size_does_not_decode() {
        assert!(RingDump::decode(&[0u8; 100]).is_err());
    }
}
