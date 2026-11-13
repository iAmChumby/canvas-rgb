//! Native adapter for the ROYUAN MonsGeek keyboard (USB 3151:4015).
//!
//! The board is driven over its vendor HID interface (interface 2, usage page 0xFFFF, usage
//! 0x0002) with 65-byte feature reports. The key-press interfaces (0 and 1) are never opened.
//!
//! Two paths light it (#312). While a frame moves, the keyboard shows one colour for the whole
//! board: the frame's average, streamed as the single `0x0E` report. When the frame holds still
//! for the still window ([`STILL_AFTER`] at first), the adapter uploads it once as a stored
//! picture (`SET_USERPIC`, seven pages 100 ms apart, then light mode 13) and stops sending the
//! whole-board colour; the keys then show their own colours. A moving frame sends the
//! direct-mode report and the whole-board colour again, at once.
//!
//! A picture is a flash write, so the limits below keep the flash from wearing out (#312's
//! review):
//!
//! - the picture already in the flash is shown again with light mode 13, and not written again,
//!   when the new still is within [`STILL_TOLERANCE`] of it;
//! - once an upload starts it finishes, whatever the frame does: a half-written picture would
//!   only leave the flash torn. A frame that moves ends a picture *on show*, and every one of
//!   those abandons doubles the still window, up to [`STILL_AFTER_MAX`]; a picture that stays a
//!   minute puts it back to [`STILL_AFTER`];
//! - an attempt counts from before its first page goes out, and no more than
//!   [`UPLOADS_PER_HOUR_MAX`] attempts go out in a rolling hour or [`UPLOADS_PER_DAY_MAX`] in a
//!   rolling day, about 8.8k a year. Past that, a still keeps the whole-board colour until a
//!   slot frees. A board that refuses every page is counted the same way.
//!
//! The attempts and the picture's hash live in a small JSON file under the daemon's user
//! directory ([`StillStore`], read at start and written on every attempt), so a restart cannot
//! reset the caps or rewrite a picture the flash already holds. A file that is missing or will
//! not parse holds every upload off for [`STORE_HOLD_OFF`] and leaves the picture unknown: the
//! counters fail closed, never open.

use async_trait::async_trait;
use canvas_proto::color::Rgb8;
use canvas_proto::device::{AdapterCapabilities, DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::traits::{AdapterError, DeviceAdapter};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const VENDOR_ID: u16 = 0x3151;
pub const PRODUCT_ID: u16 = 0x4015;
const INTERFACE: i32 = 2;
const USAGE_PAGE: u16 = 0xFFFF;
const USAGE: u16 = 0x0002;
const REPORT_LEN: usize = 65;
/// Re-send the same colour at least this often.
const KEEPALIVE: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------- stills (#312)
//
// A stored picture is a flash write. The cadences that survived on a yc500 board are pages
// 100 ms apart and one upload per 10 s (docs/adapters.md, "Frame rate"), so the picture is only
// sent when the effect holds still; a frame that keeps moving stays on the whole-board colour.

/// How long the per-key frame must stay put before it is uploaded as a stored picture.
///
/// This is the window at the start; it doubles each time a shown picture is abandoned soon
/// after it appeared (see [`ABANDON_FAST`]), up to [`STILL_AFTER_MAX`].
pub const STILL_AFTER: Duration = Duration::from_secs(2);
/// The longest the still window grows to.
///
/// At 10 minutes a slow effect can still upload six pictures an hour, which is under
/// [`UPLOADS_PER_HOUR_MAX`]; past that the window would swallow a genuinely static frame.
pub const STILL_AFTER_MAX: Duration = Duration::from_secs(600);
/// A shown picture the frame moves away from sooner than this doubles the still window: the
/// effect is changing faster than a flash write can follow, so each write buys less.
const ABANDON_FAST: Duration = Duration::from_secs(30);
/// A picture that stays this long resets the still window to [`STILL_AFTER`]: the effect has
/// settled and the next still is worth its write.
const STILL_LONG: Duration = Duration::from_secs(60);
/// The shortest gap between two upload attempts; a still that changes sooner waits.
///
/// An attempt counts from before its first page goes out, so a refused page still uses the gap.
pub const UPLOAD_MIN_GAP: Duration = Duration::from_secs(10);
/// The most upload attempts in any rolling hour.
///
/// This, the daily cap and the still window bound the flash's erase cycles. A picture is one
/// erase of one 384-byte region, not 384 bytes of wear: the flash is rated for about 100k
/// erase cycles per sector, and at this cap a pathological effect reaches 4 an hour, 24 a day,
/// about 8.8k a year. A normal effect writes a handful a day: the window alone needs a still to
/// hold 2 s, doubling to 10 minutes as pictures keep being abandoned, and a still that holds a
/// minute puts it back.
pub const UPLOADS_PER_HOUR_MAX: usize = 4;
/// The most upload attempts in any rolling 24 hours (about 8.8k a year at the ceiling).
pub const UPLOADS_PER_DAY_MAX: usize = 24;
/// How long a store that cannot be read holds every upload off, so a missing or broken file
/// cannot buy attempts (#312 review: the counters fail closed, never open).
pub const STORE_HOLD_OFF: Duration = Duration::from_secs(600);
/// The gap between two pages of one upload.
const PAGE_GAP: Duration = Duration::from_millis(100);
/// The settle after the last page before light mode 13, as in the #311 device check.
const PICTURE_SETTLE: Duration = Duration::from_secs(2);
/// Two frames within this many steps in every channel are the same still.
///
/// It is also how close a new still has to be to the picture in the flash for light mode 13 to
/// show that picture instead of writing a new one: a dim breathing effect is quantised to a
/// step or two, so its peaks are the same picture.
const STILL_TOLERANCE: u8 = 1;
/// A rolling hour, for [`UPLOADS_PER_HOUR_MAX`].
const ONE_HOUR: Duration = Duration::from_secs(3_600);
/// A rolling day, for [`UPLOADS_PER_DAY_MAX`] and the stored attempts.
const ONE_DAY: Duration = Duration::from_secs(86_400);

/// The report that switches the keyboard to direct (host-driven) lighting.
pub fn direct_mode_report() -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[1..10].copy_from_slice(&[0x07, 0x15, 0x04, 0x04, 0x07, 0x00, 0x00, 0x00, 0xD4]);
    r
}

/// The report that sets the whole keyboard to one colour.
pub fn color_report(c: Rgb8) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[1] = 0x0E;
    r[2] = c.r;
    r[3] = c.g;
    r[4] = c.b;
    r[8] = 255u8.wrapping_sub(0x0E).wrapping_sub(c.r).wrapping_sub(c.g).wrapping_sub(c.b);
    r
}

// ---------------------------------------------------------------- per-key reports (#311)
//
// The byte layout below came from other people's notes; the key map and the upload were then
// checked on this keyboard (#311, `docs/adapters.md`, "Checked on the device (2026-11-13)").
//
// The one per-key path stock firmware has is a *stored* picture: seven `SET_USERPIC` (0x0C)
// reports upload 128 slots of colour into the keyboard's flash, and light mode 13 shows it. It
// is not a live stream, so [`RoyuanAdapter::write_frame`] sends it only for a still (#312).

/// Colour slots in one picture: 128 on the yc500 family, three bytes each.
pub const PER_KEY_SLOTS: usize = 128;
/// Bytes in one picture: `PER_KEY_SLOTS` * 3.
pub const PER_KEY_BYTES: usize = PER_KEY_SLOTS * 3;
/// Reports in one picture upload.
pub const PER_KEY_PAGES: usize = 7;
/// Picture bytes in one report (the last report carries the remaining 48).
pub const PER_KEY_PAGE_DATA: usize = 56;
/// Rows in the key matrix; the slot order runs down each column.
pub const KEY_ROWS: usize = 6;
/// Columns in the key matrix: 128 slots in columns of 6, rounded up.
pub const KEY_COLS: usize = PER_KEY_SLOTS.div_ceil(KEY_ROWS);

const SET_USERPIC: u8 = 0x0C;
const SET_LEDPARAM: u8 = 0x07;
const MODE_USER_PICTURE: u8 = 13;

/// The slot a key has in a picture: the matrix runs down each column, six rows to a column.
///
/// `row` 0 is the top row. The slot is `col * 6 + row`; `None` for a row past the sixth or a
/// position past the last slot (a row of 6 would alias the next column). Which key sits at
/// which row and column is per board (the table in `docs/adapters.md`).
pub fn key_slot(row: usize, col: usize) -> Option<usize> {
    let slot = col.checked_mul(KEY_ROWS)?.checked_add(row)?;
    (row < KEY_ROWS && slot < PER_KEY_SLOTS).then_some(slot)
}

/// The row and column of a slot, the inverse of [`key_slot`].
pub fn slot_position(slot: usize) -> (usize, usize) {
    (slot % KEY_ROWS, slot / KEY_ROWS)
}

/// The zone's key matrix: `map[row * KEY_COLS + col]` is the slot of the key there, or `None`
/// for the positions past the last slot (the four bottom rows of the last column).
pub fn key_map() -> Vec<Option<u32>> {
    (0..KEY_ROWS)
        .flat_map(|row| (0..KEY_COLS).map(move |col| key_slot(row, col).map(|slot| slot as u32)))
        .collect()
}

/// The whole-board colour for a frame: the arithmetic mean of each channel over the colours
/// the matrix shows, rounded to the nearest step.
///
/// The keyboard has one whole-board LED, so it shows one colour whatever the frame holds. The
/// mean runs over the first [`PER_KEY_SLOTS`] colours, the ones the matrix has a key for: a
/// longer frame drops its extra colours as the picture does, and a shorter frame averages the
/// colours it has (so a one-colour frame is that colour, exactly as before). An empty frame is
/// black.
///
/// This is the same area average the board showed before #312: with the default `AreaAverage`
/// sampling, the old one-LED zone's cell was the whole rect, so its colour was this mean.
pub fn frame_color(colors: &[Rgb8]) -> Rgb8 {
    let shown = &colors[..colors.len().min(PER_KEY_SLOTS)];
    if shown.is_empty() {
        return Rgb8::BLACK;
    }
    let n = shown.len() as u32;
    let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
    for c in shown {
        r += u32::from(c.r);
        g += u32::from(c.g);
        b += u32::from(c.b);
    }
    let mean = |sum: u32| ((sum + n / 2) / n) as u8;
    Rgb8::new(mean(r), mean(g), mean(b))
}

/// A frame as the picture holds it: its first [`PER_KEY_SLOTS`] colours, padded with black
/// where the frame is short, which is what the seven reports write. Two frames with the same
/// slots make the same picture, so they are compared in this form.
fn picture_slots(frame: &[Rgb8]) -> Vec<Rgb8> {
    let mut slots = frame[..frame.len().min(PER_KEY_SLOTS)].to_vec();
    slots.resize(PER_KEY_SLOTS, Rgb8::BLACK);
    slots
}

/// A hash of a picture's slots, the form the store keeps them in.
///
/// The store cannot carry a frame back into the adapter, but it can tell one picture from
/// another: a still that hashes to the stored picture is the picture the flash holds. (The
/// default hasher is fixed-key, so the hash is the same in the next run; it is not a stable
/// contract across compiler versions, and a mismatch only costs one write.)
fn picture_hash(slots: &[Rgb8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for c in slots {
        (c.r, c.g, c.b).hash(&mut hasher);
    }
    hasher.finish()
}

/// What the keyboard's stills keep between runs (#312 review: the restart loop).
///
/// The counters are the wear bound, so they outlive the process: without them a daemon restart
/// would reset the caps, and a crash loop would write the flash on every start. The picture's
/// hash means a restart does not rewrite a picture the flash already holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StoredStills {
    /// The Unix second of each upload attempt of the last day, oldest first.
    pub attempts: Vec<u64>,
    /// The hash of the picture the flash holds, when the last upload finished whole.
    pub picture: Option<u64>,
}

/// Where [`StoredStills`] lives: a small JSON file under the daemon's user directory.
///
/// The daemon passes the path in (`crates/canvas-daemon/src/main.rs`), never `%ProgramData%`.
pub trait StillStore: Send + Sync {
    /// Reads the stored state. An error (a missing file included) makes the adapter treat the
    /// counters as spent and the picture as unknown.
    fn load(&self) -> Result<StoredStills, String>;
    /// Writes the state back. An error is logged and the run goes on in memory.
    fn save(&self, stored: &StoredStills) -> Result<(), String>;
}

