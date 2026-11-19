//! The 8BitDo Ultimate 2's two stick rings as a light the engine can drive (#376).
//!
//! **Stills, not streaming.** The rings live in the pad's stored profile (the custom theme holds
//! 24 colours, 12 per ring). Every write to that profile is committed to the pad's flash, so a
//! frame per tick is not on offer. This adapter takes **one still colour set per scene**: a frame
//! is sent to the pad only when its 24 colours differ from the set the pad was last given, and
//! only when every gate below allows it. The group shows such a device as taking stills.
//!
//! The device is `ring:8bitdo-ultimate-2`: 2 zones of 12 LEDs, `Linear` (a ring's wrap-around
//! order is UNKNOWN), kind `Other`, no resizing. It shares the pad's vendor channel with the
//! peripheral adapter through a [`VendorHandle`] and never opens a channel of its own.
//!
//! # What gates a write (each one a test on the fake pad)
//!
//! 1. **The frame hash.** A frame whose 24 packed colour words hash like the set the pad holds
//!    sends nothing. The hash is kept only after the commit was acknowledged and the pad's image
//!    read back as expected; a failed or partial write clears it, so the next scene change writes
//!    in full.
//! 2. **The peripheral write gate** ([`VendorHandle::writes_enabled`]): what `PERIPHERAL_ACCESS`
//!    turns on. With it off, nothing is sent, however often the scene changes.
//! 3. **An open channel.** The channel is open only while the pad's reads are on.
//! 4. **The idle rule.** The person must have been away from the keyboard and mouse for
//!    [`IDLE_BEFORE_WRITE`], the same quiet the daemon waits for before it reads the pad. A
//!    write is a stall on the receiver's radio link, and nobody is mid-game when it lands. With no
//!    idle clock set the adapter sends nothing.
//! 5. **The caps**, copied from the keyboard's stills until the pad's flash rating is known:
//!    [`UPLOAD_MIN_GAP`] between attempts, [`UPLOADS_PER_HOUR_MAX`] an hour,
//!    [`UPLOADS_PER_DAY_MAX`] a day. An attempt is counted, and written to the store, **before**
//!    its first chunk goes out. The store ([`StillStore`], a JSON file the daemon places) keeps
//!    the attempts and the hash across restarts, so a restart cannot reset the caps or rewrite a
//!    set the pad already holds. A store that is missing holds writes off for [`STORE_HOLD_OFF`];
//!    one that cannot be trusted holds them off for [`LOST_HISTORY_HOLD_OFF`]; one that cannot be
//!    written stops writes for the rest of the run.
//!
//! # What one write does
//!
//! It reads the pad's profile image (36 read requests), refuses unless the active profile's custom
//! ring record is marked in use, writes that one 100-byte record in three chunks, commits, reads
//! the image again and compares it with what it expected. If the pad's image differs anywhere,
//! the saved record is written back and committed, and the image is read once more to say whether
//! the restore took. The write session is the adapter's own ([`super::write_session`]):
//! `Reportstate(0)`, the chunks, the commit, `Reportstate(1)`.
//!
//! # What the pad has settled, and what it has not
//!
//! S2 (2026-11-18) read the stored profile; S3 (the same day) wrote 24 words of `00 00 ff 00`, the
//! pad read the image back as written, and both rings showed red. So the **byte order is B, G, R,
//! pad: CONFIRMED.** The **order of the 24 LEDs is UNKNOWN** until the marker run (`ring_s3
//! --marker`). Both live in [`pack_ring_colours`], and nowhere else. The commit argument `0x0123`
//! was accepted by the pad on that write.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canvas_proto::color::Rgb8;
use canvas_proto::device::{
    AdapterCapabilities, DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind,
};
use canvas_proto::traits::{AdapterError, DeviceAdapter};

use super::probe::CUSTOM_COLOURS;
use super::{
    codec, read_image, refuse, write_session, Pace, VendorHandle, VendorTransport, DISPLAY_NAME,
    MANUFACTURER, VENDOR_ID,
};
use crate::royuan::{
    StillStore, StoredStills, LOST_HISTORY_HOLD_OFF, STORE_HOLD_OFF, UPLOADS_PER_DAY_MAX,
    UPLOADS_PER_HOUR_MAX, UPLOAD_MIN_GAP,
};

/// The id prefix of the ring device; the daemon's composite routes on it.
pub const RING_PREFIX: &str = "ring:";
/// How many rings: one round each stick.
pub const RINGS: usize = 2;
/// LEDs per ring.
pub const LEDS_PER_RING: usize = 12;
/// LEDs on the device: the length of every frame it takes.
pub const RING_LEDS: usize = RINGS * LEDS_PER_RING;

/// How long the person must have been away from the keyboard and mouse before a ring write goes
/// out. It is the daemon's `PERIPHERAL_IDLE_READ_S`; a daemon test holds the two equal.
pub const IDLE_BEFORE_WRITE: Duration = Duration::from_secs(10);

const ONE_HOUR_MS: u64 = 3_600_000;
const ONE_DAY_MS: u64 = 86_400_000;

/// The stable id of the ring device.
pub fn ring_device_id() -> DeviceId {
    DeviceId(format!("{RING_PREFIX}8bitdo-ultimate-2"))
}

// ------------------------------------------------------------------------------------ the mapping

/// Packs a frame of 24 colours into the 24 colour words of the custom theme record, in the order
/// the pad stores them.
///
/// **This is the one place that knows the pad's byte order and LED order.**
/// - *Byte order: CONFIRMED, B, G, R, pad.* S3 ran on the real pad on 2026-11-18
///   (`ring_s3 --order bgr`): a pure-red write, packed as `00 00 ff 00`, read back as written
///   and Luke saw both rings red. So a colour word's bytes in file order are blue, green, red and
///   a pad byte of 0 (the research's `u32 0x00RRGGBB`, little-endian).
/// - *LED order: TODO(S3 marker run), UNKNOWN.* Zone 0 is taken as words 0-11 and zone 1 as words
///   12-23, each in frame order. Which index is which LED, which ring is the left stick's, and
///   which way a ring runs are UNKNOWN until `ring_s3 --marker` is run on the pad.
///
/// No other function may know either order.
pub fn pack_ring_colours(frame: &[Rgb8; RING_LEDS]) -> [[u8; 4]; CUSTOM_COLOURS] {
    pack_ring_colours_as(frame, DEFAULT_ORDER)
}