/// The real store: one small JSON file, written whole through a temp file and a rename.
pub struct FileStillStore {
    path: std::path::PathBuf,
}

impl FileStillStore {
    /// A store at `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl StillStore for FileStillStore {
    fn load(&self) -> Result<StoredStills, String> {
        let bytes = std::fs::read(&self.path).map_err(|e| format!("{}: {e}", self.path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    fn save(&self, stored: &StoredStills) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(stored).map_err(|e| e.to_string())?;
        let dir = self
            .path
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", self.path.display()))?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        // A temp file in the same directory, then the rename: a crash never leaves half a file.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

/// The Unix second now: the store's clock.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One upload attempt, on both clocks the adapter needs (#312 review B1).
struct Attempt {
    /// The caller's clock, for the rolling windows.
    at: Instant,
    /// The Unix second it happened at, for the store.
    unix: u64,
}

/// Whether `b` is a different frame from `a`: another length, or any channel more than
/// [`STILL_TOLERANCE`] away. A drift of one step in a channel is the same still.
fn frames_differ(a: &[Rgb8], b: &[Rgb8]) -> bool {
    a.len() != b.len()
        || a.iter().zip(b).any(|(x, y)| {
            x.r.abs_diff(y.r) > STILL_TOLERANCE
                || x.g.abs_diff(y.g) > STILL_TOLERANCE
                || x.b.abs_diff(y.b) > STILL_TOLERANCE
        })
}

/// The sum-complement checksum the family uses: `0xFF - (sum & 0xFF)`.
fn complement(sum: u32) -> u8 {
    0xFF - (sum & 0xFF) as u8
}

/// One report of the stored-picture upload: report `page` (0 to 6) of `frame`.
///
/// `frame` holds the colour of each slot in slot order; slots past its end go out black, and
/// colours past slot 127 are dropped.
/// Layout of the 65 bytes (index 0 is the report id): `[0]` 0x00, `[1]` 0x0C, `[2]` 0x00,
/// `[3]` 0x80 and `[4]` 0x01 (the length 384, little-endian), `[5]` page, `[6]` and `[7]`
/// 0x00, `[8]` checksum over `[1..=7]`, `[9..]` up to 56 bytes of R, G, B per slot, in slot
/// order, zero-padded. Page 6 carries 48 bytes.
pub fn per_key_report(page: usize, frame: &[Rgb8]) -> [u8; REPORT_LEN] {
    assert!(page < PER_KEY_PAGES, "a picture has {PER_KEY_PAGES} reports");
    let mut r = [0u8; REPORT_LEN];
    r[1] = SET_USERPIC;
    r[3] = (PER_KEY_BYTES & 0xFF) as u8;
    r[4] = (PER_KEY_BYTES >> 8) as u8;
    r[5] = page as u8;
    r[8] = complement(r[1..8].iter().map(|b| u32::from(*b)).sum());
    let start = page * PER_KEY_PAGE_DATA;
    let end = (start + PER_KEY_PAGE_DATA).min(PER_KEY_BYTES);
    for (i, byte) in (start..end).enumerate() {
        let c = frame.get(byte / 3).copied().unwrap_or(Rgb8::BLACK);
        r[9 + i] = [c.r, c.g, c.b][byte % 3];
    }
    r
}

/// All seven reports of one picture upload, in the order they are sent.
pub fn per_key_reports(frame: &[Rgb8]) -> [[u8; REPORT_LEN]; PER_KEY_PAGES] {
    std::array::from_fn(|page| per_key_report(page, frame))
}

/// The report that shows the stored picture: light mode 13 ("User Picture").
///
/// Layout: `[1]` 0x07, `[2]` 0x0D, `[3]` speed 3, `[4]` brightness 4, `[5]` 0x00 (the
/// yc500 user picture has no slot byte), `[6]` 0x00, `[7]` 0xC8, `[8]` 0xC8 (the colour bytes
/// the vendor app sends in this mode; what the firmware does with them is unknown) and `[9]`
/// the checksum over `[1..=8]`.
pub fn user_picture_mode_report() -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[1..9].copy_from_slice(&[SET_LEDPARAM, MODE_USER_PICTURE, 3, 4, 0, 0, 0xC8, 0xC8]);
    r[9] = complement(r[1..9].iter().map(|b| u32::from(*b)).sum());
    r
}

fn device_id() -> DeviceId {
    DeviceId(format!("royuan:{VENDOR_ID:04x}:{PRODUCT_ID:04x}"))
}

// ---------------------------------------------------------------- transport

/// An open handle that can send one feature report.
///
/// This is the seam the tests fake (the guard test over the daemon's adapters needs to see the
/// reports the keyboard sends), the way the service's `PortIo` and `FanChip` are.
pub trait Transport: Send + Sync {
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String>;
}

/// Finds and opens the vendor interface.
///
/// The seam the tests fake; the product uses [`HidOpener`].
pub trait Opener: Send + Sync {
    /// True when the vendor interface is present.
    fn present(&self) -> bool;
    /// Opens the vendor interface. `Ok(None)` when it is not there.
    fn open(&self) -> Result<Option<Box<dyn Transport>>, String>;
}

/// `HidDevice` is `Send` but not `Sync`; the mutex supplies `Sync` (it is never contended).
struct HidTransport(std::sync::Mutex<hidapi::HidDevice>);

impl Transport for HidTransport {
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        dev.send_feature_report(data).map_err(|e| e.to_string())
    }
}

struct HidOpener;

impl HidOpener {
    fn find_path(api: &hidapi::HidApi) -> Option<std::ffi::CString> {
        api.device_list()
            .find(|d| {
                d.vendor_id() == VENDOR_ID
                    && d.product_id() == PRODUCT_ID
                    && d.interface_number() == INTERFACE
                    && d.usage_page() == USAGE_PAGE
                    && d.usage() == USAGE
            })
            .map(|d| d.path().to_owned())
    }
}

impl Opener for HidOpener {
    fn present(&self) -> bool {
        hidapi::HidApi::new().map(|api| Self::find_path(&api).is_some()).unwrap_or(false)
    }

    fn open(&self) -> Result<Option<Box<dyn Transport>>, String> {
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        match Self::find_path(&api) {
            None => Ok(None),
            Some(path) => {
                let dev = api.open_path(&path).map_err(|e| e.to_string())?;
                Ok(Some(Box::new(HidTransport(std::sync::Mutex::new(dev)))))
            }
        }
    }
}

// ---------------------------------------------------------------- adapter

pub struct RoyuanAdapter {
    opener: Box<dyn Opener>,
    dev: Option<Box<dyn Transport>>,
    /// Colour and time of the last successful whole-board send.
    last_sent: Option<(Rgb8, Instant)>,
    /// Last colour handed to `write_frame` that went out (for `read_back`).
    last_color: Rgb8,
    /// The resume count when the keyboard last got its direct-mode report.
    seen_resume: u64,
    /// Set after a resume (or a refused picture): the next write sends the direct-mode report.
    direct_pending: bool,
    /// The still picture: when the per-key frame last moved, and the upload in flight (#312).
    picture: Picture,
    /// What the keyboard's flash holds (#312 review S3): a picture it holds whole, or nothing
    /// we can trust. A reconnect does not clear it: the flash keeps what it was given; a daemon
    /// restart keeps only its hash, from the store.
    flash: Flash,
    /// Every upload attempt of the last day, oldest first (#312 review B1, S1): one entry per
    /// try, before its first page goes out, so a refused page is still counted and gapped.
    attempts: VecDeque<Attempt>,
    /// Where the attempts and the picture's hash are kept between runs, when the daemon gave
    /// the adapter a file. `None` keeps them in memory only (the tests and the probes).
    store: Option<Box<dyn StillStore>>,
    /// The Unix second [`clock_base`] stood for: the store's clock, driven by the caller's.
    clock_unix: u64,
    /// The `Instant` the wall clock was read at, for [`Self::unix_at`].
    clock_base: Instant,
    /// While this has not passed, no upload starts: the store could not be read, so the
    /// counters are treated as spent (#312 review: fail closed).
    hold_off_until: Option<Instant>,
    /// How long a still has to hold before it uploads. Doubled when a shown picture is
    /// abandoned soon after it appears, back to [`STILL_AFTER`] after a picture that stays.
    still_after: Duration,
}

/// What the keyboard's flash holds (#312 review S3).
///
/// `GET_USERPIC` returns stale data (docs/adapters.md), so the adapter remembers what it wrote
/// rather than asking the board. An upload that stopped before its last page left some pages of
/// a new picture over the old one, and the board would show that torn picture if it ever entered
/// light mode 13 on its own: [`Flash::Unknown`] makes the next still write all seven pages.
#[derive(Default)]
enum Flash {
    /// Nothing uploaded in this run, or an upload ended before its last page.
    #[default]
    Unknown,
    /// The hash of a whole picture a previous run wrote and stored: enough to tell whether a
    /// still is the picture in the flash, not enough to compare step by step.
    Hashed(u64),
    /// A whole picture, its 128 slots in slot order.
    Known(Vec<Rgb8>),
}

/// The stored-picture state (#312), for one connection.
#[derive(Default)]
struct Picture {
    /// The frame the still window is measured against.
    baseline: Option<Vec<Rgb8>>,
    /// When `baseline` arrived.
    since: Option<Instant>,
    /// The latest frame within tolerance of `baseline`: the picture a still uploads.
    latest: Option<Vec<Rgb8>>,
    /// The upload in flight, if any.
    upload: Option<Upload>,
    /// True while light mode 13 shows a stored picture: no whole-board colour goes out then.
    showing: bool,
    /// When light mode 13 last showed the picture; the still window backs off from it.
    shown_at: Option<Instant>,
    /// A refused upload logs one line, not one a frame.
    warned: bool,
}

/// One upload: its seven pages, then the mode report that shows the picture.
struct Upload {
    /// The picture's 128 slots, in slot order, as the seven reports write them.
    frame: Vec<Rgb8>,
    /// The seven reports, built once when the upload starts.
    reports: Box<[[u8; REPORT_LEN]; PER_KEY_PAGES]>,
    /// How many pages have gone out. [`PER_KEY_PAGES`] means the mode report is all that is
    /// left, and an abandon from here leaves a whole picture in the flash.
    next: usize,
    /// The earliest time the next report may go.
    at: Instant,
}

impl Default for RoyuanAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl RoyuanAdapter {
    pub fn new() -> Self {
        Self::with_opener(Box::new(HidOpener))
    }

    /// The adapter with an opener and a store of the caller's.
    ///
    /// The product passes the file under the daemon's user directory, so the counters and the
    /// picture's hash outlive a restart (#312 review). The adapter reads it here, once: a store
    /// that cannot be read holds every upload off for [`STORE_HOLD_OFF`] and leaves the picture
    /// unknown, which is the closed way round.
    pub fn with_store(opener: Box<dyn Opener>, store: Box<dyn StillStore>) -> Self {
        let mut adapter = Self::with_opener(opener);
        adapter.store = Some(store);
        adapter.load_stored();
        adapter
    }

    /// The real keyboard over HID, with its counters and picture hash in `store`.
    pub fn with_file_store(store: Box<dyn StillStore>) -> Self {
        Self::with_store(Box::new(HidOpener), store)
    }

    /// The adapter with an opener of the caller's.
    ///
    /// The product uses the HID opener; the tests use a fake one, so they can count the
    /// reports the keyboard sends. Without a store the counters and the picture live in memory
    /// only.
    pub fn with_opener(opener: Box<dyn Opener>) -> Self {
        Self {
            opener,
            dev: None,
            last_sent: None,
            last_color: Rgb8::BLACK,
            seen_resume: crate::resume_epoch(),
            direct_pending: false,
            picture: Picture::default(),
            flash: Flash::Unknown,
            attempts: VecDeque::new(),
            store: None,
            clock_unix: unix_now(),
            clock_base: Instant::now(),
            hold_off_until: None,
            still_after: STILL_AFTER,
        }
    }

    /// The Unix second the caller's clock has reached at `now`.
    fn unix_at(&self, now: Instant) -> u64 {
        self.clock_unix + now.saturating_duration_since(self.clock_base).as_secs()
    }

    /// Reads the store into the counters and the flash state.
    fn load_stored(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        match store.load() {
            Ok(stored) => {
                let now = self.clock_base;
                let unix_now = self.clock_unix;
                for unix in stored.attempts {
                    // A stamp from the future (a clock set back) is kept at the base, so it
                    // still counts against the caps until it ages out.
                    let age = unix_now.saturating_sub(unix);
                    if age >= ONE_DAY.as_secs() {
                        continue;
                    }
                    let at = now.checked_sub(Duration::from_secs(age)).unwrap_or(now);
                    self.attempts.push_back(Attempt { at, unix });
                }
                self.flash = match stored.picture {
                    Some(hash) => Flash::Hashed(hash),
                    None => Flash::Unknown,
                };
            }
            Err(e) => {
                // Closed, not open: a missing or unreadable file could otherwise be deleted to
                // reset the counters, so the caps are treated as spent for a while.
                tracing::warn!(
                    error = %e,
                    "the keyboard's still bookkeeping could not be read; uploads wait ten minutes"
                );
                self.hold_off_until = Some(self.clock_base + STORE_HOLD_OFF);
            }
        }
    }

    /// Writes the counters and the picture's hash back, when there is a store.
    fn persist(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let stored = StoredStills {
            attempts: self.attempts.iter().map(|a| a.unix).collect(),
            picture: match &self.flash {
                Flash::Known(frame) => Some(picture_hash(frame)),
                Flash::Hashed(hash) => Some(*hash),
                Flash::Unknown => None,
            },
        };
        if let Err(e) = store.save(&stored) {
            tracing::warn!(error = %e, "the keyboard's still bookkeeping could not be written");
        }
    }

    fn check_id(&self, id: &DeviceId) -> Result<(), AdapterError> {
        if *id == device_id() {
            Ok(())
        } else {
            Err(AdapterError::UnknownDevice(id.clone()))
        }
    }

    fn open_and_init(&mut self) -> Result<(), AdapterError> {
        let mut dev = self
            .opener
            .open()
            .map_err(AdapterError::Protocol)?
            .ok_or(AdapterError::NotConnected)?;
        dev.send_feature(&direct_mode_report()).map_err(AdapterError::Protocol)?;
        self.dev = Some(dev);
        self.last_sent = None;
        self.seen_resume = crate::resume_epoch();
        self.direct_pending = false;
        // The connection's still state starts over; the flash keeps what it was given, and the
        // gap, the hour's uploads and the still window stay with it.
        self.drop_picture();
        self.picture = Picture::default();
        Ok(())
    }

    /// Takes the resume into account: a wake may have dropped the keyboard out of direct mode
    /// (and out of light mode 13), so the next write sets the mode again.
    ///
    /// A picture the flash holds stays known: after a resume whose frame did not move, the next
    /// still shows it again with light mode 13 instead of writing it again (#312 review S2).
    fn sync_resume(&mut self) {
        if crate::resume_epoch() != self.seen_resume {
            self.seen_resume = crate::resume_epoch();
            self.direct_pending = true;
            self.drop_picture();
            // The board's own mode over the sleep is unknown, so the picture has to be shown
            // again before its time on the board counts.
            self.picture.shown_at = None;
        }
    }

    /// Drops a picture in flight or on show: the whole-board colour owns the board again.
    ///
    /// An upload that stopped before its last page leaves part of a new picture over the old
    /// one, so the flash no longer holds a picture we know (#312 review S3); the next still
    /// writes all seven pages. A picture whose seven pages all went out is whole even if the
    /// mode report never did, so it stays known. A running adapter only comes here when the
    /// device goes (a disconnect, a resume, a failed colour), never because a frame moved.
    fn drop_picture(&mut self) {
        if let Some(upload) = self.picture.upload.take() {
            self.flash = if upload.next < PER_KEY_PAGES {
                Flash::Unknown
            } else {
                Flash::Known(upload.frame)
            };
            self.persist();
        }
        self.end_shown();
    }

    /// Ends a picture on show, keeping a picture in flight: a frame that moves does not stop a
    /// write the flash has already paid for (#312 review B2).
    fn end_shown(&mut self) {
        self.picture.showing = false;
    }

    /// A shown picture the frame has moved away from: widen the still window, or reset it.
    ///
    /// Soon after a picture appears the effect is changing faster than its write is worth, so
    /// the next still has to hold twice as long; a picture that stayed a minute is a settled
    /// effect, and the next still is back to [`STILL_AFTER`]. Every abandon goes through here:
    /// an upload is never dropped half written (#312 review B2), so an effect that keeps moving
    /// gives up its window once per picture instead of once per frame.
    fn back_off(&mut self, now: Instant) {
        let Some(shown_at) = self.picture.shown_at.take() else {
            self.still_after = (self.still_after * 2).min(STILL_AFTER_MAX);
            return;
        };
        let shown_for = now.saturating_duration_since(shown_at);
        if shown_for >= STILL_LONG {
            self.still_after = STILL_AFTER;
        } else if shown_for < ABANDON_FAST {
            self.still_after = (self.still_after * 2).min(STILL_AFTER_MAX);
        }
    }

    /// Whether the flash already holds this still, within [`STILL_TOLERANCE`] of the picture
    /// written last: showing it again then costs no write at all.
    fn flash_holds(&self, frame: &[Rgb8]) -> bool {
        let slots = picture_slots(frame);
        match &self.flash {
            Flash::Known(picture) => !frames_differ(picture, &slots),
            Flash::Hashed(hash) => picture_hash(&slots) == *hash,
            Flash::Unknown => false,
        }
    }

    /// Whether the picture on show is still the frame the effect holds.
    fn picture_is_current(&self, latest: &[Rgb8]) -> bool {
        match &self.flash {
            Flash::Known(picture) => !frames_differ(picture, &picture_slots(latest)),
            _ => false,
        }
    }

    /// Whether a still may be attempted now: the last attempt is a [`UPLOAD_MIN_GAP`] back, the
    /// hour and the day have room, and a store that could not be read is not holding us off.
    ///
    /// An attempt is counted before its first page goes out, so this is also what bounds a
    /// keyboard that refuses every page (#312 review B1).
    fn upload_allowed(&mut self, now: Instant) -> bool {
        if self.hold_off_until.is_some_and(|until| now < until) {
            return false;
        }
        self.attempts.retain(|a| now.saturating_duration_since(a.at) < ONE_DAY);
        let gap_ok = self
            .attempts
            .back()
            .is_none_or(|last| now.saturating_duration_since(last.at) >= UPLOAD_MIN_GAP);
        let last_hour = self
            .attempts
            .iter()
            .filter(|a| now.saturating_duration_since(a.at) < ONE_HOUR)
            .count();
        gap_ok
            && last_hour < UPLOADS_PER_HOUR_MAX
            && self.attempts.len() < UPLOADS_PER_DAY_MAX
    }

    /// Sends the direct-mode report and forgets the last colour, so the colour follows.
    fn resend_direct(&mut self) -> Result<(), AdapterError> {
        let Some(dev) = self.dev.as_mut() else {
            return Err(AdapterError::NotConnected);
        };
        if let Err(e) = dev.send_feature(&direct_mode_report()) {
            self.dev = None;
            self.last_sent = None;
            return Err(AdapterError::Protocol(e));
        }
        self.direct_pending = false;
        self.last_sent = None;
        Ok(())
    }

    /// The whole-board path: one colour, skipped while it is unchanged within [`KEEPALIVE`].
    fn write_at(&mut self, color: Rgb8, now: Instant) -> Result<(), AdapterError> {
        if self.dev.is_none() {
            return Err(AdapterError::NotConnected);
        }
        // A diagnostic write owns the board: a picture on show, or an upload in flight, gives
        // way. The board is in light mode 13 then, and a colour report does not switch modes, so
        // the direct-mode report goes out first or the colour would not be shown (#312 review).
        let was_showing = self.picture.showing || self.picture.upload.is_some();
        self.drop_picture();
        self.sync_resume();
        if was_showing {
            self.direct_pending = true;
        }
        if self.direct_pending {
            self.resend_direct()?;
        }
        self.send_color(color, now)
    }

    fn send_color(&mut self, color: Rgb8, now: Instant) -> Result<(), AdapterError> {
        let Some(dev) = self.dev.as_mut() else {
            return Err(AdapterError::NotConnected);
        };
        if let Some((last, at)) = self.last_sent {
            if last == color && now.saturating_duration_since(at) < KEEPALIVE {
                return Ok(());
            }
        }
        match dev.send_feature(&color_report(color)) {
            Ok(()) => {
                self.last_sent = Some((color, now));
                self.last_color = color;
                Ok(())
            }
            Err(e) => {
                self.dev = None;
                self.last_sent = None;
                // The upload in flight went with the device, and the last pages may be in the
                // flash: it holds nothing we can count on now.
                self.drop_picture();
                self.picture = Picture::default();
                Err(AdapterError::Protocol(e))
            }
        }
    }

    /// One frame: the whole-board colour, or a still picture over the keys (#312).
    ///
    /// The clock is the caller's, so the tests drive seconds without sleeping. The upload is a
    /// small state machine advanced by the frames the engine already streams: one page per
    /// [`PAGE_GAP`], then the mode report [`PICTURE_SETTLE`] after the last page. Nothing here
    /// sleeps, so the frame path never waits for the seconds an upload takes.
    ///
    /// Once an upload starts it finishes, whatever the frame does: the flash write is spent, so
    /// dropping the remaining pages would only leave a torn picture and buy nothing (#312 review
    /// B2). A frame that moves ends a picture *on show*, and each of those abandons widens the
    /// still window.
    fn write_frame_at(&mut self, colors: &[Rgb8], now: Instant) -> Result<(), AdapterError> {
        if self.dev.is_none() {
            return Err(AdapterError::NotConnected);
        }
        self.sync_resume();

        // Where this frame stands against the one the still window began with.
        let moved = match self.picture.baseline.as_deref() {
            Some(baseline) => frames_differ(baseline, colors),
            None => true,
        };
        if moved {
            // A moving frame: the picture on show gives way. An upload in flight does not: its
            // pages still go out below.
            let was_showing = self.picture.showing;
            self.end_shown();
            self.picture.baseline = Some(colors.to_vec());
            self.picture.since = Some(now);
            self.picture.latest = Some(colors.to_vec());
            if was_showing {
                // Back to the whole-board colour now, not at the next keepalive.
                self.direct_pending = true;
                self.back_off(now);
            }
        } else {
            self.picture.latest = Some(colors.to_vec());
        }

        // The pages keep their own pace whether the frame moves or not, so a started upload
        // always reaches light mode 13.
        self.advance_upload(now);

        if self.picture.showing {
            if self.picture_is_current(colors) {
                // The board shows this still: nothing else goes out while it holds.
                return Ok(());
            }
            // The frame moved on while the picture was being written: the colour takes over,
            // and this abandon widens the window like any other.
            self.end_shown();
            self.direct_pending = true;
            self.back_off(now);
        }

        // A still long enough takes the board. The picture the flash already holds (even one a
        // previous run wrote) is shown again with light mode 13, which writes nothing; a new one
        // is attempted when the gap, the hour and the day allow it.
        if self.picture.upload.is_none() {
            let still_for = self
                .picture
                .since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
            if still_for >= self.still_after {
                if let Some(frame) = self.picture.latest.clone() {
                    if self.flash_holds(&frame) {
                        self.show_picture(now);
                    } else if self.upload_allowed(now) {
                        self.start_upload(&frame, now);
                    }
                }
            }
        }

        // Until light mode 13 goes out the whole-board colour keeps streaming (#312 review S4):
        // what the board shows during the ~3 s of pages is UNKNOWN (docs/adapters.md, "Frame
        // rate"), so the colour it was already given is the one state we can count on.
        if self.picture.showing {
            return Ok(());
        }
        if self.direct_pending {
            self.resend_direct()?;
        }
        // The moving colour is the frame's per-channel mean: the same area average a one-LED
        // zone read from the whole rect before #312, so a moving effect looks as it always did.
        self.send_color(frame_color(colors), now)
    }

    /// Shows the picture the flash already holds: light mode 13 again, no write and no wear.
    fn show_picture(&mut self, now: Instant) {
        if self.send_picture_report(&user_picture_mode_report()).is_err() {
            self.upload_failed();
            return;
        }
        // The still is the picture the flash holds (that is why it is shown, not written), so
        // the frame is known again, not just its hash.
        if let Some(frame) = self.picture.latest.clone() {
            self.picture_shown(frame, now);
        }
    }

    /// Returns that light mode 13 went out, showing `frame`'s picture (or the one already
    /// known to be in the flash, for `None`).
    fn picture_shown(&mut self, frame: Vec<Rgb8>, now: Instant) {
        self.flash = Flash::Known(frame);
        // A whole picture is now in the flash, so the next run need not write it again.
        self.persist();
        self.picture.showing = true;
        self.picture.shown_at = Some(now);
        self.picture.warned = false;
        // Light mode 13 is the board's mode now: a direct-mode report after it would drop it.
        self.direct_pending = false;
    }

    /// Starts an upload of `frame`: counts the attempt, then sends its first page.
    ///
    /// The attempt is recorded before any page goes out (#312 review B1): a board that refuses
    /// page 0 is still counted, still waits out [`UPLOAD_MIN_GAP`], and still counts against the
    /// hour and the day, so a flaky endpoint cannot be made to write the flash every frame. A
    /// failed page, page 0 included, leaves the flash unknown ([`Self::upload_failed`]).
    fn start_upload(&mut self, frame: &[Rgb8], now: Instant) {
        let frame = picture_slots(frame);
        let reports = Box::new(per_key_reports(&frame));
        self.attempts.push_back(Attempt { at: now, unix: self.unix_at(now) });
        self.persist();
        self.picture.upload = Some(Upload { frame, reports, next: 0, at: now });
        self.advance_upload(now);
    }

    /// Sends what the upload owes at `now`: at most one report per call, at the adapter's pace.
    fn advance_upload(&mut self, now: Instant) {
        loop {
            let Some(upload) = self.picture.upload.take() else {
                return;
            };
            if now < upload.at {
                self.picture.upload = Some(upload);
                return;
            }
            if upload.next < PER_KEY_PAGES {
                let report = upload.reports[upload.next];
                if self.send_picture_report(&report).is_err() {
                    self.picture.upload = Some(upload);
                    self.upload_failed();
                    return;
                }
                let next = upload.next + 1;
                // The settle runs from the last page; the pages are a page gap apart.
                let at = if next == PER_KEY_PAGES { now + PICTURE_SETTLE } else { now + PAGE_GAP };
                self.picture.upload =
                    Some(Upload { frame: upload.frame, reports: upload.reports, next, at });
                continue;
            }
            if self.send_picture_report(&user_picture_mode_report()).is_err() {
                // All seven pages are in, so the picture is whole; it is just not shown.
                self.picture.upload = Some(upload);
                self.upload_failed();
                return;
            }
            self.picture_shown(upload.frame, now);
            return;
        }
    }

    /// A page or the mode report the keyboard refused: the whole-board colour takes over again,
    /// and the refusal is logged once.
    fn upload_failed(&mut self) {
        self.drop_picture();
        // The board may or may not have taken the last report: send direct mode, then the colour.
        self.direct_pending = true;
        self.last_sent = None;
        if !self.picture.warned {
            self.picture.warned = true;
            tracing::warn!(
                "the ROYUAN keyboard refused a per-key picture; the whole-board colour takes over"
            );
        }
    }

    fn send_picture_report(&mut self, report: &[u8]) -> Result<(), String> {
        match self.dev.as_mut() {
            Some(dev) => dev.send_feature(report),
            None => Err("not connected".into()),
        }
    }

    fn device_info(&self) -> DeviceInfo {
        DeviceInfo {
            id: device_id(),
            name: "ROYUAN MonsGeek Keyboard".into(),
            vendor: "ROYUAN".into(),
            kind: DeviceKind::Keyboard,
            adapter: "royuan".into(),
            adapter_index: 0,
            serial: String::new(),
            location: format!("hid:{VENDOR_ID:04x}:{PRODUCT_ID:04x}:if{INTERFACE}"),
            zones: vec![ZoneInfo {
                index: 0,
                name: "Keyboard".into(),
                kind: ZoneKind::Matrix {
                    width: KEY_COLS as u32,
                    height: KEY_ROWS as u32,
                    map: key_map(),
                },
                led_count: PER_KEY_SLOTS as u32,
                min_leds: PER_KEY_SLOTS as u32,
                max_leds: PER_KEY_SLOTS as u32,
                resizable: false,
            }],
            connected: true,
        }
    }
}

#[async_trait]
impl DeviceAdapter for RoyuanAdapter {
    fn name(&self) -> &'static str {
        "royuan"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            resizable_zones: false,
            identify: true,
            hardware_brightness: false,
            needs_elevation: false,
        }
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        if self.dev.is_some() {
            return Ok(());
        }
        self.open_and_init()
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        self.dev = None;
        self.last_sent = None;
        // The flash keeps what it was given, and the upload gap, the hour's count and the still
        // window stay with it, so a flapping device cannot buy uploads (#312 review S1).
        self.drop_picture();
        self.picture = Picture::default();
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.dev.is_some()
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        if self.dev.is_some() || self.opener.present() {
            Ok(vec![self.device_info()])
        } else {
            Ok(Vec::new())
        }
    }

    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
        self.check_id(device)?;
        self.write_frame_at(colors, Instant::now())
    }

    async fn set_zone_size(&mut self, device: &DeviceId, _zone: u32, _leds: u32) -> Result<(), AdapterError> {
        self.check_id(device)?;
        Err(AdapterError::Unsupported("the ROYUAN keyboard has a fixed key matrix"))
    }

    async fn identify(&mut self, device: &DeviceId, zone: u32, _led: u32) -> Result<(), AdapterError> {
        self.check_id(device)?;
        if zone != 0 {
            return Err(AdapterError::Zone { device: device.clone(), zone, reason: "no such zone".into() });
        }
        self.write_at(Rgb8::WHITE, Instant::now())
    }

    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        self.check_id(device)?;
        Ok(vec![self.last_color])
    }
}

impl crate::resume::ResumeBehavior for RoyuanAdapter {
    fn on_resume(&self) -> crate::resume::OnResume {
        // The keyboard may drop out of direct mode over a sleep (seen for the RAM in #5; not
        // yet seen for the keyboard), so the direct-mode report goes out again before the next
        // colour.
        crate::resume::OnResume::SetsModeAgain
    }
}

#[cfg(test)]
mod tests {
    // The serial guard is held across awaits on purpose: it keeps tests that count reports apart.
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use std::sync::{Arc, Mutex};

    /// The keyboard's colour memory as the fake models it (#311): the stored picture the
    /// per-key reports fill in, page by page, and the light mode the last mode report set.
    struct Picture {
        bytes: [u8; PER_KEY_BYTES],
        seen: [bool; PER_KEY_PAGES],
        /// Pictures that arrived whole (all seven pages).
        complete: u32,
        /// The light mode the last valid `SET_LEDPARAM` named.
        mode: Option<u8>,
    }

    impl Default for Picture {
        fn default() -> Self {
            Self { bytes: [0; PER_KEY_BYTES], seen: [false; PER_KEY_PAGES], complete: 0, mode: None }
        }
    }

    impl Picture {
        /// The colour of each slot, in slot order.
        fn keys(&self) -> Vec<Rgb8> {
            self.bytes.chunks(3).map(|c| Rgb8::new(c[0], c[1], c[2])).collect()
        }