/// The byte order of a colour word in the custom theme record. S3 settled it: [`ByteOrder::Bgr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    /// File order blue, green, red, pad: the research's `u32 0x00RRGGBB`, little-endian. This is
    /// the pad's order (S3, 2026-11-18).
    Bgr,
    /// File order red, green, blue, pad. Shown red by no pad seen so far; kept so the S3 tool can
    /// try it.
    Rgb,
}

/// The order the adapter writes with: B, G, R, pad. CONFIRMED by S3 on the pad (2026-11-18).
pub const DEFAULT_ORDER: ByteOrder = ByteOrder::Bgr;

/// [`pack_ring_colours`] with a byte order of the caller's: how the S3 tool tries a candidate. The
/// LED order is still the one in [`pack_ring_colours`]'s doc.
pub fn pack_ring_colours_as(frame: &[Rgb8; RING_LEDS], order: ByteOrder) -> [[u8; 4]; CUSTOM_COLOURS] {
    let mut words = [[0u8; 4]; CUSTOM_COLOURS];
    for (word, c) in words.iter_mut().zip(frame.iter()) {
        *word = match order {
            ByteOrder::Bgr => [c.b, c.g, c.r, 0],
            ByteOrder::Rgb => [c.r, c.g, c.b, 0],
        };
    }
    words
}

/// FNV-1a over the packed words: the frame hash. Same words, same hash; nothing else is promised.
fn frame_hash(words: &[[u8; 4]; CUSTOM_COLOURS]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in words.iter().flatten() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// ------------------------------------------------------------------------------------- the clock

/// Where the adapter gets the wall clock, as Unix milliseconds. A trait so a test steps it.
pub trait RingClock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn unix_ms(&self) -> u64;
}

/// The system clock.
pub struct SystemClock;

impl RingClock for SystemClock {
    fn unix_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
}

// ------------------------------------------------------------------------------------ the status

/// Why the last frame was not sent. The ring panel shows these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// The peripheral write gate is off.
    WritesOff,
    /// The pad's channel is closed (its reads are off, or the pad is away).
    ChannelClosed,
    /// The person was at the keyboard or mouse a moment ago, or no idle clock is set.
    NotIdle,
    /// A cap holds the write until this wall-clock time, Unix milliseconds.
    CapUntil(u64),
    /// The store is missing or cannot be trusted; writes wait until this time.
    StoreHoldUntil(u64),
    /// The store cannot be written: no write goes out for the rest of the run.
    StoreDead,
}

/// What the adapter knows about its writes. It never says what the rings show: the light cannot
/// be seen from here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingWriteStatus {
    /// When the last write attempt went out, Unix milliseconds.
    pub last_sent_ms: Option<u64>,
    /// True when the last attempt finished whole (commit acknowledged, image read back as
    /// expected). False after a failed or partial write: the rings may be partly written.
    pub complete: bool,
    /// Why the newest frame has not been sent, when something holds it.
    pub held: Option<Held>,
}

// ----------------------------------------------------------------------------------- the adapter

/// The two stick rings as a light of 24 LEDs that takes stills.
pub struct RingLightAdapter {
    handle: VendorHandle,
    store: Option<Box<dyn StillStore>>,
    clock: Arc<dyn RingClock>,
    pace: Pace,
    /// The hash of the set the pad holds, once a write finished whole (or the store said so).
    shown: Option<u64>,
    /// The colours that went with `shown`, for `read_back`.
    shown_colors: Vec<Rgb8>,
    /// The wall-clock time of each attempt of the last day, oldest first.
    attempts_ms: VecDeque<u64>,
    loaded: bool,
    hold_off_until: Option<u64>,
    day_full_until: Option<u64>,
    save_dead: bool,
    status: RingWriteStatus,
}

impl RingLightAdapter {
    /// The ring adapter over the pad adapter's vendor handle, with its bookkeeping in `store`.
    pub fn new(handle: VendorHandle, store: Box<dyn StillStore>) -> Self {
        Self::build(handle, Some(store))
    }

    /// The same with no store: the counters live in memory only. For tests; the daemon always
    /// gives a store, because a restart must not reset the caps.
    pub fn without_store(handle: VendorHandle) -> Self {
        Self::build(handle, None)
    }

    fn build(handle: VendorHandle, store: Option<Box<dyn StillStore>>) -> Self {
        Self {
            handle,
            store,
            clock: Arc::new(SystemClock),
            pace: Pace::default(),
            shown: None,
            shown_colors: vec![Rgb8::BLACK; RING_LEDS],
            attempts_ms: VecDeque::new(),
            loaded: false,
            hold_off_until: None,
            day_full_until: None,
            save_dead: false,
            status: RingWriteStatus::default(),
        }
    }

    /// A clock of the caller's (a test's).
    pub fn with_clock(mut self, clock: Arc<dyn RingClock>) -> Self {
        self.clock = clock;
        self
    }

    /// No pauses between the chunks and the commit. For a fake pad; the real pad needs its pauses.
    pub fn without_pauses(mut self) -> Self {
        self.pace = Pace {
            poll_ms: 1,
            chunk_pause: Duration::ZERO,
            write_pause: Duration::ZERO,
            session_pause: Duration::ZERO,
        };
        self
    }

    /// What the adapter knows about its writes.
    pub fn status(&self) -> RingWriteStatus {
        self.status
    }

    /// The device this adapter lists.
    pub fn device_info(&self) -> DeviceInfo {
        let zones = (0..RINGS)
            .map(|i| ZoneInfo {
                index: i as u32,
                name: format!("Ring {}", i + 1),
                kind: ZoneKind::Linear,
                led_count: LEDS_PER_RING as u32,
                min_leds: LEDS_PER_RING as u32,
                max_leds: LEDS_PER_RING as u32,
                resizable: false,
            })
            .collect();
        DeviceInfo {
            id: ring_device_id(),
            name: format!("{DISPLAY_NAME} rings"),
            vendor: MANUFACTURER.to_string(),
            kind: DeviceKind::Other,
            adapter: self.name().to_string(),
            adapter_index: 0,
            serial: format!("{VENDOR_ID:04x}"),
            location: "2.4G receiver".to_string(),
            zones,
            connected: self.handle.is_open(),
        }
    }

    fn check_id(&self, id: &DeviceId) -> Result<(), AdapterError> {
        if *id == ring_device_id() {
            Ok(())
        } else {
            Err(AdapterError::UnknownDevice(id.clone()))
        }
    }

    /// Reads the store once: the attempts of the last day and the hash of the set the pad holds.
    fn load_stored(&mut self, now_ms: u64) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Some(store) = self.store.as_ref() else {
            return;
        };
        match store.load() {
            Ok(Some(stored)) => {
                let newest = stored.attempts_ms.iter().copied().max().unwrap_or(0);
                if newest > now_ms.saturating_add(ONE_DAY_MS) {
                    self.lost_history("its newest stamp is more than a day ahead", now_ms);
                    return;
                }
                self.attempts_ms = stored
                    .attempts_ms
                    .into_iter()
                    .filter(|t| now_ms.saturating_sub(*t) < ONE_DAY_MS)
                    .collect();
                // The hash is only the pad's set if the last write finished whole: the store
                // clears it before an attempt's first chunk and sets it after the last check.
                self.shown = stored.picture;
            }
            Ok(None) => {
                tracing::info!("the rings' still bookkeeping is not there; writes wait ten minutes");
                self.hold_off_until = Some(now_ms + STORE_HOLD_OFF.as_millis() as u64);
            }
            Err(e) => self.lost_history(&e, now_ms),
        }
    }

    fn lost_history(&mut self, why: &str, now_ms: u64) {
        tracing::warn!(error = %why, "the rings' still bookkeeping cannot be trusted; no write for a day");
        self.day_full_until = Some(now_ms + LOST_HISTORY_HOLD_OFF.as_millis() as u64);
        self.shown = None;
        self.attempts_ms.clear();
    }

    /// Writes the attempts and the hash back, durably. An error stops writes for the run.
    fn persist(&mut self, picture: Option<u64>) -> Result<(), ()> {
        if self.save_dead {
            return Err(());
        }
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let stored = StoredStills {
            attempts_ms: self.attempts_ms.iter().copied().collect(),
            picture,
            modes_13_ms: Vec::new(),
        };
        match store.save(&stored) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.save_dead = true;
                tracing::warn!(error = %e, "the rings' still bookkeeping cannot be written; no ring write this run");
                Err(())
            }
        }
    }

    /// Which gate, if any, holds a write at `now_ms`.
    fn held_at(&self, now_ms: u64) -> Option<Held> {
        if !self.handle.writes_enabled() {
            return Some(Held::WritesOff);
        }
        if !self.handle.is_open() {
            return Some(Held::ChannelClosed);
        }
        if !self.handle.idle_for().is_some_and(|idle| idle >= IDLE_BEFORE_WRITE) {
            return Some(Held::NotIdle);
        }
        if self.save_dead {
            return Some(Held::StoreDead);
        }
        if let Some(until) = self.day_full_until.filter(|u| *u > now_ms) {
            return Some(Held::StoreHoldUntil(until));
        }
        if let Some(until) = self.hold_off_until.filter(|u| *u > now_ms) {
            return Some(Held::StoreHoldUntil(until));
        }
        let gap = UPLOAD_MIN_GAP.as_millis() as u64;
        if let Some(last) = self.attempts_ms.back() {
            if now_ms.saturating_sub(*last) < gap {
                return Some(Held::CapUntil(last + gap));
            }
        }
        let in_hour: Vec<u64> =
            self.attempts_ms.iter().copied().filter(|t| now_ms.saturating_sub(*t) < ONE_HOUR_MS).collect();
        if in_hour.len() >= UPLOADS_PER_HOUR_MAX {
            return Some(Held::CapUntil(in_hour[0] + ONE_HOUR_MS));
        }
        let in_day: Vec<u64> =
            self.attempts_ms.iter().copied().filter(|t| now_ms.saturating_sub(*t) < ONE_DAY_MS).collect();
        if in_day.len() >= UPLOADS_PER_DAY_MAX {
            return Some(Held::CapUntil(in_day[0] + ONE_DAY_MS));
        }
        None
    }
}

#[async_trait]
impl DeviceAdapter for RingLightAdapter {
    fn name(&self) -> &'static str {
        "eightbitdo-ultimate-2-rings"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities::default()
    }

    /// The channel is the pad adapter's: there is nothing to open here.
    async fn connect(&mut self) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.handle.is_open()
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        if self.handle.pad_present() || self.handle.is_open() {
            Ok(vec![self.device_info()])
        } else {
            Ok(Vec::new())
        }
    }

    /// Takes a frame of 24 colours. Sends the pad nothing unless the set changed and every gate
    /// allows it (see the module doc); a frame that is held returns `Ok` and the next one tries
    /// again.
    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
        self.check_id(device)?;
        let frame: &[Rgb8; RING_LEDS] = colors.try_into().map_err(|_| AdapterError::Zone {
            device: device.clone(),
            zone: 0,
            reason: format!("a ring frame is {RING_LEDS} colours, this one is {}", colors.len()),
        })?;
        let words = pack_ring_colours(frame);
        let hash = frame_hash(&words);
        let now_ms = self.clock.unix_ms();
        self.load_stored(now_ms);
        if self.shown == Some(hash) {
            self.status.held = None;
            return Ok(());
        }
        if let Some(held) = self.held_at(now_ms) {
            self.status.held = Some(held);
            return Ok(());
        }
        self.status.held = None;

        // The attempt is counted, and on disk, before its first chunk goes out; the stored hash
        // is cleared with it, so a crash in the middle leaves no hash to trust.
        self.attempts_ms.push_back(now_ms);
        self.shown = None;
        if self.persist(None).is_err() {
            self.attempts_ms.pop_back();
            self.status.held = Some(Held::StoreDead);
            return Ok(());
        }
        self.status.last_sent_ms = Some(now_ms);
        self.status.complete = false;

        let handle = self.handle.clone();
        let pace = self.pace;
        let outcome = tokio::task::spawn_blocking(move || {
            handle.with(|v| write_still(v, pace, &words))
        })
        .await
        .map_err(|e| refuse(format!("the ring write task failed: {e}")))?;
        match outcome {
            None => {
                // The channel closed between the check and the write: nothing was sent.
                self.status.held = Some(Held::ChannelClosed);
                self.attempts_ms.pop_back();
                let _ = self.persist(None);
                Ok(())
            }
            Some(Ok(())) => {
                self.shown = Some(hash);
                self.shown_colors = colors.to_vec();
                self.status.complete = true;
                let _ = self.persist(Some(hash));
                Ok(())
            }
            Some(Err(e)) => Err(e),
        }
    }

    async fn set_zone_size(&mut self, device: &DeviceId, _zone: u32, _leds: u32) -> Result<(), AdapterError> {
        self.check_id(device)?;
        Err(AdapterError::Unsupported("the rings have a fixed 12 LEDs each"))
    }

    async fn identify(&mut self, device: &DeviceId, _zone: u32, _led: u32) -> Result<(), AdapterError> {
        self.check_id(device)?;
        Err(AdapterError::Unsupported("the rings take stills only; a flash of one LED would write the pad"))
    }

    /// The colours last written whole, never what the rings show: the light cannot be seen from
    /// here.
    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        self.check_id(device)?;
        Ok(self.shown_colors.clone())
    }
}