        /// Takes one report the way the documented firmware would: the checksum, the header
        /// and the page are checked, and a bad report changes nothing.
        fn accept(&mut self, data: &[u8]) -> Result<(), String> {
            if data.len() != REPORT_LEN || data[0] != 0 {
                return Err(format!("report is {} bytes, wanted {REPORT_LEN} with id 0", data.len()));
            }
            match data[1] {
                SET_USERPIC => {
                    let sum: u32 = data[1..8].iter().map(|b| u32::from(*b)).sum();
                    if data[8] != complement(sum) {
                        return Err("bad checksum".into());
                    }
                    let (len, page) = (usize::from(data[3]) | usize::from(data[4]) << 8, usize::from(data[5]));
                    if data[2] != 0 || len != PER_KEY_BYTES || page >= PER_KEY_PAGES || data[6] != 0 || data[7] != 0 {
                        return Err(format!("bad picture header (length {len}, page {page})"));
                    }
                    let start = page * PER_KEY_PAGE_DATA;
                    let n = PER_KEY_PAGE_DATA.min(PER_KEY_BYTES - start);
                    if data[9 + n..].iter().any(|b| *b != 0) {
                        return Err("bytes past the page's data are not zero".into());
                    }
                    self.bytes[start..start + n].copy_from_slice(&data[9..9 + n]);
                    self.seen[page] = true;
                    if self.seen.iter().all(|s| *s) {
                        self.complete += 1;
                        self.seen = [false; PER_KEY_PAGES];
                    }
                    Ok(())
                }
                SET_LEDPARAM => {
                    let sum: u32 = data[1..9].iter().map(|b| u32::from(*b)).sum();
                    if data[9] != complement(sum) {
                        return Err("bad checksum".into());
                    }
                    self.mode = Some(data[2]);
                    Ok(())
                }
                _ => Ok(()),
            }
        }
    }

    #[derive(Default)]
    struct Shared {
        sent: Vec<Vec<u8>>,
        opens: u32,
        present: bool,
        fail_next_send: bool,
        /// The keyboard refuses every `SET_USERPIC` page (#312 review B1): a board whose flash
        /// write always fails. Direct mode and the whole-board colour still go through.
        refuse_pages: bool,
        /// Page-0 reports the fake refused: one per upload attempt on such a board.
        refused: usize,
        picture: Picture,
    }

    struct FakeOpener(Arc<Mutex<Shared>>);
    struct FakeTransport(Arc<Mutex<Shared>>);