impl crate::resume::ResumeBehavior for RingLightAdapter {
    fn on_resume(&self) -> crate::resume::OnResume {
        // The rings hold their colours in the pad's flash; a sleep sets no mode again and loses
        // nothing the next scene change would not rewrite.
        crate::resume::OnResume::NothingToRestore
    }
}

// ------------------------------------------------------------------------------------ the write

/// Bytes of one custom theme record: the flag word and 24 colour words.
pub(super) const SINGLE_LEN: usize = 0x64;

/// What a write needs to know before the first chunk goes out.
pub(super) struct Prepared {
    /// The image as the pad held it: the save to restore from.
    pub saved: Vec<u8>,
    /// The active profile (0-based).
    pub active: usize,
    /// Where the active profile's custom ring record starts in the image.
    pub at: usize,
    /// The record as it will be written: the flag and the new colour words.
    pub record: Vec<u8>,
}

impl Prepared {
    /// True when the pad already holds this record.
    pub(super) fn is_noop(&self) -> bool {
        self.record == self.saved[self.at..self.at + SINGLE_LEN]
    }

    /// The image the pad should read back as once the record is written.
    pub(super) fn expected(&self) -> Vec<u8> {
        let mut expected = self.saved.clone();
        expected[self.at..self.at + SINGLE_LEN].copy_from_slice(&self.record);
        expected
    }
}

/// Checks a saved image and builds the record to write. Refuses, with nothing sent, unless the
/// image names an active profile whose custom ring record is marked in use, and (when
/// `expect_profile` is given) unless that profile is the expected one.
pub(super) fn prepare_from(
    saved: Vec<u8>,
    words: &[[u8; 4]; CUSTOM_COLOURS],
    expect_profile: Option<usize>,
) -> Result<Prepared, AdapterError> {
    let Some(active) = codec::active_profile(&saved) else {
        return Err(refuse("the pad's image names no active profile; nothing was written"));
    };
    if let Some(expected) = expect_profile {
        if expected != active {
            return Err(refuse(format!(
                "the pad's active profile is {}, not {}; nothing was written",
                active + 1,
                expected + 1
            )));
        }
    }
    let at = codec::SINGLE + SINGLE_LEN * active;
    let flag = u32::from_le_bytes([saved[at], saved[at + 1], saved[at + 2], saved[at + 3]]);
    if flag != codec::ENABLED {
        return Err(refuse(
            "the active profile's custom ring record is not marked in use on the pad, so it is not \
             written (the rings may not be set to Custom); nothing was written",
        ));
    }
    let mut record = saved[at..at + SINGLE_LEN].to_vec();
    for (i, word) in words.iter().enumerate() {
        record[4 + 4 * i..8 + 4 * i].copy_from_slice(word);
    }
    Ok(Prepared { saved, active, at, record })
}

/// Writes the colour set into the active profile's custom ring record, checking before and after.
/// Blocking, with the channel to itself.
fn write_still(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    words: &[[u8; 4]; CUSTOM_COLOURS],
) -> Result<(), AdapterError> {
    let prepared = prepare_from(read_image(vendor, pace)?, words, None)?;
    if prepared.is_noop() {
        // The pad already holds this set: nothing to send beyond the reads.
        return Ok(());
    }
    put_and_verify(vendor, pace, &prepared)
}

/// Writes the prepared record in one session, reads the image back and compares it with what it
/// should be. On any failure the saved bytes are written back (see [`restore_exact`]) and the
/// error says what happened to them.
pub(super) fn put_and_verify(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    prepared: &Prepared,
) -> Result<(), AdapterError> {
    if let Err(first) = put_bytes(vendor, pace, prepared.at, &prepared.record) {
        let restored = restore(vendor, pace, &prepared.saved);
        return Err(refuse(format!("{first}; {restored}")));
    }
    let expected = prepared.expected();
    let after = match read_image(vendor, pace) {
        Ok(img) => img,
        Err(e) => {
            let restored = restore(vendor, pace, &prepared.saved);
            return Err(refuse(format!("the image could not be read back ({e}); {restored}")));
        }
    };
    if after != expected {
        let diff = diff_summary(&expected, &after);
        let restored = restore(vendor, pace, &prepared.saved);
        return Err(refuse(format!(
            "the pad's image did not read back as written ({diff}); {restored}"
        )));
    }
    Ok(())
}

/// One session that writes `bytes` at image offset `at` in chunks and commits.
pub(super) fn put_bytes(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    at: usize,
    bytes: &[u8],
) -> Result<(), AdapterError> {
    let mut packets = Vec::new();
    for (n, chunk) in bytes.chunks(codec::CHUNK).enumerate() {
        packets.push(codec::write_request(at + n * codec::CHUNK, chunk).map_err(refuse)?);
    }
    packets.push(codec::commit_request());
    write_session(vendor, pace, &packets)
}

/// Where two images differ: the first few differing ranges and the count of differing bytes.
pub(super) fn diff_summary(expected: &[u8], got: &[u8]) -> String {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut count = 0;
    for (i, (a, b)) in expected.iter().zip(got.iter()).enumerate() {
        if a != b {
            count += 1;
            match ranges.last_mut() {
                Some((_, end)) if *end + 1 == i => *end = i,
                _ => ranges.push((i, i)),
            }
        }
    }
    if expected.len() != got.len() {
        return format!("lengths differ: {} and {} bytes", expected.len(), got.len());
    }
    let shown: Vec<String> = ranges
        .iter()
        .take(6)
        .map(|(a, b)| if a == b { format!("{a:#05x}") } else { format!("{a:#05x}..{b:#05x}") })
        .collect();
    format!(
        "{count} byte(s) differ at {}{}",
        shown.join(", "),
        if ranges.len() > 6 { ", ..." } else { "" }
    )
}

/// How an exact restore went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restored {
    /// The pad already held the saved image: nothing needed writing.
    AlreadyMatched,
    /// The bytes from the first to the last difference were written back and the image then read
    /// back as saved.
    Range,
    /// The whole saved image was written back and the image then read back as saved.
    Whole,
}

/// Puts the pad's image back to `saved`, byte for byte, and proves it by reading the image again.
///
/// It reads the pad first and writes only the span from the first differing byte to the last, so
/// an image that is already right is not written at all. A write session that fails is not the
/// end: the image is read again, and if it reads back as saved the restore is done (a lost
/// acknowledgement does not make the bytes wrong). If the span does not do it, every byte is
/// written back. An image that still differs after that is an error.
pub(super) fn restore_exact(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    saved: &[u8],
) -> Result<Restored, AdapterError> {
    if saved.len() != codec::IMAGE_LEN {
        return Err(refuse(format!("the saved image is {} bytes, not {}", saved.len(), codec::IMAGE_LEN)));
    }
    let now = read_image(vendor, pace)?;
    if now == saved {
        return Ok(Restored::AlreadyMatched);
    }
    let first = now.iter().zip(saved).position(|(a, b)| a != b).unwrap_or(0);
    let last = now.iter().zip(saved).rposition(|(a, b)| a != b).unwrap_or(first);
    let ranged = put_bytes(vendor, pace, first, &saved[first..=last]);
    match read_image(vendor, pace) {
        Ok(img) if img == saved => return Ok(Restored::Range),
        Ok(_) => {}
        Err(e) => {
            return Err(refuse(format!(
                "the saved bytes were written back but the image could not be read to check it ({e})"
            )))
        }
    }
    let whole = put_bytes(vendor, pace, 0, saved);
    match read_image(vendor, pace) {
        Ok(img) if img == saved => Ok(Restored::Whole),
        Ok(img) => Err(refuse(format!(
            "the image still differs from the saved one after a full write-back ({}; first try: {:?}, \
             second try: {:?})",
            diff_summary(saved, &img),
            ranged.err().map(|e| e.to_string()),
            whole.err().map(|e| e.to_string())
        ))),
        Err(e) => Err(refuse(format!(
            "the saved image was written back but could not be read to check it ({e})"
        ))),
    }
}

/// [`restore_exact`], as a sentence for an error message.
fn restore(vendor: &mut dyn VendorTransport, pace: Pace, saved: &[u8]) -> String {
    match restore_exact(vendor, pace, saved) {
        Ok(Restored::AlreadyMatched) => {
            "the image already read back as saved; nothing needed writing back".to_string()
        }
        Ok(Restored::Range) | Ok(Restored::Whole) => {
            "the saved bytes were written back and the image read back as saved".to_string()
        }
        Err(e) => format!("the saved bytes could not be confirmed back ({e}): the rings may be partly written"),
    }
}