    impl Transport for FakeTransport {
        fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
            let mut s = self.0.lock().unwrap();
            if s.fail_next_send {
                s.fail_next_send = false;
                return Err("unplugged".into());
            }
            if s.refuse_pages && data.get(1) == Some(&SET_USERPIC) {
                if data.get(5) == Some(&0) {
                    s.refused += 1;
                }
                return Err("the page was refused".into());
            }
            s.picture.accept(data)?;
            s.sent.push(data.to_vec());
            Ok(())
        }
    }

    impl Opener for FakeOpener {
        fn present(&self) -> bool {
            self.0.lock().unwrap().present
        }
        fn open(&self) -> Result<Option<Box<dyn Transport>>, String> {
            let mut s = self.0.lock().unwrap();
            if !s.present {
                return Ok(None);
            }
            s.opens += 1;
            Ok(Some(Box::new(FakeTransport(self.0.clone()))))
        }
    }

    /// `note_resume` bumps a process-wide counter, so the tests that count reports run one at a time.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        crate::resume_serial()
    }

    fn rig(present: bool) -> (RoyuanAdapter, Arc<Mutex<Shared>>) {
        let shared = Arc::new(Mutex::new(Shared { present, ..Default::default() }));
        (RoyuanAdapter::with_opener(Box::new(FakeOpener(shared.clone()))), shared)
    }

    /// The keyboard as `rig` builds it, with a store beside it (#312 review S1).
    fn rig_stored(present: bool, store: FakeStore) -> (RoyuanAdapter, Arc<Mutex<Shared>>) {
        let shared = Arc::new(Mutex::new(Shared { present, ..Default::default() }));
        let adapter =
            RoyuanAdapter::with_store(Box::new(FakeOpener(shared.clone())), Box::new(store));
        (adapter, shared)
    }

    /// The still store the tests hold in memory: `None` stands for a file that is missing or
    /// will not parse.
    #[derive(Clone, Default)]
    struct FakeStore(Arc<Mutex<Option<StoredStills>>>);

    impl FakeStore {
        /// A store with a file holding `stored`.
        fn with(stored: StoredStills) -> Self {
            Self(Arc::new(Mutex::new(Some(stored))))
        }

        /// A store whose file is missing.
        fn missing() -> Self {
            Self::default()
        }

        /// What the adapter last wrote, or the default when it wrote nothing.
        fn saved(&self) -> StoredStills {
            self.0.lock().unwrap().clone().unwrap_or_default()
        }
    }

    impl StillStore for FakeStore {
        fn load(&self) -> Result<StoredStills, String> {
            self.0.lock().unwrap().clone().ok_or_else(|| "no file".to_string())
        }

        fn save(&self, stored: &StoredStills) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(stored.clone());
            Ok(())
        }
    }

    fn direct_count(s: &Shared) -> usize {
        s.sent.iter().filter(|r| r.as_slice() == direct_mode_report()).count()
    }

    fn userpic_count(s: &Shared) -> usize {
        s.sent.iter().filter(|r| r[1] == SET_USERPIC).count()
    }

    fn mode_count(s: &Shared) -> usize {
        s.sent
            .iter()
            .filter(|r| r[1] == SET_LEDPARAM && r[2] == MODE_USER_PICTURE)
            .count()
    }

    fn color_count(s: &Shared) -> usize {
        s.sent.iter().filter(|r| r[1] == 0x0E).count()
    }

    /// The clock the still tests drive: `t0` plus `ms`.
    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Writes `frame` every 100 ms in `from..to` (the upper bound is exclusive), as the engine
    /// streams it. No real time passes: the clock is the caller's.
    fn hold(a: &mut RoyuanAdapter, frame: &[Rgb8], from: u64, to: u64, t0: Instant) {
        for ms in (from..to).step_by(100) {
            a.write_frame_at(frame, at(t0, ms)).unwrap();
        }
    }

    /// `distinct_frame()` moved in every channel by `by`: a new frame from the still it came
    /// from. (The channels wrap, so this is for frames that must differ, not for pinning the
    /// tolerance; [`grey_frame`] does that.)
    fn shifted_frame(by: u8) -> Vec<Rgb8> {
        distinct_frame()
            .into_iter()
            .map(|c| Rgb8::new(c.r.wrapping_add(by), c.g.wrapping_sub(by), c.b.wrapping_add(by)))
            .collect()
    }

    /// A flat mid grey frame: `grey_frame(101)` is exactly one step from `grey_frame(100)` in
    /// every channel, with no wrap-around, so it pins [`STILL_TOLERANCE`] on its own.
    fn grey_frame(level: u8) -> Vec<Rgb8> {
        vec![Rgb8::new(level, level, level); PER_KEY_SLOTS]
    }

    #[test]
    fn direct_mode_bytes() {
        let r = direct_mode_report();
        assert_eq!(r.len(), 65);
        assert_eq!(&r[..10], &[0x00, 0x07, 0x15, 0x04, 0x04, 0x07, 0x00, 0x00, 0x00, 0xD4]);
        assert!(r[10..].iter().all(|b| *b == 0));
    }

    #[test]
    fn color_bytes_and_checksums() {
        let black = color_report(Rgb8::new(0, 0, 0));
        assert_eq!(&black[..9], &[0x00, 0x0E, 0, 0, 0, 0, 0, 0, 0xF1]);
        assert!(black[9..].iter().all(|b| *b == 0));
        let red = color_report(Rgb8::new(0xFF, 0, 0));
        assert_eq!(&red[..9], &[0x00, 0x0E, 0xFF, 0, 0, 0, 0, 0, 0xF2]);
        let c = color_report(Rgb8::new(1, 2, 3));
        assert_eq!(c[2..5], [1, 2, 3]);
        assert_eq!(c[8], 255u8.wrapping_sub(0x0E + 6));
        assert_eq!(c.len(), 65);
    }

    #[tokio::test]
    async fn discover_empty_when_absent() {
        let _serial = serial();
        let (mut a, _) = rig(false);
        assert!(a.discover().await.unwrap().is_empty());
        assert!(matches!(a.connect().await, Err(AdapterError::NotConnected)));
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn discover_describes_a_128_key_keyboard() {
        let _serial = serial();
        let (mut a, _) = rig(true);
        let d = a.discover().await.unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].id, DeviceId("royuan:3151:4015".into()));
        assert_eq!(d[0].name, "ROYUAN MonsGeek Keyboard");
        assert_eq!(d[0].kind, DeviceKind::Keyboard);
        assert_eq!(d[0].led_count(), PER_KEY_SLOTS as u32);
        assert!(d[0].connected);
        assert_eq!(a.name(), "royuan");
    }

    #[tokio::test]
    async fn direct_mode_sent_once_per_connect() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        a.connect().await.unwrap();
        a.connect().await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 1);
        a.disconnect().await.unwrap();
        assert!(!a.is_connected());
        a.connect().await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 2);
    }

    #[tokio::test]
    async fn after_note_resume_the_next_write_sends_direct_mode_before_the_colour() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        let red = [Rgb8::new(255, 0, 0)];
        a.write_frame(&id, &red).await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 1);
        a.write_frame(&id, &red).await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 1, "no resume: no repeat");

        // The production path: the daemon calls note_resume; the adapter notices on its next write.
        crate::note_resume();
        a.write_frame(&id, &red).await.unwrap();
        let g = s.lock().unwrap();
        assert_eq!(direct_count(&g), 2, "direct mode went out again");
        let n = g.sent.len();
        assert_eq!(g.sent[n - 2].as_slice(), direct_mode_report(), "before the colour");
        assert_eq!(&g.sent[n - 2][1..10], &[0x07, 0x15, 0x04, 0x04, 0x07, 0x00, 0x00, 0x00, 0xD4]);
        assert_eq!(g.sent[n - 1].as_slice(), color_report(red[0]), "the colour follows");
        drop(g);
        a.write_frame(&id, &red).await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 2, "once per resume");
    }

    #[tokio::test]
    async fn unchanged_colour_not_resent_within_a_second() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        let red = [Rgb8::new(255, 0, 0)];
        a.write_frame(&id, &red).await.unwrap();
        a.write_frame(&id, &red).await.unwrap();
        a.write_frame(&id, &red).await.unwrap();
        // direct mode + one colour
        assert_eq!(s.lock().unwrap().sent.len(), 2);
        // keepalive after a second
        let (c, at) = a.last_sent.unwrap();
        a.write_at(c, at + Duration::from_millis(999)).unwrap();
        assert_eq!(s.lock().unwrap().sent.len(), 2);
        a.write_at(c, at + Duration::from_secs(1)).unwrap();
        assert_eq!(s.lock().unwrap().sent.len(), 3);
    }

    #[tokio::test]
    async fn changed_colour_is_sent() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        a.write_frame(&id, &[Rgb8::new(255, 0, 0)]).await.unwrap();
        a.write_frame(&id, &[Rgb8::new(0, 255, 0)]).await.unwrap();
        {
            let g = s.lock().unwrap();
            assert_eq!(g.sent.len(), 3);
            assert_eq!(g.sent[2], color_report(Rgb8::new(0, 255, 0)).to_vec());
        }
        assert_eq!(a.read_back(&id).await.unwrap(), vec![Rgb8::new(0, 255, 0)]);
    }

    #[tokio::test]
    async fn empty_frame_sends_black() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        a.write_frame(&id, &[Rgb8::new(9, 9, 9)]).await.unwrap();
        a.write_frame(&id, &[]).await.unwrap();
        let g = s.lock().unwrap();
        assert_eq!(g.sent.last().unwrap(), &color_report(Rgb8::BLACK).to_vec());
    }

    #[tokio::test]
    async fn failed_send_closes_and_reconnect_resends_direct_mode() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        s.lock().unwrap().fail_next_send = true;
        assert!(a.write_frame(&id, &[Rgb8::new(1, 1, 1)]).await.is_err());
        assert!(!a.is_connected());
        assert!(matches!(
            a.write_frame(&id, &[Rgb8::new(1, 1, 1)]).await,
            Err(AdapterError::NotConnected)
        ));
        a.connect().await.unwrap();
        let g = s.lock().unwrap();
        assert_eq!(g.opens, 2);
        assert_eq!(direct_count(&g), 2);
    }

    /// A frame with a distinct colour in every slot: red is the slot index, so no two match.
    fn distinct_frame() -> Vec<Rgb8> {
        (0..PER_KEY_SLOTS).map(|i| Rgb8::new(i as u8, 255 - i as u8, (i as u8).wrapping_mul(2))).collect()
    }

    /// Opens the fake the way the adapter does and returns its transport.
    fn open_fake(shared: &Arc<Mutex<Shared>>) -> Box<dyn Transport> {
        FakeOpener(shared.clone()).open().unwrap().unwrap()
    }

    #[test]
    fn per_key_report_bytes_and_checksum_for_a_known_frame() {
        // LED 9 red and LED 40 blue, the rest black: the frame the issue's device check sends.
        let mut frame = vec![Rgb8::BLACK; PER_KEY_SLOTS];
        frame[9] = Rgb8::new(255, 0, 0);
        frame[40] = Rgb8::new(0, 0, 255);
        let reports = per_key_reports(&frame);
        assert_eq!(reports.len(), 7);
        for (page, r) in reports.iter().enumerate() {
            assert_eq!(r.len(), 65);
            // [id, opcode, 0, len lo, len hi, page, 0, 0, checksum]; 0xFF - (0x0C + 0x80 + 0x01 + page).
            let ck = 0x72 - page as u8;
            assert_eq!(&r[..9], &[0x00, 0x0C, 0x00, 0x80, 0x01, page as u8, 0x00, 0x00, ck], "page {page}");
        }
        // Slot 9 is bytes 27..30 of the picture: the first report, data starting at byte 9.
        assert_eq!(&reports[0][9 + 27..9 + 30], &[255, 0, 0]);
        // Slot 40 is bytes 120..123: report 2 (bytes 112..168), offset 8; blue is the last of the three.
        assert_eq!(&reports[2][9 + 8..9 + 11], &[0, 0, 255]);
        // Every other data byte is zero.
        let lit: usize = reports.iter().flat_map(|r| r[9..].iter()).filter(|b| **b != 0).count();
        assert_eq!(lit, 2);
        // The last report carries 48 bytes, then zeros.
        assert_eq!(reports[6][8], 0x6C, "checksum of page 6");
        assert!(reports[6][9 + 48..].iter().all(|b| *b == 0));
    }

    #[test]
    fn per_key_report_pads_a_short_frame_with_black() {
        let reports = per_key_reports(&[Rgb8::new(1, 2, 3)]);
        assert_eq!(&reports[0][9..13], &[1, 2, 3, 0]);
        assert!(reports.iter().all(|r| r.len() == 65));
    }

    #[test]
    fn user_picture_mode_bytes_and_checksum() {
        let r = user_picture_mode_report();
        assert_eq!(&r[..10], &[0x00, 0x07, 0x0D, 0x03, 0x04, 0x00, 0x00, 0xC8, 0xC8, 0x54]);
        assert!(r[10..].iter().all(|b| *b == 0));
    }

    #[test]
    fn fake_decodes_a_full_frame_of_distinct_colours_to_the_right_keys() {
        let (_, shared) = rig(true);
        let mut dev = open_fake(&shared);
        let frame = distinct_frame();
        for r in per_key_reports(&frame) {
            dev.send_feature(&r).unwrap();
        }
        dev.send_feature(&user_picture_mode_report()).unwrap();
        let g = shared.lock().unwrap();
        assert_eq!(g.picture.complete, 1);
        assert_eq!(g.picture.keys(), frame);
        assert_eq!(g.picture.mode, Some(13));
        // Spot checks by index, so a shifted decode cannot match by accident.
        assert_eq!(g.picture.keys()[0], Rgb8::new(0, 255, 0));
        assert_eq!(g.picture.keys()[55], Rgb8::new(55, 200, 110));
        assert_eq!(g.picture.keys()[127], Rgb8::new(127, 128, 254));
    }

    #[test]
    fn fake_refuses_a_bad_per_key_report_and_keeps_the_picture() {
        let (_, shared) = rig(true);
        let mut dev = open_fake(&shared);
        let mut r = per_key_report(0, &[Rgb8::new(9, 9, 9)]);
        r[8] ^= 1;
        assert_eq!(dev.send_feature(&r).unwrap_err(), "bad checksum");
        let mut wrong_page = per_key_report(0, &[]);
        wrong_page[5] = 7;
        wrong_page[8] = complement(wrong_page[1..8].iter().map(|b| u32::from(*b)).sum());
        assert!(dev.send_feature(&wrong_page).unwrap_err().starts_with("bad picture header"));
        assert!(dev.send_feature(&r[..64]).is_err());
        let g = shared.lock().unwrap();
        assert!(g.sent.is_empty(), "refused reports are not recorded");
        assert!(g.picture.keys().iter().all(|c| *c == Rgb8::BLACK));
        assert_eq!(g.picture.complete, 0);
    }

    #[test]
    fn a_picture_with_a_page_missing_is_not_complete() {
        let (_, shared) = rig(true);
        let mut dev = open_fake(&shared);
        let frame = distinct_frame();
        for (page, r) in per_key_reports(&frame).iter().enumerate() {
            if page != 3 {
                dev.send_feature(r).unwrap();
            }
        }
        assert_eq!(shared.lock().unwrap().picture.complete, 0);
        dev.send_feature(&per_key_report(3, &frame)).unwrap();
        assert_eq!(shared.lock().unwrap().picture.complete, 1);
    }

    #[test]
    fn slots_run_down_each_column() {
        assert_eq!(key_slot(0, 0), Some(0));
        assert_eq!(key_slot(5, 0), Some(5));
        assert_eq!(key_slot(0, 1), Some(6));
        // The two keys confirmed on a FUN60 Pro by the monsgeek-rgb notes (A = 9, B = 40) are the
        // second column's home-row key and the seventh column's bottom-letter-row key.
        assert_eq!(slot_position(9), (3, 1));
        assert_eq!(slot_position(40), (4, 6));
        for slot in 0..PER_KEY_SLOTS {
            let (row, col) = slot_position(slot);
            assert_eq!(key_slot(row, col), Some(slot));
            assert!(col < KEY_COLS);
        }
        assert_eq!(KEY_COLS, 22);
    }

    #[test]
    fn a_key_outside_the_matrix_has_no_slot() {
        // A seventh row would alias the next column's top key.
        assert_eq!(key_slot(KEY_ROWS, 0), None);
        // Column 21 holds slots 126 and 127 only.
        assert_eq!(key_slot(1, 21), Some(127));
        assert_eq!(key_slot(2, 21), None);
        assert_eq!(key_slot(0, KEY_COLS), None);
        assert_eq!(key_slot(0, usize::MAX), None);
    }

    #[test]
    fn per_key_report_drops_colours_past_the_last_slot() {
        let mut frame = vec![Rgb8::new(9, 9, 9); PER_KEY_SLOTS];
        frame[PER_KEY_SLOTS - 1] = Rgb8::new(1, 2, 3);
        let exact = per_key_reports(&frame);
        frame.extend([Rgb8::new(200, 100, 50); 10]);
        assert_eq!(per_key_reports(&frame), exact, "colours past slot 127 change no byte");
        // Slot 127 is the last 3 of page 6's 48 data bytes; the rest of the report is zero.
        assert_eq!(&exact[6][9 + 45..9 + 48], &[1, 2, 3]);
        assert!(exact[6][9 + 48..].iter().all(|b| *b == 0));
    }

    #[tokio::test]
    async fn unsupported_and_unknown() {
        let _serial = serial();
        let (mut a, _) = rig(true);
        let id = device_id();
        assert!(matches!(a.set_zone_size(&id, 0, 5).await, Err(AdapterError::Unsupported(_))));
        let other = DeviceId("x".into());
        assert!(matches!(a.write_frame(&other, &[]).await, Err(AdapterError::UnknownDevice(_))));
    }

    // ---- #312: stills on change

    /// The still tests stream at 10 fps, slower than the engine: one page of an upload a frame.
    const FRAME_MS: u64 = 100;
    /// Frames in one simulated hour at [`FRAME_MS`].
    const HOUR_FRAMES: u64 = 3_600_000 / FRAME_MS;
    /// Frames in one simulated day at [`FRAME_MS`].
    const DAY_FRAMES: u64 = 24 * HOUR_FRAMES;

    /// A breathing frame: every key the same colour, `level` of full brightness at the peak, at
    /// `hz` cycles a second.
    fn breathing_frame(t: f64, hz: f64, level: f64) -> Vec<Rgb8> {
        let swing = level * 255.0 * (1.0 + (std::f64::consts::TAU * hz * t).cos()) / 2.0;
        let v = swing.round().clamp(0.0, 255.0) as u8;
        vec![Rgb8::new(v, v, v); PER_KEY_SLOTS]
    }

    /// A ramp of `step_per_s` steps a second: the whole board climbs from black to white and
    /// starts again, a drift no still window can follow for long.
    fn ramp_frame(t: f64, step_per_s: f64) -> Vec<Rgb8> {
        let v = (t * step_per_s).rem_euclid(255.0).round() as u8;
        vec![Rgb8::new(v, v, v); PER_KEY_SLOTS]
    }

    /// Streams `frame(t)` every [`FRAME_MS`] of the simulated clock for `frames` frames.
    fn simulate(a: &mut RoyuanAdapter, t0: Instant, frames: u64, frame: impl Fn(f64) -> Vec<Rgb8>) {
        for i in 0..frames {
            let ms = i * FRAME_MS;
            let f = frame(ms as f64 / 1000.0);
            a.write_frame_at(&f, at(t0, ms)).expect("a frame");
        }
    }

    /// What a run of frames did to the flash: attempts started, whole pictures written, pages.
    #[derive(Debug, PartialEq, Eq)]
    struct Wear {
        attempts: usize,
        pictures: u32,
        pages: usize,
    }

    /// Runs `frames` of `frame` on a fresh keyboard and reports what it wrote.
    fn wear_of(frames: u64, frame: impl Fn(f64) -> Vec<Rgb8>) -> Wear {
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        simulate(&mut a, t0, frames, frame);
        let g = s.lock().unwrap();
        Wear { attempts: upload_count(&g), pictures: g.picture.complete, pages: userpic_count(&g) }
    }

    /// How many uploads started: the page-0 reports.
    fn upload_count(s: &Shared) -> usize {
        s.sent.iter().filter(|r| r[1] == SET_USERPIC && r[5] == 0).count()
    }

    #[test]
    fn an_animated_frame_never_uploads() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        // Every frame moves by more than one step in every channel for six seconds.
        for step in 0..60u32 {
            let frame: Vec<Rgb8> = (0..PER_KEY_SLOTS)
                .map(|slot| {
                    let n = slot as u32 + step * 17;
                    Rgb8::new((n & 0xFF) as u8, (n.wrapping_mul(3) & 0xFF) as u8, (n.wrapping_mul(7) & 0xFF) as u8)
                })
                .collect();
            a.write_frame_at(&frame, at(t0, 100 * u64::from(step))).expect("a frame");
        }
        let g = s.lock().unwrap();
        assert_eq!(g.picture.complete, 0, "an animated effect never uploads a picture");
        assert_eq!(userpic_count(&g), 0, "no page of a picture went out");
        assert_eq!(mode_count(&g), 0, "the board never entered light mode 13");
        assert!(color_count(&g) > 0, "the whole-board colour kept streaming");
    }

    #[test]
    fn a_frame_still_for_two_seconds_uploads_once() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        // Two seconds of one unchanged frame: nothing is uploaded yet.
        hold(&mut a, &frame, 0, 2000, t0);
        assert_eq!(userpic_count(&s.lock().unwrap()), 0, "a still alone waits for two seconds");
        // The still crosses 2 s here: the first page goes out.
        a.write_frame_at(&frame, at(t0, 2000)).expect("the trigger frame");
        assert_eq!(userpic_count(&s.lock().unwrap()), 1, "page 0 goes out at two seconds");
        // Pages 1 to 6, 100 ms apart, then light mode 13 two seconds after page 6.
        hold(&mut a, &frame, 2100, 4700, t0);
        let g = s.lock().unwrap();
        assert_eq!(g.picture.complete, 1, "one whole picture arrived");
        assert_eq!(g.picture.keys(), frame, "every key got its slot's colour");
        assert_eq!(g.picture.mode, Some(13), "light mode 13 shows it");
        assert_eq!(userpic_count(&g), PER_KEY_PAGES, "seven pages once");
        assert_eq!(mode_count(&g), 1, "one mode report");
    }

    #[test]
    fn two_changes_inside_ten_seconds_upload_once_the_last() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let first = distinct_frame();
        hold(&mut a, &first, 0, 4700, t0);
        assert_eq!(s.lock().unwrap().picture.complete, 1, "the first still uploaded");
        // Two changes inside the 10 s after that upload (it went out at 2 s).
        let second = shifted_frame(40);
        let third = shifted_frame(80);
        hold(&mut a, &second, 5000, 7000, t0);
        hold(&mut a, &third, 7000, 12000, t0);
        assert_eq!(
            userpic_count(&s.lock().unwrap()),
            PER_KEY_PAGES,
            "the deferred still does not upload early"
        );
        // The gap ends at 12 s; the latest still goes out then.
        hold(&mut a, &third, 12000, 17000, t0);
        let g = s.lock().unwrap();
        assert_eq!(g.picture.complete, 2, "one upload before the changes and one after the gap");
        assert_eq!(g.picture.keys(), third, "the latest still is the one on the board");
        assert_eq!(userpic_count(&g), 2 * PER_KEY_PAGES, "two uploads, seven pages each");
    }

    #[test]
    fn moving_again_returns_to_the_whole_board_colour() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 4700, t0);
        assert_eq!(s.lock().unwrap().picture.mode, Some(13), "the still shows");
        let colours = color_count(&s.lock().unwrap());
        // Still frames leave the board alone: no whole-board colour while the picture shows.
        hold(&mut a, &frame, 4700, 6000, t0);
        assert_eq!(color_count(&s.lock().unwrap()), colours, "the colour is silent while the still shows");
        // The frame moves: direct mode and the whole-board colour go out in that write.
        let moving = shifted_frame(60);
        let before = s.lock().unwrap().sent.len();
        a.write_frame_at(&moving, at(t0, 6000)).expect("the moving frame");
        let g = s.lock().unwrap();
        let sent = &g.sent[before..];
        assert_eq!(sent.len(), 2, "direct mode, then the colour, at once");
        assert_eq!(sent[0].as_slice(), direct_mode_report());
        assert_eq!(sent[1].as_slice(), color_report(frame_color(&moving)));
        assert_eq!(g.picture.mode, Some(21), "the board is back in canvas-rgb's own mode");
    }

    #[test]
    fn a_distinct_colour_frame_reaches_the_right_keys_by_the_key_map() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let map = key_map();
        let frame: Vec<Rgb8> = (0..PER_KEY_SLOTS)
            .map(|slot| Rgb8::new((slot * 3) as u8, (255 - slot) as u8, (slot / 2) as u8))
            .collect();
        hold(&mut a, &frame, 0, 4700, t0);
        let g = s.lock().unwrap();
        assert_eq!(g.picture.complete, 1);
        let keys = g.picture.keys();
        // A is the second column's home-row key: row 3, column 1, slot 9 (checked on the device).
        assert_eq!(map[3 * KEY_COLS + 1], Some(9));
        assert_eq!(keys[9], frame[9]);
        // B is row 4, column 6, slot 40.
        assert_eq!(map[4 * KEY_COLS + 6], Some(40));
        assert_eq!(keys[40], frame[40]);
        // Every key the map names carries its slot's colour.
        for (entry, slot) in map.iter().enumerate() {
            if let Some(slot) = slot {
                assert_eq!(keys[*slot as usize], frame[*slot as usize], "map entry {entry}");
            }
        }
    }

    #[test]
    fn gaps_in_the_matrix_are_skipped() {
        let _serial = serial();
        let (a, _) = rig(true);
        let map = key_map();
        assert_eq!(map.len(), KEY_ROWS * KEY_COLS);
        assert_eq!(
            map.iter().filter(|entry| entry.is_none()).count(),
            KEY_ROWS * KEY_COLS - PER_KEY_SLOTS,
            "the four positions past slot 127 are gaps"
        );
        for row in 0..KEY_ROWS {
            for col in 0..KEY_COLS {
                assert_eq!(
                    map[row * KEY_COLS + col],
                    key_slot(row, col).map(|slot| slot as u32),
                    "row {row}, column {col}"
                );
            }
        }
        // The last column holds the last two slots on its top rows, and gaps below them.
        assert_eq!(map[21], Some(126));
        assert_eq!(map[KEY_COLS + 21], Some(127));
        assert_eq!(map[2 * KEY_COLS + 21], None);
        // The zone the editor sees has one LED per key, not one per position.
        let zone = &a.device_info().zones[0];
        assert_eq!(zone.led_count, PER_KEY_SLOTS as u32);
        assert_eq!(zone.kind, ZoneKind::Matrix { width: 22, height: 6, map });
    }

    #[test]
    fn a_refused_upload_falls_back_to_one_colour() {
        let _serial = serial();
        let logs = crate::test_logs::LogBuf::default();
        let _guard = logs.install();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 2100, t0);
        assert_eq!(userpic_count(&s.lock().unwrap()), 1, "page 0 went out");
        // The keyboard refuses page 1.
        s.lock().unwrap().fail_next_send = true;
        hold(&mut a, &frame, 2100, 2400, t0);
        let g = s.lock().unwrap();
        assert_eq!(g.picture.complete, 0, "no picture arrived");
        assert_ne!(g.picture.mode, Some(13), "the board never entered light mode 13");
        assert_eq!(
            g.sent.last().unwrap(),
            &color_report(frame_color(&frame)).to_vec(),
            "the whole-board colour took over"
        );
        assert!(g.sent.iter().rev().any(|r| r.as_slice() == direct_mode_report()), "direct mode again");
        drop(g);
        assert!(a.is_connected(), "a refused page does not cost the device");
        let text = logs.text();
        assert_eq!(text.matches("per-key picture").count(), 1, "the refusal is logged once: {text}");
    }

    #[test]
    fn device_id_and_zone_index_are_unchanged() {
        let (a, _) = rig(true);
        let d = a.device_info();
        assert_eq!(d.id, DeviceId("royuan:3151:4015".into()));
        assert_eq!(d.zones.len(), 1);
        let zone = &d.zones[0];
        assert_eq!(zone.index, 0, "the zone index stays 0");
        assert_eq!(zone.name, "Keyboard");
        assert_eq!(zone.kind, ZoneKind::Matrix { width: 22, height: 6, map: key_map() });
        assert_eq!(zone.led_count, PER_KEY_SLOTS as u32);
        assert_eq!(zone.min_leds, PER_KEY_SLOTS as u32);
        assert_eq!(zone.max_leds, PER_KEY_SLOTS as u32);
        assert!(!zone.resizable);
    }

    #[test]
    fn a_frame_longer_or_shorter_than_the_key_count_never_panics() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        for (i, len) in [0usize, 1, 127, 128, 129, 1000].into_iter().enumerate() {
            let frame: Vec<Rgb8> =
                (0..len).map(|n| Rgb8::new((n & 0xFF) as u8, 7, 9)).collect();
            let expected = color_report(frame_color(&frame)).to_vec();
            a.write_frame_at(&frame, at(t0, 500 * i as u64)).expect("a frame of any length");
            let g = s.lock().unwrap();
            assert_eq!(g.sent.last().unwrap(), &expected, "a {len}-colour frame averages");
        }
        assert_eq!(s.lock().unwrap().picture.complete, 0, "no still ran long enough to upload");
    }

    // ---- #312 review pass 2: flash wear, over an hour and over a day (B1, B2)

    #[test]
    fn a_dim_breathing_effect_uploads_a_handful_of_pictures_a_day() {
        let _serial = serial();
        // 5% of full brightness at 0.1 Hz: a 13-step swing over ten seconds, the effect the
        // review measured at 360 uploads an hour (one every ten seconds, all day).
        let dim = |t: f64| breathing_frame(t, 0.1, 0.05);
        // One picture, in the first hour, and never another: the frame moves away while its
        // picture is being written, so the abandon doubles the window to 4 s, and the swing
        // never holds within a step that long again. Before pass 2 the window never saw those
        // abandons at all, so this effect sat on the hourly cap forever.
        let hour = Wear { attempts: 1, pictures: 1, pages: PER_KEY_PAGES };
        assert_eq!(wear_of(HOUR_FRAMES, dim), hour, "a dim breathing effect in an hour");
        assert_eq!(wear_of(DAY_FRAMES, dim), hour, "and in a day");
    }

    #[test]
    fn a_slow_ramp_uploads_a_handful_of_pictures_a_day() {
        let _serial = serial();
        // A step every 2.5 s: a still holds within one step for five seconds, the review's 0.4
        // steps a second, 360 uploads an hour before the caps.
        let ramp = |t: f64| ramp_frame(t, 0.4);
        // Two: the first still uploads and shows; the frame moves, the window doubles to 4 s and
        // a second picture goes up; its abandon doubles to 8 s, which the ramp never holds.
        let day = Wear { attempts: 2, pictures: 2, pages: 2 * PER_KEY_PAGES };
        assert_eq!(wear_of(HOUR_FRAMES, ramp), day, "a slow ramp in an hour");
        assert_eq!(wear_of(DAY_FRAMES, ramp), day, "and in a day");
    }

    #[test]
    fn a_wide_slow_breathing_effect_uploads_a_handful_of_pictures_a_day() {
        let _serial = serial();
        // Black to white and back over 50 s: a peak holds within a step for about 2.8 s.
        let wide = |t: f64| breathing_frame(t, 0.02, 1.0);
        let day = Wear { attempts: 1, pictures: 1, pages: PER_KEY_PAGES };
        assert_eq!(wear_of(HOUR_FRAMES, wide), day, "a wide breathing effect in an hour");
        assert_eq!(wear_of(DAY_FRAMES, wide), day, "and in a day");
    }

    #[test]
    fn a_static_rainbow_uploads_once_a_day() {
        let _serial = serial();
        let frame = distinct_frame();
        let day = Wear { attempts: 1, pictures: 1, pages: PER_KEY_PAGES };
        assert_eq!(wear_of(HOUR_FRAMES, |_| frame.clone()), day, "a static rainbow in an hour");
        assert_eq!(wear_of(DAY_FRAMES, |_| frame.clone()), day, "and in a day");
    }

    /// A keyboard that leaves and comes back every five seconds, for `spans` of five seconds.
    fn flapping(spans: u64) -> Wear {
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        let frame = distinct_frame();
        for i in 0..spans {
            let t = i * 5000;
            a.open_and_init().expect("the keyboard came back");
            hold(&mut a, &frame, t, t + 2500, t0);
        }
        let g = s.lock().unwrap();
        Wear { attempts: upload_count(&g), pictures: g.picture.complete, pages: userpic_count(&g) }
    }

    #[test]
    fn a_flapping_device_uploads_at_most_the_caps() {
        let _serial = serial();
        // Each connection gets a fresh still window, so the gap and the caps are all that hold
        // it: four an hour, twenty-four a day, and a picture is never whole because the
        // keyboard leaves in the middle of its pages.
        assert_eq!(flapping(720).attempts, UPLOADS_PER_HOUR_MAX, "one hour of flapping");
        assert_eq!(flapping(17_280).attempts, UPLOADS_PER_DAY_MAX, "one day of flapping");
    }

    /// A still frame for `frames`, on a keyboard that refuses every picture page.
    fn refused_pages(frames: u64) -> (Wear, usize, bool, usize, usize) {
        let shared =
            Arc::new(Mutex::new(Shared { present: true, refuse_pages: true, ..Default::default() }));
        let mut a = RoyuanAdapter::with_opener(Box::new(FakeOpener(shared.clone())));
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        simulate(&mut a, t0, frames, |_| frame.clone());
        let connected = a.is_connected();
        let attempts = a.attempts.len();
        let g = shared.lock().unwrap();
        let wear = Wear { attempts, pictures: g.picture.complete, pages: userpic_count(&g) };
        (wear, g.refused, connected, direct_count(&g), color_count(&g))
    }

    #[test]
    fn a_keyboard_that_refuses_every_page_uploads_at_most_the_caps() {
        let _serial = serial();
        let logs = crate::test_logs::LogBuf::default();
        let _guard = logs.install();
        // Before pass 2 a refused page 0 was not counted and did not start the gap, so the next
        // frame tried the flash again: an erroring endpoint wrote once per frame.
        let (hour, hour_refused, connected, direct, colors) = refused_pages(HOUR_FRAMES);
        assert_eq!(
            hour_refused, UPLOADS_PER_HOUR_MAX,
            "page-0 sends in an hour with every page refused"
        );
        assert_eq!(hour.attempts, UPLOADS_PER_HOUR_MAX, "and the attempts they count as");
        assert_eq!(hour.pages, 0, "no page ever landed");
        assert!(connected, "a refused page does not cost the device");
        assert!(direct > 0, "direct mode goes back out");
        assert!(colors > 0, "and the whole-board colour streams again");
        let (day, day_refused, ..) = refused_pages(DAY_FRAMES);
        assert_eq!(day.attempts, UPLOADS_PER_DAY_MAX, "attempts in a day with every page refused");
        assert_eq!(day_refused, day.attempts, "one page-0 send per attempt, all day");
        assert_eq!(
            logs.text().matches("per-key picture").count(),
            2,
            "one refusal line per run, not one per attempt"
        );
    }

    // ---- #312 review pass 2: the counters and the picture across a restart (S1)

    #[test]
    fn a_picture_a_previous_run_wrote_is_shown_again_without_a_write() {
        let _serial = serial();
        let frame = distinct_frame();
        // The first run writes the picture and leaves the file behind.
        let store = FakeStore::with(StoredStills::default());
        {
            let (mut a, s) = rig_stored(true, store.clone());
            let t0 = Instant::now();
            a.open_and_init().expect("connect");
            hold(&mut a, &frame, 0, 4700, t0);
            assert_eq!(s.lock().unwrap().picture.complete, 1, "the first run wrote it");
        }
        let saved = store.saved();
        assert_eq!(saved.picture, Some(picture_hash(&frame)), "the file holds its hash");
        assert_eq!(saved.attempts.len(), 1, "and the attempt");
        // A daemon restart: the same still comes back, and only light mode 13 goes out.
        let (mut b, s) = rig_stored(true, store);
        let t0 = Instant::now();
        b.open_and_init().expect("connect");
        hold(&mut b, &frame, 0, 12_000, t0);
        let g = s.lock().unwrap();
        assert_eq!(upload_count(&g), 0, "the picture was not written again");
        assert_eq!(mode_count(&g), 1, "light mode 13 showed it");
        assert_eq!(g.picture.mode, Some(13));
    }

    #[test]
    fn the_attempts_a_previous_run_made_still_count() {
        let _serial = serial();
        let frame = distinct_frame();
        // Four attempts in the last hour: the hour is spent, so the first frees at 30 minutes.
        let now = unix_now();
        let stored = StoredStills {
            attempts: vec![now - 1800, now - 1200, now - 600, now - 60],
            picture: None,
        };
        let (mut a, s) = rig_stored(true, FakeStore::with(stored));
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        simulate(&mut a, t0, 25 * 60 * 10, |_| frame.clone());
        assert_eq!(upload_count(&s.lock().unwrap()), 0, "no attempt while the stored hour is spent");
        // The oldest stored attempt is half an hour old, so the hour frees at 30 minutes.
        simulate(&mut a, at(t0, 25 * 60 * 1000), 6 * 60 * 10, |_| frame.clone());
        assert_eq!(upload_count(&s.lock().unwrap()), 1, "an attempt once the hour frees");
        // The same over a day: twenty-four stored attempts spend the day, and only the oldest
        // ageing out of the rolling window (at about 55 minutes) frees one write.
        let stored = StoredStills {
            attempts: (0..24).map(|k| now - 3600 * k - 60).collect(),
            picture: None,
        };
        let (mut b, s) = rig_stored(true, FakeStore::with(stored));
        let t0 = Instant::now();
        b.open_and_init().expect("connect");
        simulate(&mut b, t0, DAY_FRAMES, |_| frame.clone());
        assert_eq!(upload_count(&s.lock().unwrap()), 1, "only the slot that ages out is used");
    }

    #[test]
    fn a_missing_or_broken_store_fails_closed_for_ten_minutes() {
        let _serial = serial();
        let frame = distinct_frame();
        let (mut a, s) = rig_stored(true, FakeStore::missing());
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        // Nothing may be written until the hold-off ends: a missing file cannot buy attempts.
        simulate(&mut a, t0, 6000, |_| frame.clone());
        assert_eq!(upload_count(&s.lock().unwrap()), 0, "no attempt inside the hold-off");
        // Ten minutes after the start, one still is written, all seven pages.
        simulate(&mut a, at(t0, 600_000), 4000, |_| frame.clone());
        assert_eq!(upload_count(&s.lock().unwrap()), 1, "an attempt once the hold-off ends");
        assert_eq!(userpic_count(&s.lock().unwrap()), PER_KEY_PAGES, "and it is whole");
    }

    #[test]
    fn a_file_store_round_trips_and_a_missing_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-stills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("royuan-stills.json");
        let store = FileStillStore::new(&path);
        assert!(store.load().is_err(), "a missing file does not load");
        let stored = StoredStills { attempts: vec![1_700_000_000, 1_700_000_010], picture: Some(42) };
        store.save(&stored).expect("the file is written");
        assert_eq!(store.load().expect("the file reads"), stored);
        // A file that will not parse is an error too, not a silent reset.
        std::fs::write(&path, b"{ not json").expect("the broken file is written");
        assert!(store.load().is_err(), "a broken file does not load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_still_the_flash_already_holds_is_shown_again_without_a_write() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 4700, t0);
        assert_eq!(s.lock().unwrap().picture.complete, 1, "the still uploaded");
        // The frame moves for one write and comes back: the flash still holds it.
        a.write_frame_at(&shifted_frame(60), at(t0, 4700)).expect("a moving frame");
        hold(&mut a, &frame, 4800, 13000, t0);
        let g = s.lock().unwrap();
        // The one write of the picture, and the mode report that showed it the second time.
        assert_eq!(upload_count(&g), 1, "the picture in the flash was not written again");
        assert_eq!(userpic_count(&g), PER_KEY_PAGES, "seven pages, once");
        assert_eq!(mode_count(&g), 2, "light mode 13 showed it again");
        assert_eq!(g.picture.mode, Some(13));
    }

    #[test]
    fn a_still_window_that_keeps_losing_its_picture_waits_longer() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let first = distinct_frame();
        hold(&mut a, &first, 0, 4700, t0);
        assert_eq!(s.lock().unwrap().picture.mode, Some(13), "the first still shows");
        // The effect moves away 100 ms after the picture appeared: the window doubles to 4 s.
        let mut step = 0u32;
        let mut ms = 4700;
        while ms < 12500 {
            a.write_frame_at(&shifted_frame((10 + step) as u8), at(t0, ms)).expect("a moving frame");
            step += 1;
            ms += 500;
        }
        // A still from 12.5 s: two seconds is no longer enough, four is.
        let second = shifted_frame(200);
        hold(&mut a, &second, 12500, 16000, t0);
        assert_eq!(upload_count(&s.lock().unwrap()), 1, "two seconds is not enough any more");
        hold(&mut a, &second, 16000, 17000, t0);
        assert_eq!(upload_count(&s.lock().unwrap()), 2, "four seconds is");
    }

    #[test]
    fn a_picture_that_stayed_a_minute_puts_the_window_back_to_two_seconds() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 4700, t0);
        // Away and back: the window doubles, then the picture shows again and stays.
        a.write_frame_at(&shifted_frame(60), at(t0, 4700)).expect("a moving frame");
        hold(&mut a, &frame, 4800, 80000, t0);
        assert_eq!(s.lock().unwrap().picture.mode, Some(13), "the picture shows again");
        // It stayed well past a minute, so the next still is back to two seconds.
        a.write_frame_at(&shifted_frame(120), at(t0, 80000)).expect("a moving frame");
        let next = shifted_frame(200);
        hold(&mut a, &next, 80500, 84000, t0);
        assert_eq!(upload_count(&s.lock().unwrap()), 2, "two seconds is enough again");
    }

    // ---- #312 review: a resume (S2)

    #[test]
    fn a_resume_with_the_picture_in_flash_shows_it_again_without_uploading() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 4700, t0);
        assert_eq!(s.lock().unwrap().picture.complete, 1, "the still uploaded");
        // The machine sleeps and wakes with the frame where it was.
        crate::note_resume();
        a.write_frame_at(&frame, at(t0, 5000)).expect("the frame after the resume");
        {
            let g = s.lock().unwrap();
            assert_eq!(upload_count(&g), 1, "the picture in the flash was not written again");
            assert_eq!(mode_count(&g), 2, "light mode 13 showed it again");
            assert_eq!(g.picture.mode, Some(13));
        }
        // The frame moved while the machine woke: the colour takes the board, still no write.
        let moved = shifted_frame(60);
        a.write_frame_at(&moved, at(t0, 5100)).expect("a moving frame");
        let g = s.lock().unwrap();
        let n = g.sent.len();
        assert_eq!(g.sent[n - 2].as_slice(), direct_mode_report(), "direct mode first");
        assert_eq!(g.sent[n - 1].as_slice(), color_report(frame_color(&moved)), "then the colour");
        assert_eq!(upload_count(&g), 1, "and still no write");
    }

    // ---- #312 review pass 2: a cut-off upload (S3, and B2's "an upload finishes")

    #[tokio::test]
    async fn an_upload_cut_off_at_page_three_is_never_shown_and_is_written_again() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.connect().await.expect("connect");
        let first = distinct_frame();
        hold(&mut a, &first, 0, 4700, t0);
        assert_eq!(s.lock().unwrap().picture.complete, 1, "the first picture is up");
        // Another still starts at 12 s; four of its pages go out before the keyboard goes away,
        // so the flash holds new pages over the picture that was there.
        let second = shifted_frame(60);
        hold(&mut a, &second, 4700, 12_400, t0);
        a.disconnect().await.expect("the keyboard goes");
        {
            let g = s.lock().unwrap();
            assert_eq!(userpic_count(&g), PER_KEY_PAGES + 4, "pages 0 to 3 of the second landed");
            assert_eq!(mode_count(&g), 1, "a torn picture is never shown");
        }
        a.connect().await.expect("the keyboard comes back");
        // The first still comes back. The flash holds half of another picture, so its seven
        // pages have to go again: light mode 13 would show a torn picture.
        hold(&mut a, &first, 12_500, 26_000, t0);
        let g = s.lock().unwrap();
        assert_eq!(userpic_count(&g), PER_KEY_PAGES + 4 + PER_KEY_PAGES, "all seven pages again");
        assert_eq!(g.picture.complete, 2, "and the first picture is whole on the board again");
        assert_eq!(g.picture.keys(), first);
        assert_eq!(mode_count(&g), 2, "light mode 13 only ever for a whole picture");
    }

    #[test]
    fn a_frame_that_moves_during_an_upload_does_not_stop_it() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        // The still crosses two seconds, then the frame moves before the pages are all out.
        hold(&mut a, &frame, 0, 2100, t0);
        assert_eq!(userpic_count(&s.lock().unwrap()), 1, "page 0 went out");
        a.write_frame_at(&shifted_frame(60), at(t0, 2150)).expect("a moving frame");
        hold(&mut a, &shifted_frame(60), 2300, 6000, t0);
        let g = s.lock().unwrap();
        // The picture was finished and shown, then the moving frame took the board back.
        assert_eq!(g.picture.complete, 1, "the whole picture was written");
        assert_eq!(userpic_count(&g), PER_KEY_PAGES, "seven pages, none dropped");
        assert_eq!(mode_count(&g), 1, "and light mode 13 showed it");
        assert_eq!(g.picture.mode, Some(21), "before the colour took over");
    }

    // ---- #312 review: the look during an upload (S4)

    #[test]
    fn the_whole_board_colour_keeps_streaming_until_the_picture_shows() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 2100, t0);
        assert_eq!(userpic_count(&s.lock().unwrap()), 1, "page 0 went out at two seconds");
        let during = color_count(&s.lock().unwrap());
        // The pages go out over the next 2.5 s, and the colour keeps streaming meanwhile.
        hold(&mut a, &frame, 2100, 4500, t0);
        assert_eq!(mode_count(&s.lock().unwrap()), 0, "the picture is not shown yet");
        let more = color_count(&s.lock().unwrap());
        assert!(more > during, "the whole-board colour kept streaming: {during} to {more}");
        // Light mode 13 goes out at 4.6 s, and the board is the picture's from then.
        hold(&mut a, &frame, 4600, 6000, t0);
        assert_eq!(mode_count(&s.lock().unwrap()), 1, "the picture shows");
        assert_eq!(color_count(&s.lock().unwrap()), more, "and the colour stops");
    }

    // ---- #312 review: nits

    #[test]
    fn a_one_step_drift_is_the_same_still_and_three_steps_is_a_new_one() {
        let frame = grey_frame(100);
        assert!(!frames_differ(&frame, &grey_frame(101)), "one step is the same still");
        assert!(frames_differ(&frame, &grey_frame(103)), "three steps is a new frame");
        assert!(!frames_differ(&frame, &grey_frame(99)), "and one step down is still one step");
        assert!(frames_differ(&frame, &vec![Rgb8::new(100, 100, 100); PER_KEY_SLOTS - 1]), "a length change");
    }

    #[test]
    fn a_three_step_drift_starts_the_still_window_again() {
        let _serial = serial();
        // A one-step drift keeps the still going: it crosses two seconds at 2 s.
        let (mut a, s1) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        hold(&mut a, &grey_frame(100), 0, 1900, t0);
        a.write_frame_at(&grey_frame(101), at(t0, 2000)).expect("a drifted frame");
        assert_eq!(upload_count(&s1.lock().unwrap()), 1, "one step is the same still");
        // Three steps is a new still, and its window starts over.
        let (mut b, s2) = rig(true);
        let t0 = Instant::now();
        b.open_and_init().expect("connect");
        hold(&mut b, &grey_frame(100), 0, 1900, t0);
        b.write_frame_at(&grey_frame(103), at(t0, 1900)).expect("a changed frame");
        b.write_frame_at(&grey_frame(103), at(t0, 2000)).expect("a still 0.1 s along");
        assert_eq!(upload_count(&s2.lock().unwrap()), 0, "three steps is a new still");
        hold(&mut b, &grey_frame(103), 2100, 3900, t0);
        b.write_frame_at(&grey_frame(103), at(t0, 3900)).expect("two seconds of the new still");
        assert_eq!(upload_count(&s2.lock().unwrap()), 1, "and it uploads at two seconds");
    }

    #[test]
    fn a_refusal_on_a_later_page_falls_back_to_one_colour() {
        let _serial = serial();
        let logs = crate::test_logs::LogBuf::default();
        let _guard = logs.install();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 2400, t0);
        assert_eq!(userpic_count(&s.lock().unwrap()), 4, "pages 0 to 3 went out");
        // The keyboard refuses page 4, half way through the picture.
        s.lock().unwrap().fail_next_send = true;
        hold(&mut a, &frame, 2400, 2600, t0);
        let g = s.lock().unwrap();
        assert_eq!(g.picture.complete, 0, "no whole picture arrived");
        assert_eq!(mode_count(&g), 0, "light mode 13 never went out");
        let n = g.sent.len();
        assert_eq!(g.sent[n - 2].as_slice(), direct_mode_report(), "direct mode again");
        assert_eq!(g.sent[n - 1].as_slice(), color_report(frame_color(&frame)), "then the colour");
        drop(g);
        assert!(a.is_connected(), "a refused page does not cost the device");
        let text = logs.text();
        assert_eq!(text.matches("per-key picture").count(), 1, "the refusal is logged once: {text}");
        // And the torn picture is written out in full by the next still.
        hold(&mut a, &frame, 2600, 15000, t0);
        assert_eq!(userpic_count(&s.lock().unwrap()), 4 + PER_KEY_PAGES, "seven pages again");
    }

    #[test]
    fn ending_a_shown_picture_for_a_diagnostic_sends_direct_mode_first() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let t0 = Instant::now();
        a.open_and_init().expect("connect");
        let frame = distinct_frame();
        hold(&mut a, &frame, 0, 4700, t0);
        assert_eq!(s.lock().unwrap().picture.mode, Some(13), "the picture shows");
        // Identify: one white colour. In light mode 13 a colour is not shown, so the direct-mode
        // report has to go first.
        a.write_at(Rgb8::WHITE, at(t0, 4800)).expect("a diagnostic colour");
        let g = s.lock().unwrap();
        let n = g.sent.len();
        assert_eq!(g.sent[n - 2].as_slice(), direct_mode_report(), "direct mode first");
        assert_eq!(g.sent[n - 1].as_slice(), color_report(Rgb8::WHITE), "then the colour");
        assert_eq!(g.picture.mode, Some(21), "the board is back in direct mode");
    }
}