// ----------------------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::eightbitdo_ultimate2::{Opener, Personality};
    use crate::fake_pad::{self, FakePad};

    struct FakeClock(AtomicU64);

    impl FakeClock {
        fn at(ms: u64) -> Arc<Self> {
            Arc::new(Self(AtomicU64::new(ms)))
        }
        fn advance(&self, ms: u64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }

    impl RingClock for FakeClock {
        fn unix_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[derive(Clone, Default)]
    struct MemStore(Arc<Mutex<Option<StoredStills>>>, Arc<Mutex<bool>>);

    impl MemStore {
        fn with(stored: StoredStills) -> Self {
            Self(Arc::new(Mutex::new(Some(stored))), Arc::default())
        }
        fn saved(&self) -> Option<StoredStills> {
            self.0.lock().unwrap().clone()
        }
        fn read_only(stored: StoredStills) -> Self {
            Self(Arc::new(Mutex::new(Some(stored))), Arc::new(Mutex::new(true)))
        }
    }

    impl StillStore for MemStore {
        fn load(&self) -> Result<Option<StoredStills>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, stored: &StoredStills) -> Result<(), String> {
            if *self.1.lock().unwrap() {
                return Err("the disk is full".into());
            }
            *self.0.lock().unwrap() = Some(stored.clone());
            Ok(())
        }
    }

    const T0: u64 = 1_000_000_000_000;

    struct Rig {
        pad: FakePad,
        handle: VendorHandle,
        adapter: RingLightAdapter,
        clock: Arc<FakeClock>,
        store: MemStore,
        idle: Arc<AtomicU64>,
    }

    /// A ring adapter over an open fake pad: the gate on, the person away for a minute.
    async fn rig() -> Rig {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let handle = VendorHandle::new();
        let transport = pad.open_vendor(Personality::XInput).unwrap().unwrap();
        handle.open_with(transport);
        handle.set_pad_present(true);
        handle.set_writes_enabled(true);
        let idle = Arc::new(AtomicU64::new(60));
        let idle_clone = idle.clone();
        handle.set_idle_source(Arc::new(move || Duration::from_secs(idle_clone.load(Ordering::SeqCst))));
        let clock = FakeClock::at(T0);
        let store = MemStore::with(StoredStills::default());
        let adapter = RingLightAdapter::new(handle.clone(), Box::new(store.clone()))
            .with_clock(clock.clone())
            .without_pauses();
        Rig { pad, handle, adapter, clock, store, idle }
    }

    fn solid(r: u8, g: u8, b: u8) -> Vec<Rgb8> {
        vec![Rgb8::new(r, g, b); RING_LEDS]
    }

    fn counts(pad: &FakePad) -> (usize, usize, usize, usize) {
        pad.report_counts()
    }

    /// The custom record of the fixture's active profile (profile 3) as the pad holds it.
    fn record_of(image: &[u8]) -> Vec<u8> {
        let at = codec::SINGLE + SINGLE_LEN * 2;
        image[at..at + SINGLE_LEN].to_vec()
    }

    #[tokio::test]
    async fn the_device_is_two_zones_of_twelve_that_cannot_be_resized() {
        let mut r = rig().await;
        let d = r.adapter.discover().await.unwrap();
        assert_eq!(d.len(), 1);
        let d = &d[0];
        assert_eq!(d.id, DeviceId("ring:8bitdo-ultimate-2".into()));
        assert_eq!(d.kind, DeviceKind::Other);
        assert_eq!(d.led_count(), 24);
        assert_eq!(d.zones.len(), 2);
        for z in &d.zones {
            assert_eq!((z.led_count, z.min_leds, z.max_leds, z.resizable), (12, 12, 12, false));
            assert_eq!(z.kind, ZoneKind::Linear);
        }
        let err = r.adapter.set_zone_size(&d.id, 0, 8).await.unwrap_err();
        assert!(matches!(err, AdapterError::Unsupported(_)));
        // No pad, no device.
        r.handle.close();
        r.handle.set_pad_present(false);
        assert!(r.adapter.discover().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_still_is_written_to_the_active_profile_and_the_pad_reads_back_what_was_sent() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.adapter.write_frame(&id, &solid(255, 0, 0)).await.unwrap();
        let (reads, writes, commits, states) = counts(&r.pad);
        assert_eq!(reads, 72, "the image is read before and read back after: 2 x 36");
        assert_eq!(writes, 3, "the 100-byte record is three chunks");
        assert_eq!(commits, 1);
        assert_eq!(states, 2, "Reportstate(0) before and Reportstate(1) after");
        let image = r.pad.image();
        let record = record_of(&image);
        assert_eq!(&record[..4], &codec::ENABLED.to_le_bytes(), "the flag stays marked in use");
        // Pure red, in the order the mapping function says; the test names no byte order itself.
        let expected = pack_ring_colours(&[Rgb8::new(255, 0, 0); RING_LEDS]);
        for (i, word) in expected.iter().enumerate() {
            assert_eq!(&record[4 + 4 * i..8 + 4 * i], word, "word {i}");
        }
        // Nothing outside the active profile's custom record changed.
        let before = fake_pad::ring_fixture_image();
        let at = codec::SINGLE + SINGLE_LEN * 2;
        assert_eq!(&image[..at], &before[..at]);
        assert_eq!(&image[at + SINGLE_LEN..], &before[at + SINGLE_LEN..]);
        let status = r.adapter.status();
        assert!(status.complete);
        assert_eq!(status.last_sent_ms, Some(T0));
        assert_eq!(status.held, None);
        assert_eq!(r.adapter.read_back(&id).await.unwrap(), solid(255, 0, 0));
    }

    #[tokio::test]
    async fn the_same_set_sends_nothing_however_many_frames_arrive() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.adapter.write_frame(&id, &solid(0, 0, 255)).await.unwrap();
        let after_first = counts(&r.pad);
        for _ in 0..50 {
            r.clock.advance(1000);
            r.adapter.write_frame(&id, &solid(0, 0, 255)).await.unwrap();
        }
        assert_eq!(counts(&r.pad), after_first, "a hash match writes nothing");
    }

    #[tokio::test]
    async fn with_the_write_gate_off_a_scene_change_sends_nothing_at_all() {
        let mut r = rig().await;
        r.handle.set_writes_enabled(false);
        let id = ring_device_id();
        for n in 0..5u8 {
            r.clock.advance(60_000);
            r.adapter.write_frame(&id, &solid(n, 0, 0)).await.unwrap();
        }
        assert!(r.pad.sent_reports().is_empty(), "gate off: not one report, not even a read");
        assert_eq!(r.adapter.status().held, Some(Held::WritesOff));
        assert_eq!(r.store.saved(), Some(StoredStills::default()), "no attempt was counted either");
        // Turn it on: the next frame goes out.
        r.handle.set_writes_enabled(true);
        r.adapter.write_frame(&id, &solid(9, 0, 0)).await.unwrap();
        assert_eq!(counts(&r.pad).1, 3);
    }

    #[tokio::test]
    async fn the_person_at_the_keyboard_holds_the_write_and_the_next_frame_after_the_quiet_sends_it() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.idle.store(3, Ordering::SeqCst);
        r.adapter.write_frame(&id, &solid(1, 2, 3)).await.unwrap();
        assert!(r.pad.sent_reports().is_empty(), "idle 3 s of the 10 s the rule needs");
        assert_eq!(r.adapter.status().held, Some(Held::NotIdle));
        r.idle.store(10, Ordering::SeqCst);
        r.adapter.write_frame(&id, &solid(1, 2, 3)).await.unwrap();
        assert_eq!(counts(&r.pad).2, 1, "the held set went out once the person was away");
    }

    #[tokio::test]
    async fn without_an_idle_clock_or_with_the_channel_closed_nothing_is_sent() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let handle = VendorHandle::new();
        handle.set_writes_enabled(true);
        let mut a = RingLightAdapter::without_store(handle.clone()).without_pauses();
        let id = ring_device_id();
        // Closed channel.
        a.write_frame(&id, &solid(5, 5, 5)).await.unwrap();
        assert_eq!(a.status().held, Some(Held::ChannelClosed));
        // Open, but no idle clock to say the person is away.
        handle.open_with(pad.open_vendor(Personality::XInput).unwrap().unwrap());
        a.write_frame(&id, &solid(5, 5, 5)).await.unwrap();
        assert_eq!(a.status().held, Some(Held::NotIdle));
        assert!(pad.sent_reports().is_empty());
    }

    #[tokio::test]
    async fn the_gaps_and_caps_hold_the_fifth_scene_change_in_an_hour() {
        let mut r = rig().await;
        let id = ring_device_id();
        for n in 0..UPLOADS_PER_HOUR_MAX as u8 {
            r.adapter.write_frame(&id, &solid(10 + n, 0, 0)).await.unwrap();
            assert!(r.adapter.status().complete, "write {n}");
            // Just past the 10 s gap: the hour fills at 4 attempts.
            r.clock.advance(UPLOAD_MIN_GAP.as_millis() as u64 + 1);
        }
        let sent = counts(&r.pad).2;
        assert_eq!(sent, 4);
        r.adapter.write_frame(&id, &solid(99, 0, 0)).await.unwrap();
        assert_eq!(counts(&r.pad).2, sent, "the fifth change inside the hour is refused");
        let Some(Held::CapUntil(until)) = r.adapter.status().held else {
            panic!("expected a cap, got {:?}", r.adapter.status().held)
        };
        assert_eq!(until, T0 + ONE_HOUR_MS, "the first attempt's slot frees an hour after it");
        // When the slot frees, the held set goes out.
        r.clock.advance(ONE_HOUR_MS);
        r.adapter.write_frame(&id, &solid(99, 0, 0)).await.unwrap();
        assert_eq!(counts(&r.pad).2, sent + 1);
    }

    #[tokio::test]
    async fn two_changes_ten_seconds_apart_are_not_both_written() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.adapter.write_frame(&id, &solid(1, 0, 0)).await.unwrap();
        r.clock.advance(4_000);
        r.adapter.write_frame(&id, &solid(2, 0, 0)).await.unwrap();
        assert_eq!(counts(&r.pad).2, 1, "the second change is inside the 10 s gap");
        assert_eq!(r.adapter.status().held, Some(Held::CapUntil(T0 + 10_000)));
    }

    #[tokio::test]
    async fn the_day_cap_holds_after_twenty_four_attempts_even_when_the_hour_is_clear() {
        let day = (0..UPLOADS_PER_DAY_MAX as u64).map(|n| T0 - 10 * n * 3_600_000 / 4).collect::<Vec<_>>();
        // 24 attempts spread over the last 6 hours: every hour holds fewer than 4, the day is full.
        let mut sorted = day.clone();
        sorted.sort_unstable();
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let handle = VendorHandle::new();
        handle.open_with(pad.open_vendor(Personality::XInput).unwrap().unwrap());
        handle.set_writes_enabled(true);
        handle.set_idle_source(Arc::new(|| Duration::from_secs(60)));
        let store = MemStore::with(StoredStills { attempts_ms: sorted, picture: None, modes_13_ms: vec![] });
        let mut a = RingLightAdapter::new(handle, Box::new(store))
            .with_clock(FakeClock::at(T0))
            .without_pauses();
        a.write_frame(&ring_device_id(), &solid(7, 7, 7)).await.unwrap();
        assert!(pad.sent_reports().is_empty());
        assert!(matches!(a.status().held, Some(Held::CapUntil(_))), "{:?}", a.status().held);
    }

    #[tokio::test]
    async fn the_attempt_is_on_the_disk_before_the_first_chunk_and_the_hash_after_the_last() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.adapter.write_frame(&id, &solid(0, 200, 0)).await.unwrap();
        let stored = r.store.saved().unwrap();
        assert_eq!(stored.attempts_ms, vec![T0]);
        let expected = frame_hash(&pack_ring_colours(&[Rgb8::new(0, 200, 0); RING_LEDS]));
        assert_eq!(stored.picture, Some(expected), "the hash is stored once the write finished whole");
        // A restart with that store does not rewrite the set the pad holds.
        let pad2 = FakePad::with_image(r.pad.image(), Personality::XInput);
        let handle2 = VendorHandle::new();
        handle2.open_with(pad2.open_vendor(Personality::XInput).unwrap().unwrap());
        handle2.set_writes_enabled(true);
        handle2.set_idle_source(Arc::new(|| Duration::from_secs(60)));
        let mut again = RingLightAdapter::new(handle2, Box::new(r.store.clone()))
            .with_clock(r.clock.clone())
            .without_pauses();
        r.clock.advance(1_000_000);
        again.write_frame(&id, &solid(0, 200, 0)).await.unwrap();
        assert!(pad2.sent_reports().is_empty(), "same hash after a restart: nothing sent");
    }

    #[tokio::test]
    async fn a_failed_commit_clears_the_hash_so_the_next_change_writes_in_full() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.pad.silence_commits();
        let err = r.adapter.write_frame(&id, &solid(0, 0, 255)).await.unwrap_err();
        assert!(err.to_string().contains("commit"), "{err}");
        assert!(!r.adapter.status().complete, "the rings may be partly written");
        let stored = r.store.saved().unwrap();
        assert_eq!(stored.picture, None, "no hash is kept for a write that did not finish");
        assert_eq!(stored.attempts_ms.len(), 1, "the attempt still counts");
        // The saved record was written back after the failure.
        assert_eq!(r.pad.image(), fake_pad::ring_fixture_image());
        // Past the gap, the same frame is tried again in full: the hash was not kept.
        let before = counts(&r.pad);
        r.clock.advance(11_000);
        let _ = r.adapter.write_frame(&id, &solid(0, 0, 255)).await;
        assert!(counts(&r.pad).1 > before.1, "the next frame wrote again");
    }

    #[tokio::test]
    async fn a_pad_that_changes_a_byte_outside_the_ring_block_is_restored_and_the_hash_is_not_kept() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.pad.scribble_on_next_commit(codec::NAME);
        let err = r.adapter.write_frame(&id, &solid(255, 255, 255)).await.unwrap_err();
        let text = err.to_string();
        assert!(text.contains("did not read back as written"), "{text}");
        assert!(text.contains("read back as saved"), "{text}");
        assert_eq!(r.store.saved().unwrap().picture, None);
        assert!(!r.adapter.status().complete);
        assert_eq!(
            r.pad.image(),
            fake_pad::ring_fixture_image(),
            "every byte is back as it was saved, the stray one included"
        );
    }

    #[tokio::test]
    async fn a_pad_that_drops_the_write_is_caught_by_the_read_back() {
        let mut r = rig().await;
        let id = ring_device_id();
        r.pad.drop_next_commit();
        let err = r.adapter.write_frame(&id, &solid(255, 0, 255)).await.unwrap_err();
        assert!(err.to_string().contains("did not read back as written"), "{err}");
        assert_eq!(r.store.saved().unwrap().picture, None);
    }

    #[tokio::test]
    async fn a_profile_whose_custom_ring_record_is_not_in_use_is_never_written() {
        let mut image = fake_pad::ring_fixture_image();
        let at = codec::SINGLE + SINGLE_LEN * 2;
        image[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
        let pad = FakePad::with_image(image, Personality::XInput);
        let handle = VendorHandle::new();
        handle.open_with(pad.open_vendor(Personality::XInput).unwrap().unwrap());
        handle.set_writes_enabled(true);
        handle.set_idle_source(Arc::new(|| Duration::from_secs(60)));
        let mut a = RingLightAdapter::without_store(handle).without_pauses();
        let err = a.write_frame(&ring_device_id(), &solid(4, 4, 4)).await.unwrap_err();
        assert!(err.to_string().contains("not marked in use"), "{err}");
        let (reads, writes, commits, states) = pad.report_counts();
        assert_eq!((writes, commits, states), (0, 0, 0), "only the read ran");
        assert_eq!(reads, 36);
    }

    #[tokio::test]
    async fn a_missing_store_holds_writes_for_ten_minutes_and_a_broken_one_for_a_day() {
        // Missing file.
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let handle = VendorHandle::new();
        handle.open_with(pad.open_vendor(Personality::XInput).unwrap().unwrap());
        handle.set_writes_enabled(true);
        handle.set_idle_source(Arc::new(|| Duration::from_secs(60)));
        let clock = FakeClock::at(T0);
        let empty = MemStore::default();
        let mut a = RingLightAdapter::new(handle.clone(), Box::new(empty))
            .with_clock(clock.clone())
            .without_pauses();
        let id = ring_device_id();
        a.write_frame(&id, &solid(8, 8, 8)).await.unwrap();
        assert!(pad.sent_reports().is_empty());
        assert_eq!(a.status().held, Some(Held::StoreHoldUntil(T0 + 600_000)));
        clock.advance(600_001);
        a.write_frame(&id, &solid(8, 8, 8)).await.unwrap();
        assert_eq!(pad.report_counts().2, 1, "the hold-off ended");
        // A store whose newest stamp is more than a day ahead has lost its history.
        let broken = MemStore::with(StoredStills {
            attempts_ms: vec![T0 + 3 * ONE_DAY_MS],
            picture: None,
            modes_13_ms: vec![],
        });
        let pad2 = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let handle2 = VendorHandle::new();
        handle2.open_with(pad2.open_vendor(Personality::XInput).unwrap().unwrap());
        handle2.set_writes_enabled(true);
        handle2.set_idle_source(Arc::new(|| Duration::from_secs(60)));
        let mut b = RingLightAdapter::new(handle2, Box::new(broken))
            .with_clock(FakeClock::at(T0))
            .without_pauses();
        b.write_frame(&id, &solid(8, 8, 8)).await.unwrap();
        assert!(pad2.sent_reports().is_empty());
        assert_eq!(b.status().held, Some(Held::StoreHoldUntil(T0 + 86_400_000)));
    }

    #[tokio::test]
    async fn a_store_that_cannot_be_written_stops_every_write_and_nothing_reaches_the_pad() {
        let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
        let handle = VendorHandle::new();
        handle.open_with(pad.open_vendor(Personality::XInput).unwrap().unwrap());
        handle.set_writes_enabled(true);
        handle.set_idle_source(Arc::new(|| Duration::from_secs(60)));
        let store = MemStore::read_only(StoredStills::default());
        let mut a = RingLightAdapter::new(handle, Box::new(store))
            .with_clock(FakeClock::at(T0))
            .without_pauses();
        a.write_frame(&ring_device_id(), &solid(3, 3, 3)).await.unwrap();
        assert!(pad.sent_reports().is_empty(), "an attempt that is not on the disk never reaches the pad");
        assert_eq!(a.status().held, Some(Held::StoreDead));
    }

    #[tokio::test]
    async fn a_frame_of_the_wrong_length_is_refused_and_an_unknown_device_too() {
        let mut r = rig().await;
        let err = r.adapter.write_frame(&ring_device_id(), &[Rgb8::BLACK; 5]).await.unwrap_err();
        assert!(matches!(err, AdapterError::Zone { .. }));
        let err = r.adapter.write_frame(&DeviceId("ring:other".into()), &solid(0, 0, 0)).await.unwrap_err();
        assert!(matches!(err, AdapterError::UnknownDevice(_)));
        let err = r.adapter.identify(&ring_device_id(), 0, 0).await.unwrap_err();
        assert!(matches!(err, AdapterError::Unsupported(_)));
        assert!(r.pad.sent_reports().is_empty());
    }

    #[test]
    fn the_mapping_is_the_only_function_that_knows_the_byte_and_led_order() {
        // The test pins the byte order (confirmed) and the LED order (TODO, the marker run) in one place, so the day the marker run settles the LED order,
        // this is the test that changes with `pack_ring_colours` and nothing else does.
        let mut frame = [Rgb8::BLACK; RING_LEDS];
        frame[0] = Rgb8::new(1, 2, 3);
        frame[12] = Rgb8::new(4, 5, 6);
        let words = pack_ring_colours(&frame);
        assert_eq!(words[0], [3, 2, 1, 0], "CONFIRMED by S3: B, G, R, pad");
        assert_eq!(words[12], [6, 5, 4, 0], "UNCONFIRMED: zone 1 is words 12-23");
        assert_eq!(words[1], [0, 0, 0, 0]);
    }

    #[test]
    fn pure_red_packs_as_00_00_ff_00_the_order_s3_confirmed_on_the_pad() {
        // S3 (2026-11-18): `ring_s3 --order bgr` wrote 24 words of 00 00 ff 00 and both rings
        // showed red. This pins it: a change to the byte order fails here.
        let red = [Rgb8::new(255, 0, 0); RING_LEDS];
        for word in pack_ring_colours(&red) {
            assert_eq!(word, [0x00, 0x00, 0xFF, 0x00]);
        }
        assert_eq!(pack_ring_colours_as(&red, ByteOrder::Bgr), pack_ring_colours(&red));
        assert_eq!(DEFAULT_ORDER, ByteOrder::Bgr);
        // And the frame a scene sends reaches the pad in that order end to end: green and blue.
        assert_eq!(pack_ring_colours(&[Rgb8::new(0, 255, 0); RING_LEDS])[5], [0x00, 0xFF, 0x00, 0x00]);
        assert_eq!(pack_ring_colours(&[Rgb8::new(0, 0, 255); RING_LEDS])[23], [0xFF, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn the_idle_gate_is_the_same_ten_seconds_the_peripheral_reads_wait_for() {
        assert_eq!(IDLE_BEFORE_WRITE, Duration::from_secs(10));
    }
}
