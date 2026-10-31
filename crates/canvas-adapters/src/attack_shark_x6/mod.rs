//! The Attack Shark X6 mouse (2.4G receiver, USB `1D57:FA60`) as a canvas-rgb peripheral.
//!
//! The X6 speaks the Beken-family HID feature-report protocol the vendor app `X6.exe` uses.
//! Everything here was decoded in `peripheral-hub`'s research (the report
//! `peripheral-hub/docs/research/attack-shark-x6.md`, read in full before this port); this
//! module is the canvas-rgb side of that protocol:
//!
//! - `codec.rs` is the port of the codec: pure bytes in, pure bytes out, no I/O anywhere in it.
//! - This file is the adapter: it finds the mouse, describes what the mouse has, reads its
//!   settings with the `0xA0` read-back handshake, decodes the status events, and can encode a
//!   config write of any setting. **Config writes are off by default**: the write path is behind
//!   [`AttackSharkX6::writes_enabled`], which is **false** until a caller turns it on, and nothing
//!   in this repository turns it on.
//!
//! # The value model
//!
//! A setting is a property, not a string. [`PeripheralAdapter::describe`] says what the mouse
//! has: sections, properties, and for every property its range, its step or its list of options,
//! taken from the research. [`PeripheralAdapter::read_settings`] gives the current values in a
//! [`Props`] map keyed by [`PropertyDef::key`], and
//! [`PeripheralAdapter::write_settings`] takes the same map back. The editor renders a control
//! per property out of the schema, the way it already does for an effect.
//!
//! The properties are flat. Nothing is packed into a string: the DPI table is eight numbers and
//! eight colours, not one `800@#FF0000,...` blob.
//!
//! | Section | Properties |
//! |---|---|
//! | DPI | `dpi-stage-count`, `dpi-stage-1` … `dpi-stage-8`, `dpi-stage-1-colour` … `dpi-stage-8-colour`, `dpi-active-stage` (read-only) |
//! | Performance | `polling`, `debounce-ms` |
//! | Lighting | `led-mode`, `led-colour`, `led-brightness`, `led-speed` |
//! | Power | `sleep-after-min`, `deep-sleep-min` |
//! | Buttons | `button-1`, `button-2`, `button-3`, `button-4`, `button-7`, `button-8` (read-only) |
//! | Profile | `profile` (read-only) |
//!
//! # Which HID collection is which
//!
//! The receiver (`1D57:FA60`, "2.4G Wireless Device" by Beken) exposes seven HID collections on
//! USB interface 2 (MI_02). Two of them matter, and they are told apart by their usage page,
//! never by their interface number (research section 8.1: "Match by VID/PID plus caps (not by
//! interface number)"):
//!
//! | Collection | Usage page | Usage | What it carries |
//! |---|---|---|---|
//! | COL04 | `0x000B` (vendor defined) | `0x0000` | The configuration reports. Feature length 262. |
//! | COL03 | `0x000A` (vendor defined) | `0x0000` | The status events. Five-byte input reports. |
//!
//! COL04 is where the read-back lives: a feature-set of the eight-byte request
//! `A0 <rid> <len> 00 01 00 00 00`, then a feature-get of `A0` for the receiver's grant
//! (`A0 01`), then a feature-get of the report id itself. COL03 is where the battery arrives
//! (event `0x40`) and where the mouse's DPI button announces itself (event `0x10`).
//!
//! The reports go out with exactly their own length. hidapi pads every feature report to the
//! collection's `FeatureReportByteLength` (262 bytes on this device) before it calls
//! `HidD_SetFeature`, and the X6's read request is eight bytes (the live read log's
//! `SEND SET_FEATURE len=8` lines) while the vendor's config reports are 9 to 59 bytes (research
//! 3.2), so the config collection is opened here as well and the write path calls
//! `HidD_SetFeature` with the length it was given. The reads stay on hidapi:
//! `get_feature_report` does not pad.
//!
//! The other five collections are the keyboard, the mouse itself, the system-control and
//! consumer-control collections and a second keyboard. None of them is opened.
//!
//! # The wired path
//!
//! `X6.exe` hard-codes a second pair, `1D57:FA61`, for the USB-C cable, but nobody has seen
//! that PID on an X6 (research U1: candidates are FA61, FA55 and 0x2111). This adapter therefore
//! matches `1D57:FA60` only. [`AttackSharkX6::with_product_id`] exists for the day the cable's
//! PID is settled; the read-back on the wired path is itself UNKNOWN (research 3.5), so it would
//! need its own hardware test.
//!
//! # Threads
//!
//! Every hidapi call blocks: a read of the five reports spends about 3.25 s of quiet time, and
//! the handshake's 250 ms and 500 ms gaps are real waits. All of it runs on tokio's blocking pool
//! through `spawn_blocking`, never on a runtime worker. The open handles move into the blocking
//! closure and back out; if the awaiting future is dropped mid-call the handle is dropped with
//! it and the adapter reads as disconnected.
//!
//! # Config writes
//!
//! **Config writes are off by default.** [`AttackSharkX6::writes_enabled`] is **false** until a
//! caller turns it on, and nothing in this repository turns it on. A *settings read* does send a
//! feature report — the eight-byte `A0` read request — but that request changes no setting: it
//! asks the receiver to hand back what it already holds. When config writes are on they still
//! refuse what the device cannot take: the button map is read-only until the physical button
//! behind each firmware slot is known (research U12), and the bytes of report 04 that the research
//! leaves UNKNOWN (3, 4, 6, 7 and 49; research U5) are copied from the frame the device last
//! reported rather than invented.
//!
//! # The battery
//!
//! The read-back handshake does not carry the battery. It arrives as an event on COL03
//! (`03 10 40 <state> <level>`), and the level byte's scale is UNKNOWN (research U3: `0x0a` is
//! either 100 % or 10 %). The adapter reports the charging state and never invents a percentage.
//!
//! # The sleeping mouse
//!
//! The test mouse sleeps after half a minute (its own setting) and a sleeping mouse cannot be
//! reached over the dongle: every read comes back with the grant refused (`0x00`), and the
//! vendor app shows "Device Sleep!" and retries (research 3.3). That is not a protocol failure,
//! so it gets its own error: [`ASLEEP`], "the mouse is asleep: move it and try again".
//!
//! Before it gives up, the adapter waits for the mouse to wake: it watches COL03 for the
//! mouse's own events for up to five seconds (the vendor app's own retry budget per command)
//! and reads once more as soon as one arrives. While the mouse is awake the battery event comes
//! about every two seconds (research 3.4), so a mouse that wakes up is heard within a couple of
//! seconds. A mouse that stays asleep gets the asleep error, and the user's next move is the one
//! the message names.
//!
//! # What the protocol does not carry
//!
//! - **Lift-off distance.** The X6 build never sends it: the vendor app's row is
//!   `visible="false"` and the profile field is never written (research 2). It is not a property
//!   of this mouse at all: the adapter lists it as unreported and never invents a value for it.
//! - **4000 Hz polling.** The wire byte has no encoding for it (research U10). It is not offered.
//! - **LED modes 7..11.** The vendor app offers twelve modes; the firmware's support for the
//!   last five is UNKNOWN (research U6). Only the seven named modes are exposed.
//! - **Macros** (report `0x09`, three chunks): the delay encoding disagrees between the writer
//!   and the reader (research U13), so nothing here encodes them.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canvas_proto::color::Rgb8;
use canvas_proto::device::DeviceId;
use canvas_proto::props::{PropKind, PropValue, PropertyDef, Props};
use canvas_proto::traits::AdapterError;

pub mod codec;
pub(crate) mod hid_cache;

use self::codec::{Event, Kind, LightChange};

// The peripheral types and the `PeripheralAdapter` trait live in the spine
// (`canvas_proto::peripheral`, `canvas_proto::traits`).
pub use canvas_proto::peripheral::{
    PeripheralEvent, PeripheralInfo, PeripheralKind, PeripheralSchema, PeripheralSection,
};
pub use canvas_proto::traits::PeripheralAdapter;

// ------------------------------------------------------------------------------------- the mouse

/// The vendor id of the receiver (usb.ids: `1d57` is Xenta; the PIDs are Beken's).
pub const VENDOR_ID: u16 = 0x1D57;
/// The product id of the 2.4G receiver.
pub const PRODUCT_ID: u16 = 0xFA60;
/// The product name the receiver reports.
pub const PRODUCT_NAME: &str = "2.4G Wireless Device";
/// The manufacturer string the receiver reports.
pub const MANUFACTURER: &str = "Beken";
/// The name of the product canvas-rgb shows.
pub const DISPLAY_NAME: &str = "Attack Shark X6";

/// The usage page of the configuration collection (COL04): vendor defined.
pub const CONFIG_USAGE_PAGE: u16 = 0x000B;
/// The usage of the configuration collection (COL04).
pub const CONFIG_USAGE: u16 = 0x0000;
/// The usage page of the status collection (COL03): vendor defined.
pub const EVENT_USAGE_PAGE: u16 = 0x000A;
/// The usage of the status collection (COL03).
pub const EVENT_USAGE: u16 = 0x0000;

/// The feature buffer the research used: 262 bytes, the configuration collection's feature report
/// length (research section 3.1).
const FEATURE_BUFFER_LEN: usize = 262;
/// The status collection's input report: five bytes (research section 3.4).
const EVENT_REPORT_LEN: usize = 5;
/// Read attempts per report: send the request, check the grant, read the answer once. The
/// sequence is ATKsoft's; this adapter tries twice and does not re-poll, because each request and
/// each answer spends quiet time the firmware needs.
const GRANT_ATTEMPTS: u32 = 2;

/// The error the adapter gives when the mouse is asleep.
///
/// A sleeping mouse answers every read with a refused grant (`0x00`): the receiver cannot reach
/// it over the dongle (research 3.3: "A sleeping mouse cannot receive config over the dongle;
/// the vendor UI shows `sleep_device_text` ("Device Sleep!") and retries"). The daemon shows a
/// protocol message as it is, so this reaches the user word for word.
pub const ASLEEP: &str = "the mouse is asleep: move it and try again";

// ---------------------------------------------------------------------------------- property keys

const KEY_STAGE_COUNT: &str = "dpi-stage-count";
const KEY_ACTIVE_STAGE: &str = "dpi-active-stage";
const KEY_POLLING: &str = "polling";
const KEY_DEBOUNCE: &str = "debounce-ms";
const KEY_LIFT_OFF: &str = "lift-off";
const KEY_LED_MODE: &str = "led-mode";
const KEY_LED_COLOUR: &str = "led-colour";
const KEY_LED_BRIGHTNESS: &str = "led-brightness";
const KEY_LED_SPEED: &str = "led-speed";
const KEY_SLEEP_AFTER: &str = "sleep-after-min";
const KEY_DEEP_SLEEP: &str = "deep-sleep-min";
const KEY_PROFILE: &str = "profile";

/// The property key of a DPI stage, 1-based.
fn stage_key(index: usize) -> String {
    format!("dpi-stage-{}", index + 1)
}

/// The property key of a DPI stage's colour, 1-based.
fn stage_colour_key(index: usize) -> String {
    format!("dpi-stage-{}-colour", index + 1)
}

/// The property key of a button slot, in the vendor's own numbering.
fn button_key(slot: usize) -> String {
    format!("button-{}", slot + 1)
}

/// The properties that live in the lighting report.
const LIGHT_KEYS: [&str; 7] = [
    KEY_LED_MODE,
    KEY_LED_COLOUR,
    KEY_LED_BRIGHTNESS,
    KEY_LED_SPEED,
    KEY_SLEEP_AFTER,
    KEY_DEEP_SLEEP,
    KEY_DEBOUNCE,
];

// ---------------------------------------------------------------------------------- what is known

/// How many DPI stages the mouse has.
const STAGE_COUNT: usize = codec::STAGE_COUNT;
/// The lowest resolution a stage takes.
const DPI_MIN: f64 = 50.0;
/// The highest resolution a stage takes.
const DPI_MAX: f64 = 26_000.0;
/// The step a stage's resolution moves in.
const DPI_STEP: f64 = 50.0;

/// The resolutions the mouse held when the research read it (research 3.5): 800, 1200, 1600,
/// 3200, 5600 and 26000 DPI, the last two stages off. The research does not document a factory
/// DPI table, so these are the device's known values; the editor replaces them with the real ones
/// as soon as it reads the mouse.
const KNOWN_STAGE_DPI: [u32; STAGE_COUNT] = [800, 1200, 1600, 3200, 5600, 26000, 50, 50];
/// The stage colours the mouse held when the research read it (research 3.5).
const KNOWN_STAGE_COLOUR: [(u8, u8, u8); STAGE_COUNT] = [
    (0xFF, 0, 0),
    (0, 0xFF, 0),
    (0, 0, 0xFF),
    (0xFF, 0xFF, 0),
    (0, 0xFF, 0xFF),
    (0xFF, 0, 0xFF),
    (0xFF, 0x40, 0),
    (0xFF, 0xFF, 0xFF),
];
/// How many stages were on: the live read's mask was `0x3f`, six stages (research 3.5).
const KNOWN_STAGE_COUNT: usize = 6;
/// The active stage the live read reported (research 3.5): the DPI button moved the mouse to
/// stage 2 after the last Apply.
const KNOWN_ACTIVE_STAGE: u8 = 2;

/// The quiet times the mouse needs between its calls.
#[derive(Clone, Copy, Debug)]
struct Pace {
    /// Between a read request and the receiver's grant (research 3.5: 250 ms).
    grant_wait: Duration,
    /// Between one answer and the next request (the vendor app's 500 ms pause, research 3.3).
    report_pause: Duration,
    /// How long the adapter waits for a sleeping mouse to wake before it gives up.
    wake_wait: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            grant_wait: Duration::from_millis(250),
            report_pause: Duration::from_millis(500),
            wake_wait: Duration::from_secs(5),
        }
    }
}

/// The frames the device last reported, in write layout, so a write can change one field and
/// keep every other byte.
#[derive(Default)]
struct Cache {
    dpi: Option<Vec<u8>>,
    light: Option<Vec<u8>>,
    /// The DPI button was pressed after the DPI frame was read: that frame's active stage is out
    /// of date. The event has not been seen live (research 3.4), so it is announced and never
    /// written into the frame; the next DPI write waits for a fresh read.
    dpi_stale: bool,
}

fn refuse(message: impl Into<String>) -> AdapterError {
    AdapterError::Protocol(message.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn join_error(e: tokio::task::JoinError) -> AdapterError {
    refuse(format!("the X6 blocking task failed: {e}"))
}

/// Why a property cannot be written yet.
fn read_only_reason(key: &str) -> &'static str {
    if key.starts_with("button-") {
        "which physical button each firmware slot belongs to is not known yet (research U12)"
    } else if key == KEY_ACTIVE_STAGE {
        "the mouse changes the active stage with its own DPI button, and the vendor app's device \
         model lists it as a runtime readout with no write report (research 2)"
    } else if key == KEY_PROFILE {
        "the mouse holds one profile and the profile list lives on the host (research 2)"
    } else {
        "this adapter cannot write it yet"
    }
}

/// The settings the device holds that this adapter does not report as values.
fn unreported() -> Vec<String> {
    let mut out = Vec::new();
    // The twelve firmware slots the vendor app does not name: which physical button sits behind
    // each slot is UNKNOWN (research U12).
    let named: Vec<usize> = codec::BUTTON_SLOTS.iter().map(|(_, slot)| *slot).collect();
    for slot in 0..codec::SLOTS {
        if !named.contains(&slot) {
            out.push(button_key(slot));
        }
    }
    // The option bytes of report 04 whose meaning on the X6 firmware is not settled (research U5).
    out.push("ripple-control".into());
    out.push("angle-snap".into());
    out.push("motion-sync".into());
    // The vendor app has a lift-off setting; the X6's firmware never carries it (research 2), so
    // it is not a property of the mouse either.
    out.push(KEY_LIFT_OFF.into());
    out
}

/// The property keys this adapter cannot write yet.
fn read_only() -> Vec<String> {
    let mut out: Vec<String> = codec::BUTTON_SLOTS.iter().map(|(name, _)| (*name).to_string()).collect();
    out.push(KEY_ACTIVE_STAGE.into());
    out.push(KEY_PROFILE.into());
    out
}

/// Checks one value against the property's kind: the right shape, and inside the range or the
/// option list.
fn check_value(def: &PropertyDef, value: &PropValue) -> Result<(), AdapterError> {
    match (&def.kind, value) {
        (PropKind::Number { min, max, step, .. }, PropValue::Number(n)) => {
            if *n < *min || *n > *max {
                return Err(refuse(format!("{} is {n}; it takes {min} to {max}", def.key)));
            }
            if *step > 0.0 && ((*n - *min) / *step).fract() != 0.0 {
                return Err(refuse(format!("{} is {n}; it takes steps of {step}", def.key)));
            }
            Ok(())
        }
        (PropKind::Choice { options, .. }, PropValue::Choice(c)) => {
            if options.iter().any(|o| o == c) {
                Ok(())
            } else {
                Err(refuse(format!("{} has no option {c:?}", def.key)))
            }
        }
        (PropKind::Color { .. }, PropValue::Color(_)) => Ok(()),
        (PropKind::Bool { .. }, PropValue::Bool(_)) => Ok(()),
        (PropKind::Hue { .. }, PropValue::Hue(_)) => Ok(()),
        _ => Err(refuse(format!("{} does not take that kind of value", def.key))),
    }
}

// ------------------------------------------------------------------------------------ transport

/// The configuration collection (COL04): feature reports in and out.
trait FeatureTransport: Send + Sync {
    /// Sends one feature report (byte 0 is the report id).
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String>;
    /// Reads one feature report into `buf`. Byte 0 is the report id on the way in and on the way
    /// out. Returns the number of bytes the device filled.
    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String>;
}

/// The status collection (COL03): five-byte input reports.
trait EventTransport: Send + Sync {
    /// Reads one input report into `buf`. `Ok(0)` means nothing arrived within `timeout_ms`.
    fn read_input(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String>;
}

/// Finds and opens the mouse's two collections.
trait Opener: Send + Sync {
    /// True when the receiver is enumerated. Does not open it.
    fn present(&self) -> bool;
    /// Opens the configuration collection. `Ok(None)` when it is not there.
    fn open_config(&self) -> Result<Option<Box<dyn FeatureTransport>>, String>;
    /// Opens the status collection. `Ok(None)` when it is not there.
    fn open_events(&self) -> Result<Option<Box<dyn EventTransport>>, String>;
}

/// `HidDevice` is `Send` but not `Sync`; the mutex supplies `Sync` (it is never contended).
struct HidEvent(std::sync::Mutex<hidapi::HidDevice>);

impl EventTransport for HidEvent {
    fn read_input(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        dev.read_timeout(buf, timeout_ms).map_err(|e| e.to_string())
    }
}

// -------------------------------------------------------------------------------- exact writes

/// A Windows HID handle that sends a feature report with exactly the length it is given.
///
/// hidapi pads every feature report to the collection's `FeatureReportByteLength` (262 bytes on
/// this device) before it calls `HidD_SetFeature` (hidapi-2.6.7
/// `src/windows_native/mod.rs`: `send_feature_report` at :228, `AsyncState::fill_buffer` at
/// :102). The X6's read request is eight bytes (the live read log's `SEND SET_FEATURE len=8`
/// lines, `raw/hid_readgrant_log.txt`) and the vendor's config reports are 9 to 59 bytes
/// (research 3.2), so the config collection is opened here instead and a report goes out with
/// the length it was given.
///
/// The reads stay on hidapi: `get_feature_report` does not pad. It sends the report id in byte 0
/// and hands back what the device filled.
#[cfg(windows)]
struct ExactFeature {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// The handle is only ever used from the thread that holds the transport, and the transport sits
// behind a mutex; a raw pointer is not `Send` on its own.
#[cfg(windows)]
unsafe impl Send for ExactFeature {}
#[cfg(windows)]
unsafe impl Sync for ExactFeature {}

#[cfg(windows)]
impl ExactFeature {
    /// Opens the collection at `path` for feature writes.
    fn open(path: &std::ffi::CStr) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        // The device path is ASCII, but this is the conversion Windows itself expects.
        let text = std::str::from_utf8(path.to_bytes())
            .map_err(|_| "the device path is not UTF-8".to_string())?;
        let wide: Vec<u16> = std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // The research's own working path opened it exactly like this: read|write, shared
        // read|write, OPEN_EXISTING, no overlapped flag (`raw/scripts/hid_readgrant.py:48`).
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("CreateFileW failed: {}", std::io::Error::last_os_error()));
        }
        Ok(Self { handle })
    }

    /// Sends one feature report with exactly `data.len()` bytes.
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        use windows_sys::Win32::Devices::HumanInterfaceDevice::HidD_SetFeature;
        if data.is_empty() {
            return Err("a feature report needs at least its report id".into());
        }
        let ok = unsafe {
            HidD_SetFeature(self.handle, data.as_ptr() as *const std::ffi::c_void, data.len() as u32)
        };
        if !ok {
            return Err(format!("HidD_SetFeature failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ExactFeature {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

/// Everywhere else: sending a feature report of an exact length needs Windows' HID stack.
#[cfg(not(windows))]
struct ExactFeature;

#[cfg(not(windows))]
impl ExactFeature {
    fn open(_path: &std::ffi::CStr) -> Result<Self, String> {
        Err("sending a feature report of an exact length needs Windows".into())
    }

    fn send(&mut self, _data: &[u8]) -> Result<(), String> {
        Err("sending a feature report of an exact length needs Windows".into())
    }
}

/// The configuration collection (COL04): hidapi for the reads, a raw handle for the writes.
struct HidFeature {
    /// The hidapi device, for `get_feature`.
    device: std::sync::Mutex<hidapi::HidDevice>,
    /// The raw handle, for `send_feature`.
    writer: ExactFeature,
}

impl FeatureTransport for HidFeature {
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        // Exactly the bytes given: hidapi would pad the report to the collection's
        // FeatureReportByteLength (262 bytes) before it calls HidD_SetFeature.
        self.writer.send(data)
    }

    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let dev = self.device.get_mut().map_err(|e| e.to_string())?;
        dev.get_feature_report(buf).map_err(|e| e.to_string())
    }
}

/// What the opener needs from a HID context: `hidapi::HidApi` in the product, a counting fake in
/// the tests.
trait HidSource: hid_cache::Scan {
    /// The path of the collection with this usage page and usage, by vendor id and product id.
    fn find_path(
        &self,
        product_id: u16,
        usage_page: u16,
        usage: u16,
    ) -> Option<std::ffi::CString>;
    /// Opens the configuration collection at this path.
    fn open_config_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn FeatureTransport>, String>;
    /// Opens the status collection at this path.
    fn open_events_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn EventTransport>, String>;
}

impl HidSource for hidapi::HidApi {
    fn find_path(
        &self,
        product_id: u16,
        usage_page: u16,
        usage: u16,
    ) -> Option<std::ffi::CString> {
        self.device_list()
            .find(|d| {
                d.vendor_id() == VENDOR_ID
                    && d.product_id() == product_id
                    && d.usage_page() == usage_page
                    && d.usage() == usage
            })
            .map(|d| d.path().to_owned())
    }

    fn open_config_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn FeatureTransport>, String> {
        let dev = self.open_path(path).map_err(|e| e.to_string())?;
        // A second handle on the same collection, for the writes: hidapi pads a feature report
        // and the X6's read request is eight bytes (research 3.5).
        let writer = ExactFeature::open(path)?;
        Ok(Box::new(HidFeature { device: std::sync::Mutex::new(dev), writer }))
    }

    fn open_events_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn EventTransport>, String> {
        let dev = self.open_path(path).map_err(|e| e.to_string())?;
        Ok(Box::new(HidEvent(std::sync::Mutex::new(dev))))
    }
}

/// Finds the mouse by vendor id, product id and the collection's usage page and usage.
///
/// The interface number is deliberately not part of the match: the research says to match by
/// VID/PID plus caps, because the interface number moves between the receiver's firmware
/// revisions (research section 8.1).
///
/// One HID context serves every call (see `hid_cache`): `present()`, `open_config()` and
/// `open_events()` no longer each scan every HID device on the machine (issue #7).
struct HidOpener<S: HidSource = hidapi::HidApi> {
    product_id: u16,
    cache: hid_cache::HidCache<S>,
}

impl<S: HidSource> HidOpener<S> {
    fn new(product_id: u16) -> Self {
        Self { product_id, cache: hid_cache::HidCache::default() }
    }

    /// Finds the collection and opens it. A list that is a little old can name a path that is
    /// gone, or miss one that just arrived: both rescan once and try again.
    fn open_collection<T>(
        &self,
        usage_page: u16,
        usage: u16,
        open: impl Fn(&S, &std::ffi::CStr) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let attempt = |api: &S| match api.find_path(self.product_id, usage_page, usage) {
            None => Ok(None),
            Some(path) => open(api, &path).map(Some),
        };
        if let Ok(Ok(Some(found))) = self.cache.with(attempt) {
            return Ok(Some(found));
        }
        self.cache.with_fresh(attempt)?
    }
}

impl<S: HidSource> Opener for HidOpener<S> {
    fn present(&self) -> bool {
        self.cache
            .with(|api| api.find_path(self.product_id, CONFIG_USAGE_PAGE, CONFIG_USAGE).is_some())
            .unwrap_or(false)
    }

    fn open_config(&self) -> Result<Option<Box<dyn FeatureTransport>>, String> {
        self.open_collection(CONFIG_USAGE_PAGE, CONFIG_USAGE, |api, path| api.open_config_at(path))
    }

    fn open_events(&self) -> Result<Option<Box<dyn EventTransport>>, String> {
        self.open_collection(EVENT_USAGE_PAGE, EVENT_USAGE, |api, path| api.open_events_at(path))
    }
}

// -------------------------------------------------------------------------------- the handshake

/// Why one report could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ReadFailure {
    /// The receiver refused the read: it answered the grant with `0x00`. Every report refused
    /// this way means the mouse is asleep (research 3.3).
    Refused,
    /// Anything else: the grant never came, the answer was malformed, the checksum failed.
    Other(String),
}

impl ReadFailure {
    /// The message the failure carries.
    fn message(&self) -> String {
        match self {
            ReadFailure::Refused => {
                "the receiver did not grant the read (answered 0x00): the mouse is asleep".into()
            }
            ReadFailure::Other(why) => why.clone(),
        }
    }
}

/// What one pass over the five reports found.
#[derive(Default)]
struct ReadPass {
    /// The reports that were read.
    reports: Vec<(Kind, Vec<u8>)>,
    /// The first failure, when there was one.
    failure: Option<ReadFailure>,
    /// True when every report was refused the same way: the mouse answered every grant with
    /// `0x00`, which is what a sleeping mouse does.
    all_refused: bool,
}

/// Reads one report: request, grant, answer.
///
/// Blocking: call it from `spawn_blocking`. A receiver that refuses the grant or answers with a
/// buffer it did not refill is an error after the attempts.
fn read_report(
    config: &mut dyn FeatureTransport,
    kind: Kind,
    pace: Pace,
) -> Result<Vec<u8>, ReadFailure> {
    let mut last = ReadFailure::Other(String::from("no answer"));
    for attempt in 1..=GRANT_ATTEMPTS {
        config
            .send_feature(&codec::read_request(kind))
            .map_err(|e| {
                ReadFailure::Other(format!("the {} read request failed: {e}", kind.reply_id()))
            })?;
        std::thread::sleep(pace.grant_wait);
        let mut grant = [0u8; FEATURE_BUFFER_LEN];
        grant[0] = codec::GRANT_REPORT_ID;
        let n = config
            .get_feature(&mut grant)
            .map_err(|e| ReadFailure::Other(format!("the {} grant read failed: {e}", kind.reply_id())))?;
        if n < 2 || grant[0] != codec::GRANT_REPORT_ID {
            last = ReadFailure::Other(format!(
                "the grant answer for {} is {n} bytes and does not start with the grant report id",
                kind.reply_id()
            ));
            tracing::warn!(report = kind.reply_id(), "{}", last.message());
            continue;
        }
        if grant[1] != 0x01 {
            last = if grant[1] == 0x00 {
                ReadFailure::Refused
            } else {
                ReadFailure::Other(format!(
                    "the receiver did not grant the {} read (answered {:#04x})",
                    kind.reply_id(),
                    grant[1]
                ))
            };
            tracing::warn!(report = kind.reply_id(), "{}", last.message());
            continue;
        }
        // One poll per grant. A bad answer is retried with a new request, not by polling again.
        let mut frame = [0u8; FEATURE_BUFFER_LEN];
        frame[0] = kind.rid();
        let n = config
            .get_feature(&mut frame)
            .map_err(|e| ReadFailure::Other(format!("the {} read failed: {e}", kind.reply_id())))?;
        match codec::check_reply(kind, &frame[..n.max(kind.reply_len())]) {
            Ok(()) => return Ok(frame.to_vec()),
            Err(why) => last = ReadFailure::Other(why),
        }
        tracing::warn!(report = kind.reply_id(), attempt, "{}", last.message());
    }
    Err(last)
}

/// Reads every report the adapter reads, in the order of the approved live test.
///
/// Blocking: call it from `spawn_blocking`. A report that could not be read leaves its own
/// properties unread; the other reports are still read. The first failure is kept, for the case
/// where nothing at all could be read.
fn read_all(config: &mut dyn FeatureTransport, pace: Pace) -> ReadPass {
    let mut out = ReadPass::default();
    let order = codec::READ_ORDER;
    let mut refused = 0usize;
    for (i, kind) in order.iter().enumerate() {
        match read_report(config, *kind, pace) {
            Ok(frame) => out.reports.push((*kind, frame)),
            Err(failure) => {
                tracing::warn!(report = kind.reply_id(), "{}", failure.message());
                if matches!(failure, ReadFailure::Refused) {
                    refused += 1;
                }
                if out.failure.is_none() {
                    out.failure = Some(failure);
                }
            }
        }
        if i + 1 < order.len() {
            std::thread::sleep(pace.report_pause);
        }
    }
    out.all_refused = out.reports.is_empty() && refused == order.len();
    out
}

/// Waits for the mouse to wake, watching the status collection for its events.
///
/// A mouse that is asleep sends nothing at all. While it is awake the battery event arrives
/// about every two seconds (research 3.4, the live capture's `03 10 40 01 0a` every two
/// seconds), so the first event after it wakes says it is back. Returns true when one arrived.
///
/// Blocking: call it from `spawn_blocking`.
fn wait_for_wake(events: &mut dyn EventTransport, wait: Duration) -> bool {
    let deadline = std::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        // At least one millisecond, so a zero wait still takes one look.
        let timeout_ms = left.as_millis().clamp(1, 200) as i32;
        let mut buf = [0u8; EVENT_REPORT_LEN];
        match events.read_input(&mut buf, timeout_ms) {
            Ok(0) => {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
            }
            Ok(n) => match codec::decode_event(&buf[..n]) {
                // Any report the mouse sends proves it is awake: it only sends while it is.
                Ok(_) => return true,
                Err(why) => {
                    tracing::warn!("the X6 sent a malformed status report while waiting: {why}")
                }
            },
            Err(e) => {
                tracing::warn!("the X6 status read failed while waiting for it to wake: {e}");
                return false;
            }
        }
    }
}

// ----------------------------------------------------------------------------- reading a report

/// The DPI report as properties.
fn read_dpi(frame: &[u8], props: &mut Props) {
    let stages = codec::stages_of(frame);
    let count = stages.iter().filter(|s| s.dpi.is_some()).count();
    props.insert(KEY_STAGE_COUNT.into(), PropValue::Number(count as f64));
    for (i, stage) in stages.iter().enumerate() {
        // A stage that is off holds zero in the device's bytes, which decodes as the range
        // minimum; the count says how many stages are on.
        let dpi = stage.dpi.unwrap_or_else(|| codec::stage_value(frame, i));
        props.insert(stage_key(i), PropValue::Number(f64::from(dpi)));
        props.insert(
            stage_colour_key(i),
            PropValue::Color(Rgb8::new(stage.rgb.0, stage.rgb.1, stage.rgb.2)),
        );
    }
    if let Some(active) = codec::active_stage_of(frame) {
        props.insert(KEY_ACTIVE_STAGE.into(), PropValue::Choice(active.to_string()));
    }
}

/// The lighting report as properties.
fn read_light(frame: &[u8], props: &mut Props) {
    let l = codec::light_of(frame);
    if let Some(mode) = l.mode {
        props.insert(KEY_LED_MODE.into(), PropValue::Choice(mode.to_string()));
    }
    props.insert(
        KEY_LED_COLOUR.into(),
        PropValue::Color(Rgb8::new(l.rgb.0, l.rgb.1, l.rgb.2)),
    );
    if let Some(b) = l.brightness {
        props.insert(KEY_LED_BRIGHTNESS.into(), PropValue::Number(f64::from(b)));
    }
    if let Some(s) = l.speed_slider {
        props.insert(KEY_LED_SPEED.into(), PropValue::Number(f64::from(s)));
    }
    if let Some(h) = l.sleep_half_min {
        props.insert(KEY_SLEEP_AFTER.into(), PropValue::Number(f64::from(h) / 2.0));
    }
    if let Some(m) = l.deep_sleep_min {
        props.insert(KEY_DEEP_SLEEP.into(), PropValue::Number(f64::from(m)));
    }
    if let Some(ms) = l.key_response_ms {
        props.insert(KEY_DEBOUNCE.into(), PropValue::Number(f64::from(ms)));
    }
}

/// The polling report as properties.
fn read_poll(frame: &[u8], props: &mut Props) {
    if let Some(id) = codec::polling_of(frame) {
        props.insert(KEY_POLLING.into(), PropValue::Choice(id.to_string()));
    }
}

/// The button report as properties: the rows the vendor app names.
fn read_buttons(frame: &[u8], props: &mut Props) {
    for (key, slot) in codec::BUTTON_SLOTS {
        props.insert(key.into(), PropValue::Choice(codec::action_of(frame, slot).to_string()));
    }
}

/// The profile report as properties: which profile the mouse is on. The mouse holds one profile
/// and the list of profiles lives on the host (research 2).
fn read_profile(frame: &[u8], props: &mut Props) {
    props.insert(KEY_PROFILE.into(), PropValue::Choice(frame[3].to_string()));
}

// --------------------------------------------------------------------------------- the adapter

/// The Attack Shark X6, seen through its 2.4G receiver.
pub struct AttackSharkX6 {
    opener: Arc<dyn Opener>,
    product_id: u16,
    config: Option<Box<dyn FeatureTransport>>,
    events: Option<Box<dyn EventTransport>>,
    /// False until a caller turns config writes on. Nothing is written to the hardware while it
    /// is false.
    writes_enabled: bool,
    pace: Pace,
    cache: Cache,
}

impl Default for AttackSharkX6 {
    fn default() -> Self {
        Self::new()
    }
}

impl AttackSharkX6 {
    /// The adapter for the 2.4G receiver, `1D57:FA60`.
    pub fn new() -> Self {
        Self::with_product_id(PRODUCT_ID)
    }

    /// The adapter for another product id. The wired cable's PID is UNKNOWN (research U1); this
    /// exists for the day it is settled.
    pub fn with_product_id(product_id: u16) -> Self {
        Self {
            opener: Arc::new(HidOpener::<hidapi::HidApi>::new(product_id)),
            product_id,
            config: None,
            events: None,
            writes_enabled: false,
            pace: Pace::default(),
            cache: Cache::default(),
        }
    }

    /// Turns config writes on or off. They are off until a caller says otherwise.
    pub fn set_writes_enabled(&mut self, on: bool) {
        self.writes_enabled = on;
    }

    /// The stable id of this mouse.
    pub fn device_id(&self) -> DeviceId {
        DeviceId(format!("attack-shark-x6:{VENDOR_ID:04x}:{:04x}", self.product_id))
    }

    fn check_id(&self, id: &DeviceId) -> Result<(), AdapterError> {
        if *id == self.device_id() {
            Ok(())
        } else {
            Err(AdapterError::UnknownDevice(id.clone()))
        }
    }

    fn info(&self) -> PeripheralInfo {
        PeripheralInfo {
            id: self.device_id(),
            name: DISPLAY_NAME.into(),
            vendor: MANUFACTURER.into(),
            kind: PeripheralKind::Mouse,
            adapter: "attack-shark-x6".into(),
            serial: String::new(),
            location: format!(
                "hid:{VENDOR_ID:04x}:{:04x}:usage{:04x}",
                self.product_id, CONFIG_USAGE_PAGE
            ),
            connected: self.config.is_some(),
        }
    }

    /// What the mouse has: its sections, its properties and their ranges.
    ///
    /// The defaults are the values the research's live read returned for this unit (research 3.5).
    /// The editor replaces them with the device's real values as soon as it reads the mouse; they
    /// are only what it shows before that.
    pub fn schema(&self) -> PeripheralSchema {
        let mut sections = Vec::new();

        // DPI: the stages, their colours and the active one.
        let mut dpi = vec![PropertyDef {
            key: KEY_STAGE_COUNT.into(),
            label: "DPI stages in use".into(),
            kind: PropKind::Number {
                min: 1.0,
                max: STAGE_COUNT as f64,
                step: 1.0,
                default: KNOWN_STAGE_COUNT as f64,
            },
        }];
        for i in 0..STAGE_COUNT {
            dpi.push(PropertyDef {
                key: stage_key(i),
                label: format!("DPI stage {}", i + 1),
                kind: PropKind::Number {
                    min: DPI_MIN,
                    max: DPI_MAX,
                    step: DPI_STEP,
                    default: f64::from(KNOWN_STAGE_DPI[i]),
                },
            });
            let (r, g, b) = KNOWN_STAGE_COLOUR[i];
            dpi.push(PropertyDef {
                key: stage_colour_key(i),
                label: format!("DPI stage {} colour", i + 1),
                kind: PropKind::Color { default: Rgb8::new(r, g, b) },
            });
        }
        dpi.push(PropertyDef {
            key: KEY_ACTIVE_STAGE.into(),
            label: "Active DPI stage".into(),
            kind: PropKind::Choice {
                options: (1..=STAGE_COUNT).map(|n| n.to_string()).collect(),
                default: KNOWN_ACTIVE_STAGE.to_string(),
            },
        });
        sections.push(PeripheralSection { id: "dpi".into(), title: "DPI".into(), properties: dpi });

        // Performance: how fast the mouse reports, how quickly it reacts, how far it lifts.
        let performance = vec![
            PropertyDef {
                key: KEY_POLLING.into(),
                label: "Polling rate (Hz)".into(),
                kind: PropKind::Choice {
                    options: codec::POLLING.iter().map(|(id, _)| (*id).to_string()).collect(),
                    default: "1000".into(),
                },
            },
            PropertyDef {
                key: KEY_DEBOUNCE.into(),
                label: "Key response time (ms)".into(),
                kind: PropKind::Number { min: 4.0, max: 50.0, step: 2.0, default: 8.0 },
            },
        ];
        sections.push(PeripheralSection {
            id: "performance".into(),
            title: "Performance".into(),
            properties: performance,
        });

        // Lighting.
        let lighting = vec![
            PropertyDef {
                key: KEY_LED_MODE.into(),
                label: "LED mode".into(),
                kind: PropKind::Choice {
                    options: codec::LED_MODES.iter().map(|m| (*m).to_string()).collect(),
                    default: "static".into(),
                },
            },
            PropertyDef {
                key: KEY_LED_COLOUR.into(),
                label: "LED colour".into(),
                kind: PropKind::Color { default: Rgb8::new(0xEF, 0x3F, 0xFF) },
            },
            PropertyDef {
                key: KEY_LED_BRIGHTNESS.into(),
                label: "LED brightness".into(),
                kind: PropKind::Number { min: 1.0, max: 8.0, step: 1.0, default: 8.0 },
            },
            PropertyDef {
                key: KEY_LED_SPEED.into(),
                label: "LED speed".into(),
                kind: PropKind::Number { min: 4.0, max: 8.0, step: 1.0, default: 6.0 },
            },
        ];
        sections.push(PeripheralSection {
            id: "lighting".into(),
            title: "Lighting".into(),
            properties: lighting,
        });

        // Power: when the mouse goes to sleep.
        let power = vec![
            PropertyDef {
                key: KEY_SLEEP_AFTER.into(),
                label: "Sleep after (min)".into(),
                kind: PropKind::Number { min: 0.5, max: 30.0, step: 0.5, default: 0.5 },
            },
            PropertyDef {
                key: KEY_DEEP_SLEEP.into(),
                label: "Deep sleep after (min)".into(),
                kind: PropKind::Number { min: 1.0, max: 60.0, step: 1.0, default: 10.0 },
            },
        ];
        sections.push(PeripheralSection {
            id: "power".into(),
            title: "Power".into(),
            properties: power,
        });

        // Buttons: the rows the vendor app names. Which physical button sits behind each slot is
        // UNKNOWN (research U12), so all of them are read-only.
        let mut buttons = Vec::new();
        for ((key, _slot), action) in codec::BUTTON_SLOTS.iter().zip(KNOWN_BUTTON_ACTION) {
            buttons.push(PropertyDef {
                key: (*key).into(),
                label: format!("Button {}", key.trim_start_matches("button-")),
                kind: PropKind::Choice { options: button_options(), default: action.to_string() },
            });
        }
        sections.push(PeripheralSection {
            id: "buttons".into(),
            title: "Buttons".into(),
            properties: buttons,
        });

        // Profile: the mouse holds one, and the list of profiles lives on the host.
        let profile = vec![PropertyDef {
            key: KEY_PROFILE.into(),
            label: "Profile".into(),
            kind: PropKind::Choice { options: vec!["1".into()], default: "1".into() },
        }];
        sections.push(PeripheralSection {
            id: "profile".into(),
            title: "Profile".into(),
            properties: profile,
        });

        PeripheralSchema {
            device: self.device_id(),
            kind: PeripheralKind::Mouse,
            sections,
            read_only: read_only(),
            unreported: unreported(),
        }
    }

    /// Reads the status events the mouse sends: the battery and the DPI button.
    ///
    /// Reads until the device goes quiet for `timeout_ms`, so a 3000 ms window catches the battery
    /// event that arrives about every two seconds. Blocking work happens on the blocking pool.
    pub async fn read_events(&mut self, timeout_ms: i32) -> Result<Vec<PeripheralEvent>, AdapterError> {
        let Some(mut events) = self.events.take() else {
            return Err(AdapterError::NotConnected);
        };
        let (events, out, dpi_stage) = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            let mut dpi_stage = false;
            loop {
                let mut buf = [0u8; EVENT_REPORT_LEN];
                match events.read_input(&mut buf, timeout_ms) {
                    Ok(0) => break,
                    Ok(n) => match codec::decode_event(&buf[..n]) {
                        Ok(Event::Battery { charging, fully_charged, level_byte }) => {
                            // The level byte's scale is UNKNOWN (research U3), so no percentage is
                            // ever reported.
                            out.push(PeripheralEvent::Battery { level: None, charging });
                            if fully_charged {
                                out.push(PeripheralEvent::Notice {
                                    message: "fully-charged".into(),
                                });
                            }
                            if let Some(b) = level_byte {
                                out.push(PeripheralEvent::Notice {
                                    message: format!("battery-level-byte={b}"),
                                });
                            }
                        }
                        Ok(Event::DpiStage(stage)) => {
                            // Announce only. The cached frame is not touched: this event has not
                            // been seen live, so its stage number is not trusted into a frame a
                            // later write would persist. The frame is marked stale instead, so the
                            // next DPI write waits for a fresh read.
                            dpi_stage = true;
                            out.push(PeripheralEvent::Notice {
                                message: format!("active-dpi-stage={stage}"),
                            });
                        }
                        Ok(Event::Ignored) => {}
                        Err(why) => tracing::warn!("the X6 sent a malformed status report: {why}"),
                    },
                    Err(e) => {
                        tracing::warn!("the X6 status read failed: {e}");
                        break;
                    }
                }
            }
            (events, out, dpi_stage)
        })
        .await
        .map_err(join_error)?;
        self.events = Some(events);
        if dpi_stage {
            self.cache.dpi_stale = true;
        }
        Ok(out)
    }

    /// The frames a batch of changes needs, without sending anything.
    fn plan(&self, props: &Props) -> Result<Plan, AdapterError> {
        let mut dpi: Option<Vec<u8>> = None;
        let mut light: Option<Vec<u8>> = None;
        let mut poll: Option<Vec<u8>> = None;

        if props.keys().any(|k| k.starts_with("dpi-stage-")) {
            dpi = Some(self.plan_dpi(props)?);
        }
        if props.keys().any(|k| LIGHT_KEYS.contains(&k.as_str())) {
            light = Some(self.plan_light(props)?);
        }
        if let Some(PropValue::Choice(id)) = props.get(KEY_POLLING) {
            poll = Some(codec::polling_frame(id).map_err(refuse)?);
        }

        let mut packets = Vec::new();
        if dpi.is_some() || light.is_some() || poll.is_some() {
            // The vendor app sends the apply header first, then the reports that changed.
            packets.push(codec::APPLY_HEADER.to_vec());
        }
        packets.extend(dpi.clone());
        packets.extend(light.clone());
        packets.extend(poll);
        Ok(Plan { packets, dpi, light })
    }

    /// The DPI frame a batch of changes needs.
    fn plan_dpi(&self, props: &Props) -> Result<Vec<u8>, AdapterError> {
        if self.cache.dpi_stale {
            return Err(refuse(
                "the DPI button was pressed since the DPI settings were read; read the device first",
            ));
        }
        let Some(frame) = self.cache.dpi.clone() else {
            return Err(refuse("the DPI settings have not been read yet; read the device first"));
        };
        let mut frame = frame;
        let mut stages = codec::stages_of(&frame);
        // The count says how many stages are on; the vendor app writes them in order, so the
        // first `count` stages are the ones that stay on.
        let count = match props.get(KEY_STAGE_COUNT) {
            Some(PropValue::Number(n)) => *n as usize,
            _ => stages.iter().filter(|s| s.dpi.is_some()).count(),
        };
        // A stage beyond the count would be dropped on the way out: the device's mask only holds
        // the first `count` stages. Refuse it instead of pretending it was written.
        for i in 0..STAGE_COUNT {
            let key = stage_key(i);
            if props.contains_key(&key) && i >= count {
                return Err(refuse(format!(
                    "{key} is beyond the {count} stages in use; write {KEY_STAGE_COUNT} first"
                )));
            }
        }
        for (i, stage) in stages.iter_mut().enumerate() {
            let resolution = match props.get(&stage_key(i)) {
                Some(PropValue::Number(n)) => Some(*n as u32),
                // A stage that was off holds the range minimum in the device's bytes; that is
                // what it takes when it is switched on again.
                _ => stage.dpi.or_else(|| Some(codec::stage_value(&frame, i))),
            };
            stage.dpi = if i < count { resolution } else { None };
            if let Some(PropValue::Color(colour)) = props.get(&stage_colour_key(i)) {
                stage.rgb = (colour.r, colour.g, colour.b);
            }
        }
        codec::patch_stages(&mut frame, &stages).map_err(refuse)?;
        Ok(frame)
    }

    /// The lighting frame a batch of changes needs.
    fn plan_light(&self, props: &Props) -> Result<Vec<u8>, AdapterError> {
        let Some(frame) = self.cache.light.clone() else {
            return Err(refuse("the lighting settings have not been read yet; read the device first"));
        };
        let mut frame = frame;
        // The brightness only exists in the modes that take it: the static mode (index 1) and the
        // unnameable index 9 (research 3.2, byte 5). In every other mode the firmware ignores the
        // brightness and the vendor app sends the constant 8, so a brightness written there would
        // be dropped: it is refused instead.
        let mode = match props.get(KEY_LED_MODE) {
            Some(PropValue::Choice(id)) => codec::LED_MODES
                .iter()
                .position(|m| *m == id)
                .ok_or_else(|| refuse(format!("no LED mode {id:?}")))? as u8,
            _ => frame[3] >> 4,
        };
        if props.contains_key(KEY_LED_BRIGHTNESS) && !codec::mode_takes_brightness(mode) {
            let named = match codec::LED_MODES.get(usize::from(mode)).copied() {
                Some(name) => format!("{name:?}"),
                None => format!("{mode}"),
            };
            return Err(refuse(format!(
                "the LED mode {named} does not take a brightness; only the static mode does"
            )));
        }
        if props.contains_key(KEY_LED_MODE) && !codec::mode_takes_brightness(mode) {
            // The vendor app sends the constant 8 with every mode change, so the brightness byte
            // follows the mode. Not a refusal, but not silent either.
            tracing::warn!("the LED mode does not take a brightness; the vendor app sends the constant 8 with it");
        }
        let mut change = LightChange::default();
        if let Some(PropValue::Choice(id)) = props.get(KEY_LED_MODE) {
            let index = codec::LED_MODES
                .iter()
                .position(|m| *m == id)
                .ok_or_else(|| refuse(format!("no LED mode {id:?}")))?;
            change.mode = Some(index as u8);
        }
        if let Some(PropValue::Color(colour)) = props.get(KEY_LED_COLOUR) {
            change.rgb = Some((colour.r, colour.g, colour.b));
        }
        if let Some(PropValue::Number(n)) = props.get(KEY_LED_BRIGHTNESS) {
            change.brightness = Some(*n as u8);
        }
        if let Some(PropValue::Number(n)) = props.get(KEY_LED_SPEED) {
            change.speed_slider = Some(*n as u8);
        }
        if let Some(PropValue::Number(n)) = props.get(KEY_SLEEP_AFTER) {
            change.sleep_half_min = Some((*n * 2.0) as u8);
        }
        if let Some(PropValue::Number(n)) = props.get(KEY_DEEP_SLEEP) {
            change.deep_sleep_min = Some(*n as u8);
        }
        if let Some(PropValue::Number(n)) = props.get(KEY_DEBOUNCE) {
            change.key_response_ms = Some(*n as u8);
        }
        codec::patch_light(&mut frame, &change).map_err(refuse)?;
        Ok(frame)
    }

    /// Keeps the frame the device reported, so a later write can patch one field and keep every
    /// other byte. The button map is not kept: it is read-only (research U12), so nothing is
    /// ever written from it.
    fn store(&mut self, kind: Kind, frame: &[u8]) {
        match kind {
            Kind::Dpi => {
                self.cache.dpi_stale = false;
                self.cache.dpi = Some(codec::write_frame_from_reply(kind, frame));
            }
            Kind::Light => self.cache.light = Some(codec::write_frame_from_reply(kind, frame)),
            Kind::Poll | Kind::Buttons | Kind::Profile => {}
        }
    }
}

/// The frames a batch of changes needs, and the packets that carry them.
struct Plan {
    /// The packets, in the order they go out: the apply header, then the changed reports.
    packets: Vec<Vec<u8>>,
    /// The DPI frame, when it changed.
    dpi: Option<Vec<u8>>,
    /// The lighting frame, when it changed.
    light: Option<Vec<u8>>,
}

/// The options a button row offers: the plain actions plus "another action" for the slots that
/// hold a keyboard key, a shortcut or a macro.
fn button_options() -> Vec<String> {
    let mut out: Vec<String> = codec::ACTIONS.iter().map(|(id, _)| (*id).to_string()).collect();
    out.push(codec::OTHER_ACTION.to_string());
    out
}

/// The actions the mouse held when the research read it (research 3.5), in the order of
/// [`codec::BUTTON_SLOTS`].
const KNOWN_BUTTON_ACTION: [&str; 6] =
    ["left-click", "right-click", "middle-click", "dpi-cycle", "forward", "back"];

#[async_trait]
impl PeripheralAdapter for AttackSharkX6 {
    fn name(&self) -> &'static str {
        "attack-shark-x6"
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        if self.config.is_some() {
            return Ok(());
        }
        let opener = self.opener.clone();
        let (config, events) = tokio::task::spawn_blocking(move || {
            let config = opener
                .open_config()
                .map_err(AdapterError::Protocol)?
                .ok_or(AdapterError::NotConnected)?;
            // The status collection is a bonus: a mouse that only answers the read-back still works.
            let events = opener.open_events().map_err(AdapterError::Protocol)?;
            Ok::<_, AdapterError>((config, events))
        })
        .await
        .map_err(join_error)??;
        self.config = Some(config);
        self.events = events;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        self.config = None;
        self.events = None;
        self.cache = Cache::default();
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.config.is_some()
    }

    async fn discover(&mut self) -> Result<Vec<PeripheralInfo>, AdapterError> {
        let present = if self.config.is_some() {
            true
        } else {
            // Enumerates every HID device with hidapi (metadata only, no I/O), off the runtime.
            let opener = self.opener.clone();
            tokio::task::spawn_blocking(move || opener.present()).await.map_err(join_error)?
        };
        if present {
            Ok(vec![self.info()])
        } else {
            Ok(Vec::new())
        }
    }

    async fn describe(&mut self, device: &DeviceId) -> Result<PeripheralSchema, AdapterError> {
        self.check_id(device)?;
        Ok(self.schema())
    }

    async fn read_settings(&mut self, device: &DeviceId) -> Result<Props, AdapterError> {
        self.check_id(device)?;
        let Some(mut config) = self.config.take() else {
            return Err(AdapterError::NotConnected);
        };
        // The status collection comes along: it is the only way to hear that the mouse woke up.
        let mut events = self.events.take();
        let pace = self.pace;
        let (config, events, pass) = tokio::task::spawn_blocking(move || {
            let mut pass = read_all(&mut *config, pace);
            if pass.all_refused {
                // Every read was refused: the mouse is asleep. Wait for it to wake (any movement
                // wakes it) and read once more before giving up.
                if let Some(events) = events.as_mut() {
                    if wait_for_wake(&mut **events, pace.wake_wait) {
                        tracing::info!("the mouse sent a status event: it is awake. Reading again.");
                        pass = read_all(&mut *config, pace);
                    } else {
                        tracing::info!("the mouse stayed asleep");
                    }
                }
            }
            (config, events, pass)
        })
        .await
        .map_err(join_error)?;
        self.config = Some(config);
        self.events = events;
        if pass.reports.is_empty() {
            return Err(match pass.failure {
                // Every report was refused: the mouse is asleep. That is its own error, not a
                // protocol failure, and the message is the instruction the user needs.
                Some(ReadFailure::Refused) => refuse(ASLEEP),
                Some(ReadFailure::Other(why)) => {
                    refuse(format!("the mouse answered none of the five read requests: {why}"))
                }
                None => refuse("the mouse answered none of the five read requests: no answer"),
            });
        }

        let mut props = Props::new();
        for (kind, frame) in &pass.reports {
            self.store(*kind, frame);
            match kind {
                Kind::Dpi => read_dpi(frame, &mut props),
                Kind::Light => read_light(frame, &mut props),
                Kind::Poll => read_poll(frame, &mut props),
                Kind::Buttons => read_buttons(frame, &mut props),
                Kind::Profile => read_profile(frame, &mut props),
            }
        }
        Ok(props)
    }

    async fn write_settings(&mut self, device: &DeviceId, props: &Props) -> Result<(), AdapterError> {
        self.check_id(device)?;
        if !self.writes_enabled {
            return Err(AdapterError::Unsupported(
                "config writes are off: turn them on with set_writes_enabled(true) first",
            ));
        }
        if props.is_empty() {
            return Ok(());
        }
        let schema = self.schema();
        for (key, value) in props {
            let Some(def) = schema.property(key) else {
                return Err(refuse(format!("the mouse has no setting called {key}")));
            };
            if !schema.is_writable(key) {
                return Err(refuse(format!("{key} is read-only: {}", read_only_reason(key))));
            }
            check_value(def, value)?;
        }
        let plan = self.plan(props)?;
        if plan.packets.is_empty() {
            return Ok(());
        }
        let Some(mut config) = self.config.take() else {
            return Err(AdapterError::NotConnected);
        };
        let pace = self.pace;
        let packets = plan.packets;
        let (config, result) = tokio::task::spawn_blocking(move || {
            let mut result = Ok(());
            for (i, packet) in packets.iter().enumerate() {
                if i > 0 {
                    // The vendor app leaves 500 ms between commands; libratbag reports firmware
                    // lockup under 250 ms (research U11).
                    std::thread::sleep(pace.report_pause);
                }
                tracing::info!(bytes = packet.len(), raw = %hex(packet), "the X6 write goes out");
                if let Err(e) = config.send_feature(packet) {
                    result = Err(refuse(format!("the X6 write failed: {e}")));
                    break;
                }
            }
            (config, result)
        })
        .await
        .map_err(join_error)?;
        self.config = Some(config);
        result?;
        // The device now holds what we sent, so the next write builds on it instead of dropping
        // this one. A read re-syncs the cache from the device either way.
        if let Some(frame) = plan.dpi {
            self.cache.dpi = Some(frame);
        }
        if let Some(frame) = plan.light {
            self.cache.light = Some(frame);
        }
        Ok(())
    }

    fn writes_enabled(&self) -> bool {
        self.writes_enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::sync::Mutex;

    /// The replies the live read of 2026-10-17 returned: the fake device answers with the bytes
    /// in [`codec::evidence`], so the adapter's tests are checked against the same evidence as
    /// the codec's.
    fn live_replies() -> BTreeMap<u8, Vec<u8>> {
        let mut out = BTreeMap::new();
        out.insert(0x04, codec::evidence::REPLY_04.to_vec());
        out.insert(0x05, codec::evidence::REPLY_05.to_vec());
        out.insert(0x06, codec::evidence::REPLY_06.to_vec());
        out.insert(0x08, codec::evidence::REPLY_08.to_vec());
        out.insert(0x0c, codec::evidence::REPLY_0C.to_vec());
        out
    }

    #[derive(Default)]
    struct Shared {
        present: bool,
        replies: BTreeMap<u8, Vec<u8>>,
        /// Every feature report the adapter sent, in order.
        sent: Vec<Vec<u8>>,
        /// The grant answer: 1 grants the read.
        grant: u8,
        /// The grant the fake answers with once the mouse has woken up (the status collection
        /// handed over an event). `None` keeps answering `grant`.
        grant_when_awake: Option<u8>,
        /// True once the status collection handed over an event: the mouse is awake.
        awake: bool,
        /// Break the answer of this report id (flip a byte, so the checksum fails).
        break_report: Option<u8>,
        /// Refuse every answer, the way a sleeping mouse does.
        silent: bool,
        opens: u32,
        /// The status reports the fake hands over, in order.
        events: VecDeque<Vec<u8>>,
    }

    struct FakeOpener(Arc<Mutex<Shared>>);
    struct FakeFeature(Arc<Mutex<Shared>>);
    struct FakeEvents(Arc<Mutex<Shared>>);

    impl FeatureTransport for FakeFeature {
        fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
            let mut s = self.0.lock().unwrap();
            s.sent.push(data.to_vec());
            Ok(())
        }

        fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            let s = self.0.lock().unwrap();
            let rid = buf[0];
            if rid == codec::GRANT_REPORT_ID {
                let grant = match s.grant_when_awake {
                    Some(g) if s.awake => g,
                    _ => s.grant,
                };
                buf[..2].copy_from_slice(&[codec::GRANT_REPORT_ID, grant]);
                return Ok(2);
            }
            if s.silent {
                return Ok(0);
            }
            let Some(reply) = s.replies.get(&rid).cloned() else {
                return Ok(0);
            };
            let mut reply = reply;
            if s.break_report == Some(rid) {
                // A flipped byte breaks the checksum or the complement.
                let at = if rid == 0x0c { 4 } else { 3 };
                reply[at] ^= 0x10;
            }
            let n = reply.len().min(buf.len());
            buf[..n].copy_from_slice(&reply[..n]);
            Ok(n)
        }
    }

    impl EventTransport for FakeEvents {
        fn read_input(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            let mut s = self.0.lock().unwrap();
            match s.events.pop_front() {
                None => Ok(0),
                Some(report) => {
                    // The mouse only sends while it is awake: handing a report over wakes it.
                    s.awake = true;
                    let n = report.len().min(buf.len());
                    buf[..n].copy_from_slice(&report[..n]);
                    Ok(n)
                }
            }
        }
    }

    impl Opener for FakeOpener {
        fn present(&self) -> bool {
            self.0.lock().unwrap().present
        }

        fn open_config(&self) -> Result<Option<Box<dyn FeatureTransport>>, String> {
            let mut s = self.0.lock().unwrap();
            if !s.present {
                return Ok(None);
            }
            s.opens += 1;
            Ok(Some(Box::new(FakeFeature(self.0.clone()))))
        }

        fn open_events(&self) -> Result<Option<Box<dyn EventTransport>>, String> {
            let s = self.0.lock().unwrap();
            if !s.present {
                return Ok(None);
            }
            Ok(Some(Box::new(FakeEvents(self.0.clone()))))
        }
    }

    /// The adapter over a fake receiver that answers with the live read's bytes.
    fn rig() -> (AttackSharkX6, Arc<Mutex<Shared>>) {
        let shared = Arc::new(Mutex::new(Shared {
            present: true,
            replies: live_replies(),
            grant: 1,
            ..Default::default()
        }));
        let mut adapter = AttackSharkX6::new();
        adapter.opener = Arc::new(FakeOpener(shared.clone()));
        adapter.pace = Pace {
            grant_wait: Duration::ZERO,
            report_pause: Duration::ZERO,
            wake_wait: Duration::ZERO,
        };
        (adapter, shared)
    }

    fn props_of(pairs: &[(&str, PropValue)]) -> Props {
        pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
    }

    fn number(pairs: &[(&str, f64)]) -> Props {
        props_of(&pairs.iter().map(|(k, v)| (*k, PropValue::Number(*v))).collect::<Vec<_>>())
    }

    fn choice(pairs: &[(&str, &str)]) -> Props {
        props_of(&pairs.iter().map(|(k, v)| (*k, PropValue::Choice((*v).into()))).collect::<Vec<_>>())
    }

    // ------------------------------------------------------------ the HID context (issue #7)

    use std::cell::Cell;

    thread_local! {
        /// Full scans of the fake machine on this thread (each test runs on its own thread).
        static SCANS: Cell<u32> = const { Cell::new(0) };
        /// Opens that fail because the path in the list is stale, before they work again.
        static STALE_OPENS: Cell<u32> = const { Cell::new(0) };
    }

    /// A machine whose HID list is counted: a `HidApi` that scans nothing real.
    struct CountingApi;

    impl hid_cache::Scan for CountingApi {
        fn first() -> Result<Self, String> {
            SCANS.with(|s| s.set(s.get() + 1));
            Ok(CountingApi)
        }

        fn rescan(&mut self) -> Result<(), String> {
            SCANS.with(|s| s.set(s.get() + 1));
            Ok(())
        }
    }

    struct NullFeature;

    impl FeatureTransport for NullFeature {
        fn send_feature(&mut self, _data: &[u8]) -> Result<(), String> {
            Ok(())
        }

        fn get_feature(&mut self, _buf: &mut [u8]) -> Result<usize, String> {
            Ok(0)
        }
    }

    struct NullEvents;

    impl EventTransport for NullEvents {
        fn read_input(&mut self, _buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            Ok(0)
        }
    }

    impl HidSource for CountingApi {
        fn find_path(&self, product_id: u16, usage_page: u16, _usage: u16) -> Option<std::ffi::CString> {
            (product_id == PRODUCT_ID).then(|| std::ffi::CString::new(format!("fake/{usage_page:04x}")).unwrap())
        }

        fn open_config_at(&self, _path: &std::ffi::CStr) -> Result<Box<dyn FeatureTransport>, String> {
            if STALE_OPENS.with(|s| s.replace(s.get().saturating_sub(1)) > 0) {
                return Err("the path is gone".into());
            }
            Ok(Box::new(NullFeature))
        }

        fn open_events_at(&self, _path: &std::ffi::CStr) -> Result<Box<dyn EventTransport>, String> {
            Ok(Box::new(NullEvents))
        }
    }

    fn scans() -> u32 {
        SCANS.with(Cell::get)
    }

    #[test]
    fn many_present_and_open_calls_scan_the_machine_once() {
        SCANS.with(|s| s.set(0));
        let opener = HidOpener::<CountingApi>::new(PRODUCT_ID);
        // The Peripherals view's calls, over and over: a discover (present), a connect (the two
        // opens), and the same again.
        for _ in 0..4 {
            assert!(opener.present());
            assert!(opener.open_config().unwrap().is_some());
            assert!(opener.open_events().unwrap().is_some());
        }
        assert_eq!(scans(), 1, "twelve calls, one scan of the HID devices (old code: twelve)");
    }

    #[test]
    fn a_path_that_no_longer_opens_rescans_once() {
        SCANS.with(|s| s.set(0));
        STALE_OPENS.with(|s| s.set(1));
        let opener = HidOpener::<CountingApi>::new(PRODUCT_ID);
        assert!(opener.present());
        assert!(opener.open_config().unwrap().is_some(), "the retry opens it");
        assert_eq!(scans(), 2, "the first scan, and one rescan for the stale path");
    }

    #[test]
    fn a_receiver_that_is_not_there_is_not_found_after_one_rescan() {
        SCANS.with(|s| s.set(0));
        let opener = HidOpener::<CountingApi>::new(PRODUCT_ID + 1);
        assert!(!opener.present());
        assert!(opener.open_config().unwrap().is_none());
        assert_eq!(scans(), 2, "present() scans; the open double-checks with one rescan");
    }

    /// The sends of one view-open as the daemon makes them: the list route's discover, the
    /// detail route's discover and describe, then the connect and the read.
    #[tokio::test]
    async fn one_view_open_sends_one_round_of_read_requests() {
        let (mut a, s) = rig();
        let id = a.device_id();
        a.discover().await.unwrap();
        a.discover().await.unwrap();
        a.describe(&id).await.unwrap();
        a.connect().await.unwrap();
        a.read_settings(&id).await.unwrap();
        let sent = s.lock().unwrap().sent.len();
        assert_eq!(
            sent,
            codec::READ_ORDER.len(),
            "one read request per report, nothing sent by the discovers, describe or connect"
        );
        assert_eq!(s.lock().unwrap().opens, 1);
    }

    // ------------------------------------------------------------------------------ discovery

    #[tokio::test]
    async fn discover_is_empty_when_the_receiver_is_not_there() {
        let (mut a, s) = rig();
        s.lock().unwrap().present = false;
        assert!(a.discover().await.unwrap().is_empty());
        assert!(matches!(a.connect().await, Err(AdapterError::NotConnected)));
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn discover_names_the_mouse_and_does_not_open_it() {
        let (mut a, s) = rig();
        let found = a.discover().await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, DeviceId("attack-shark-x6:1d57:fa60".into()));
        assert_eq!(found[0].name, "Attack Shark X6");
        assert_eq!(found[0].kind, PeripheralKind::Mouse);
        assert_eq!(found[0].adapter, "attack-shark-x6");
        assert!(!found[0].connected);
        assert_eq!(s.lock().unwrap().opens, 0, "the enumeration opens nothing");
        assert_eq!(a.name(), "attack-shark-x6");
    }

    #[tokio::test]
    async fn connect_opens_the_two_collections_and_disconnect_closes_them() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        assert!(a.is_connected());
        assert_eq!(s.lock().unwrap().opens, 1);
        a.connect().await.unwrap();
        assert_eq!(s.lock().unwrap().opens, 1, "a second connect is a no-op");
        a.disconnect().await.unwrap();
        assert!(!a.is_connected());
    }

    // --------------------------------------------------------------------------------- describe

    #[tokio::test]
    async fn the_schema_has_the_sections_the_mouse_has() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        assert_eq!(schema.kind, PeripheralKind::Mouse);
        let ids: Vec<&str> = schema.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["dpi", "performance", "lighting", "power", "buttons", "profile"]);
        assert_eq!(schema.sections[0].title, "DPI");
    }

    #[tokio::test]
    async fn the_schema_carries_the_real_ranges_steps_and_options() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        let kind = |key: &str| schema.property(key).unwrap().kind.clone();
        assert_eq!(
            kind("dpi-stage-1"),
            PropKind::Number { min: 50.0, max: 26000.0, step: 50.0, default: 800.0 }
        );
        assert_eq!(
            kind("dpi-stage-count"),
            PropKind::Number { min: 1.0, max: 8.0, step: 1.0, default: 6.0 }
        );
        assert_eq!(
            kind("polling"),
            PropKind::Choice { options: vec!["125".into(), "250".into(), "500".into(), "1000".into()], default: "1000".into() }
        );
        assert_eq!(
            kind("debounce-ms"),
            PropKind::Number { min: 4.0, max: 50.0, step: 2.0, default: 8.0 }
        );
        assert_eq!(
            kind("led-brightness"),
            PropKind::Number { min: 1.0, max: 8.0, step: 1.0, default: 8.0 }
        );
        assert_eq!(
            kind("sleep-after-min"),
            PropKind::Number { min: 0.5, max: 30.0, step: 0.5, default: 0.5 }
        );
        assert_eq!(
            kind("deep-sleep-min"),
            PropKind::Number { min: 1.0, max: 60.0, step: 1.0, default: 10.0 }
        );
        // The seven named LED modes, and no more: the last five are UNKNOWN on the X6 firmware.
        let PropKind::Choice { options, .. } = kind("led-mode") else { panic!("led-mode is a choice") };
        assert_eq!(options, vec!["off", "static", "breathing", "neon", "color-breathing", "static-dpi", "breathing-dpi"]);
        // The button rows offer the plain actions plus "another action".
        let PropKind::Choice { options, .. } = kind("button-1") else { panic!("button-1 is a choice") };
        assert!(options.contains(&"left-click".to_string()));
        assert!(options.contains(&"other".to_string()));
        assert_eq!(options.len(), 15);
    }

    #[tokio::test]
    async fn the_button_map_the_active_stage_and_the_profile_are_read_only() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        for (key, _) in codec::BUTTON_SLOTS {
            assert!(!schema.is_writable(key), "{key} must be read-only");
            assert!(schema.read_only.contains(&key.to_string()));
        }
        // The active stage: the mouse changes it with its own DPI button, and the vendor app's
        // device model lists it as a runtime readout with no write report.
        assert!(!schema.is_writable("dpi-active-stage"));
        // The profile: the mouse holds one, and the list of profiles lives on the host.
        assert!(!schema.is_writable("profile"));
        // Everything else can be written.
        for key in ["polling", "debounce-ms", "led-mode", "led-colour", "sleep-after-min", "dpi-stage-1"] {
            assert!(schema.is_writable(key), "{key} must be writable");
        }
    }

    #[tokio::test]
    async fn the_settings_that_cannot_be_named_are_listed_as_unreported() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        // The twelve button slots nobody can name (research U12).
        for slot in [5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18] {
            assert!(
                schema.unreported.contains(&format!("button-{slot}")),
                "button-{slot} is not listed"
            );
        }
        // The three option bytes of report 04 whose meaning is UNKNOWN (research U5).
        for field in ["ripple-control", "angle-snap", "motion-sync"] {
            assert!(schema.unreported.contains(&field.to_string()), "{field} is not listed");
        }
        // The vendor app's lift-off setting, which the X6's firmware never carries.
        assert!(schema.unreported.contains(&"lift-off".to_string()));
    }

    // ------------------------------------------------------------------------------- the read

    #[tokio::test]
    async fn the_read_returns_the_values_the_live_read_returned() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        assert_eq!(props.get("dpi-stage-count"), Some(&PropValue::Number(6.0)));
        assert_eq!(props.get("dpi-stage-1"), Some(&PropValue::Number(800.0)));
        assert_eq!(props.get("dpi-stage-2"), Some(&PropValue::Number(1200.0)));
        assert_eq!(props.get("dpi-stage-3"), Some(&PropValue::Number(1600.0)));
        assert_eq!(props.get("dpi-stage-4"), Some(&PropValue::Number(3200.0)));
        assert_eq!(props.get("dpi-stage-5"), Some(&PropValue::Number(5600.0)));
        assert_eq!(props.get("dpi-stage-6"), Some(&PropValue::Number(26000.0)));
        assert_eq!(
            props.get("dpi-stage-1-colour"),
            Some(&PropValue::Color(Rgb8::new(0xFF, 0, 0)))
        );
        assert_eq!(
            props.get("dpi-stage-6-colour"),
            Some(&PropValue::Color(Rgb8::new(0xFF, 0, 0xFF)))
        );
        assert_eq!(props.get("dpi-active-stage"), Some(&PropValue::Choice("2".into())));
        assert_eq!(props.get("polling"), Some(&PropValue::Choice("1000".into())));
        assert_eq!(props.get("debounce-ms"), Some(&PropValue::Number(8.0)));
        assert_eq!(
            props.get("led-colour"),
            Some(&PropValue::Color(Rgb8::new(0xEF, 0x3F, 0xFF)))
        );
        assert_eq!(props.get("led-mode"), Some(&PropValue::Choice("static".into())));
        assert_eq!(props.get("led-brightness"), Some(&PropValue::Number(8.0)));
        assert_eq!(props.get("led-speed"), Some(&PropValue::Number(6.0)));
        assert_eq!(props.get("sleep-after-min"), Some(&PropValue::Number(0.5)));
        assert_eq!(props.get("deep-sleep-min"), Some(&PropValue::Number(10.0)));
        assert_eq!(props.get("button-1"), Some(&PropValue::Choice("left-click".into())));
        assert_eq!(props.get("button-2"), Some(&PropValue::Choice("right-click".into())));
        assert_eq!(props.get("button-3"), Some(&PropValue::Choice("middle-click".into())));
        assert_eq!(props.get("button-4"), Some(&PropValue::Choice("dpi-cycle".into())));
        assert_eq!(props.get("button-7"), Some(&PropValue::Choice("forward".into())));
        assert_eq!(props.get("button-8"), Some(&PropValue::Choice("back".into())));
        assert_eq!(props.get("profile"), Some(&PropValue::Choice("1".into())));
    }

    #[tokio::test]
    async fn describe_and_read_settings_agree_on_the_keys() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        let schema = a.describe(&a.device_id()).await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        let schema_keys: BTreeSet<&str> = schema.properties().map(|p| p.key.as_str()).collect();
        let read_keys: BTreeSet<&str> = props.keys().map(|k| k.as_str()).collect();
        assert_eq!(schema_keys, read_keys, "every property the mouse has is read back");
        // Nothing is invented: the lift-off distance is not a property of this mouse, and the
        // device never reports it (research 2).
        assert!(!schema.property("lift-off").is_some());
        assert!(!props.contains_key("lift-off"));
    }

    /// The properties each report carries: the report, the live reply, and the property keys it
    /// owns.
    fn report_properties() -> Vec<(Kind, Vec<u8>, Vec<&'static str>)> {
        vec![
            (
                Kind::Dpi,
                codec::evidence::REPLY_04.to_vec(),
                vec![
                    "dpi-stage-count",
                    "dpi-stage-1",
                    "dpi-stage-2",
                    "dpi-stage-3",
                    "dpi-stage-4",
                    "dpi-stage-5",
                    "dpi-stage-6",
                    "dpi-stage-7",
                    "dpi-stage-8",
                    "dpi-stage-1-colour",
                    "dpi-stage-2-colour",
                    "dpi-stage-3-colour",
                    "dpi-stage-4-colour",
                    "dpi-stage-5-colour",
                    "dpi-stage-6-colour",
                    "dpi-stage-7-colour",
                    "dpi-stage-8-colour",
                    "dpi-active-stage",
                ],
            ),
            (
                Kind::Light,
                codec::evidence::REPLY_05.to_vec(),
                vec![
                    "led-mode",
                    "led-colour",
                    "led-brightness",
                    "led-speed",
                    "sleep-after-min",
                    "deep-sleep-min",
                    "debounce-ms",
                ],
            ),
            (Kind::Poll, codec::evidence::REPLY_06.to_vec(), vec!["polling"]),
            (
                Kind::Buttons,
                codec::evidence::REPLY_08.to_vec(),
                vec!["button-1", "button-2", "button-3", "button-4", "button-7", "button-8"],
            ),
            (Kind::Profile, codec::evidence::REPLY_0C.to_vec(), vec!["profile"]),
        ]
    }

    #[test]
    fn every_property_a_report_names_decodes_to_a_value_from_the_live_bytes() {
        // peripheral-hub's test of the same name, in the property model: every property a report
        // names is in the schema, and every one of them has a value from the live bytes.
        let schema = AttackSharkX6::new().schema();
        for (kind, frame, keys) in report_properties() {
            let mut props = Props::new();
            match kind {
                Kind::Dpi => read_dpi(&frame, &mut props),
                Kind::Light => read_light(&frame, &mut props),
                Kind::Poll => read_poll(&frame, &mut props),
                Kind::Buttons => read_buttons(&frame, &mut props),
                Kind::Profile => read_profile(&frame, &mut props),
            }
            for key in &keys {
                assert!(schema.property(key).is_some(), "{kind:?}: the schema has no property {key}");
                assert!(props.contains_key(*key), "{kind:?}: {key} has no value from the live bytes");
            }
            assert_eq!(props.len(), keys.len(), "{kind:?}: the report produced other properties");
        }
    }

    #[tokio::test]
    async fn every_value_read_lies_inside_its_kind() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        let schema = a.describe(&a.device_id()).await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        for def in schema.properties() {
            let value = props.get(&def.key).unwrap_or_else(|| panic!("{} was not read", def.key));
            match (&def.kind, value) {
                (PropKind::Number { min, max, step, .. }, PropValue::Number(n)) => {
                    assert!(*n >= *min && *n <= *max, "{} is {n}, outside {min}..{max}", def.key);
                    assert!(
                        ((*n - *min) / *step).fract() == 0.0,
                        "{} is {n}, not a multiple of {step}",
                        def.key
                    );
                }
                (PropKind::Choice { options, .. }, PropValue::Choice(c)) => {
                    assert!(options.contains(c), "{} has the option {c:?}", def.key);
                }
                (PropKind::Color { .. }, PropValue::Color(_)) => {}
                (PropKind::Bool { .. }, PropValue::Bool(_)) => {}
                (PropKind::Hue { .. }, PropValue::Hue(_)) => {}
                other => panic!("{} has the wrong kind of value: {other:?}", def.key),
            }
        }
    }

    #[tokio::test]
    async fn the_read_sends_the_five_requests_of_the_live_test_in_the_logged_order() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let sent = s.lock().unwrap().sent.clone();
        assert_eq!(sent.len(), 5);
        assert_eq!(
            sent,
            vec![
                vec![0xa0, 0x06, 0x09, 0x00, 0x01, 0x00, 0x00, 0x00],
                vec![0xa0, 0x04, 0x38, 0x00, 0x01, 0x00, 0x00, 0x00],
                vec![0xa0, 0x05, 0x0f, 0x00, 0x01, 0x00, 0x00, 0x00],
                vec![0xa0, 0x08, 0x3b, 0x00, 0x01, 0x00, 0x00, 0x00],
                vec![0xa0, 0x0c, 0x0a, 0x00, 0x01, 0x00, 0x00, 0x00],
            ]
        );
    }

    #[tokio::test]
    async fn a_report_the_mouse_answers_badly_leaves_only_its_own_properties_unread() {
        let (mut a, s) = rig();
        s.lock().unwrap().break_report = Some(0x05);
        a.connect().await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        // The lighting properties are gone; every other report was read.
        assert_eq!(props.get("polling"), Some(&PropValue::Choice("1000".into())));
        assert_eq!(props.get("led-mode"), None);
        assert_eq!(props.get("led-colour"), None);
        assert_eq!(props.get("debounce-ms"), None);
        assert_eq!(props.get("dpi-stage-1"), Some(&PropValue::Number(800.0)));
    }

    #[tokio::test]
    async fn a_mouse_that_answers_nothing_is_an_error() {
        let (mut a, s) = rig();
        s.lock().unwrap().silent = true;
        a.connect().await.unwrap();
        let err = a.read_settings(&a.device_id()).await.unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("none of the five read requests")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_receiver_that_refuses_the_grant_is_retried_and_then_gives_up() {
        let (mut a, s) = rig();
        s.lock().unwrap().grant = 0;
        a.connect().await.unwrap();
        let err = a.read_settings(&a.device_id()).await.unwrap_err();
        // Every report was refused, so the mouse is asleep: that is the message.
        assert!(matches!(&err, AdapterError::Protocol(m) if m == ASLEEP), "{err:?}");
        // Two attempts per report.
        assert_eq!(s.lock().unwrap().sent.len(), 10);
    }

    // -------------------------------------------------------------------------- the sleeping mouse

    #[tokio::test]
    async fn a_mouse_that_refuses_every_grant_is_reported_as_asleep() {
        // The 2026-10-27 read: every one of the five reports came back with the grant refused
        // (0x00), which is what a sleeping mouse does (research 3.3).
        let (mut a, s) = rig();
        s.lock().unwrap().grant = 0;
        a.connect().await.unwrap();
        let err = a.read_settings(&a.device_id()).await.unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m == ASLEEP),
            "{err:?}: the asleep error is its own message"
        );
        // The message is the instruction, and nothing else.
        assert_eq!(ASLEEP, "the mouse is asleep: move it and try again");
    }

    #[tokio::test]
    async fn a_mouse_that_wakes_up_is_read_without_being_asked_again() {
        // The same session, after the mouse was moved: the mouse sends a status event, the
        // adapter sees it and reads once more.
        let (mut a, s) = rig();
        {
            let mut s = s.lock().unwrap();
            s.grant = 0;
            s.grant_when_awake = Some(1);
            s.events.push_back(vec![0x03, 0x10, 0x40, 0x01, 0x0a]);
        }
        a.connect().await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        assert_eq!(props.get("dpi-stage-1"), Some(&PropValue::Number(800.0)));
        assert_eq!(props.get("polling"), Some(&PropValue::Choice("1000".into())));
        // Ten requests for the refused pass and five for the one that worked.
        assert_eq!(s.lock().unwrap().sent.len(), 15);
    }

    #[tokio::test]
    async fn a_mouse_that_stays_asleep_is_not_read_again() {
        let (mut a, s) = rig();
        s.lock().unwrap().grant = 0;
        a.connect().await.unwrap();
        let err = a.read_settings(&a.device_id()).await.unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m == ASLEEP), "{err:?}");
        // Only the first pass went out: no event arrived, so there was nothing to read again.
        assert_eq!(s.lock().unwrap().sent.len(), 10);
    }

    #[tokio::test]
    async fn a_mouse_that_is_awake_is_not_asked_to_wake_up() {
        // The status collection is only watched when every grant was refused.
        let (mut a, s) = rig();
        s.lock().unwrap().events.push_back(vec![0x03, 0x10, 0x40, 0x01, 0x0a]);
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        // The event is still queued: the read did not touch the status collection.
        assert_eq!(s.lock().unwrap().events.len(), 1);
    }

    // --------------------------------------------------------------------------------- events

    #[tokio::test]
    async fn the_battery_event_never_invents_a_percentage() {
        let (mut a, s) = rig();
        s.lock().unwrap().events.push_back(vec![0x03, 0x10, 0x40, 0x01, 0x0a]);
        a.connect().await.unwrap();
        let events = a.read_events(10).await.unwrap();
        assert_eq!(events, vec![PeripheralEvent::Battery { level: None, charging: false }]);
    }

    #[tokio::test]
    async fn a_charging_mouse_and_a_level_byte_above_ten_are_reported() {
        let (mut a, s) = rig();
        {
            let mut s = s.lock().unwrap();
            s.events.push_back(vec![0x03, 0x10, 0x40, 0x03, 0x0a]);
            s.events.push_back(vec![0x03, 0x10, 0x40, 0x01, 0x5a]);
        }
        a.connect().await.unwrap();
        let events = a.read_events(10).await.unwrap();
        assert_eq!(
            events,
            vec![
                PeripheralEvent::Battery { level: None, charging: true },
                PeripheralEvent::Battery { level: None, charging: false },
                PeripheralEvent::Notice { message: "battery-level-byte=90".into() },
            ]
        );
    }

    #[tokio::test]
    async fn the_dpi_button_event_is_announced_and_makes_the_next_dpi_write_wait() {
        let (mut a, s) = rig();
        s.lock().unwrap().events.push_back(vec![0x03, 0x10, 0x10, 0x04, 0x00]);
        a.connect().await.unwrap();
        // The DPI report is read first; the button is pressed afterwards.
        a.read_settings(&a.device_id()).await.unwrap();
        let events = a.read_events(10).await.unwrap();
        assert_eq!(events, vec![PeripheralEvent::Notice { message: "active-dpi-stage=4".into() }]);
        // The next DPI write waits for a fresh read: the frame's active stage is out of date.
        a.set_writes_enabled(true);
        let err = a
            .write_settings(&a.device_id(), &number(&[("dpi-stage-1", 800.0)]))
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("DPI button")), "{err:?}");
        // A fresh read clears it.
        a.read_settings(&a.device_id()).await.unwrap();
        a.write_settings(&a.device_id(), &number(&[("dpi-stage-1", 800.0)])).await.unwrap();
    }

    #[tokio::test]
    async fn reading_events_without_the_status_collection_is_not_connected() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        a.events = None;
        assert!(matches!(a.read_events(10).await, Err(AdapterError::NotConnected)));
    }

    // --------------------------------------------------------------------------------- writes

    #[tokio::test]
    async fn writes_are_off_until_a_caller_turns_them_on() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(&a.device_id(), &choice(&[("polling", "500")]))
            .await
            .unwrap_err();
        assert!(matches!(err, AdapterError::Unsupported(_)), "{err:?}");
        assert_eq!(s.lock().unwrap().sent.len(), before, "nothing reached the mouse");
        assert!(!a.writes_enabled());
    }

    #[tokio::test]
    async fn a_write_sends_the_apply_header_first_and_then_the_changed_report() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        assert!(a.writes_enabled());
        a.write_settings(&a.device_id(), &choice(&[("polling", "500")])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], codec::APPLY_HEADER.to_vec());
        assert_eq!(sent[1], vec![0x06, 0x09, 0x01, 0x02, 0xfd, 0x00, 0x00, 0x00, 0x00]);
    }

    #[tokio::test]
    async fn a_write_round_trips_through_the_fake_transport() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        a.write_settings(&a.device_id(), &choice(&[("polling", "500")])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        // The frame that went out decodes back to the value that was asked for.
        let frame = &sent[1];
        assert_eq!(codec::polling_of(frame), Some("500"));
        // The lighting frame, the same way round.
        let before = s.lock().unwrap().sent.len();
        a.write_settings(
            &a.device_id(),
            &props_of(&[
                ("led-colour", PropValue::Color(Rgb8::new(1, 2, 3))),
                ("led-speed", PropValue::Number(4.0)),
            ]),
        )
        .await
        .unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 2);
        let light = codec::light_of(&sent[1]);
        assert_eq!(light.rgb, (1, 2, 3));
        assert_eq!(light.speed_slider, Some(4));
        // And the DPI frame.
        let before = s.lock().unwrap().sent.len();
        a.write_settings(&a.device_id(), &number(&[("dpi-stage-3", 2000.0)])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        let stages = codec::stages_of(&sent[1]);
        assert_eq!(stages[2].dpi, Some(2000));
        assert_eq!(stages[0].dpi, Some(800), "the other stages keep the device's own values");
    }

    #[tokio::test]
    async fn a_dpi_write_patches_the_frame_the_mouse_reported() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        a.write_settings(&a.device_id(), &number(&[("dpi-stage-3", 2000.0)])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], codec::APPLY_HEADER.to_vec());
        let frame = &sent[1];
        assert_eq!(frame.len(), 56);
        assert_eq!(frame[24], 2, "the active stage the device reported is written back");
        assert_eq!(codec::stages_of(frame)[2].dpi, Some(2000));
        assert_eq!(codec::stages_of(frame)[5].dpi, Some(26000));
        assert_eq!(frame[49], 0x01, "the byte whose meaning is UNKNOWN is copied, not rewritten");
    }

    #[tokio::test]
    async fn the_stage_count_turns_the_stages_off() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        a.write_settings(&a.device_id(), &number(&[("dpi-stage-count", 4.0)])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        let stages = codec::stages_of(&sent[1]);
        assert_eq!(stages[3].dpi, Some(3200), "the first four stages stay on");
        assert_eq!(stages[4].dpi, None, "the fifth stage is off");
        assert_eq!(stages[4].rgb, (0, 0xFF, 0xFF), "an off stage keeps its colour");
    }

    #[tokio::test]
    async fn a_write_without_a_prior_read_is_refused() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(
                &a.device_id(),
                &props_of(&[("led-colour", PropValue::Color(Rgb8::new(1, 2, 3)))]),
            )
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("read the device first")), "{err:?}");
        assert_eq!(s.lock().unwrap().sent.len(), before);
    }

    #[tokio::test]
    async fn an_unknown_key_is_refused_with_a_clear_error() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(&a.device_id(), &number(&[("dpi-stage-99", 800.0)]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("no setting called dpi-stage-99")),
            "{err:?}"
        );
        // The lift-off distance is not a property of this mouse: the firmware never carries it.
        let err = a
            .write_settings(&a.device_id(), &choice(&[("lift-off", "2mm")]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("no setting called lift-off")),
            "{err:?}"
        );
        assert_eq!(s.lock().unwrap().sent.len(), before);
    }

    #[tokio::test]
    async fn a_read_only_key_is_refused_with_a_clear_error() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        for (key, why) in [("button-1", "read-only"), ("dpi-active-stage", "read-only"), ("profile", "read-only")] {
            let err = a
                .write_settings(&a.device_id(), &choice(&[(key, "off")]))
                .await
                .unwrap_err();
            assert!(matches!(&err, AdapterError::Protocol(m) if m.contains(why)), "{key}: {err:?}");
        }
        assert_eq!(s.lock().unwrap().sent.len(), before);
    }

    #[tokio::test]
    async fn a_value_outside_its_range_is_refused() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        for (props, why) in [
            (number(&[("led-brightness", 9.0)]), "takes 1 to 8"),
            (number(&[("led-speed", 3.0)]), "takes 4 to 8"),
            (number(&[("debounce-ms", 5.0)]), "steps of 2"),
            (number(&[("dpi-stage-1", 75.0)]), "steps of 50"),
            (number(&[("sleep-after-min", 31.0)]), "takes 0.5 to 30"),
            (choice(&[("polling", "4000")]), "no option"),
            (choice(&[("led-mode", "rainbow")]), "no option"),
        ] {
            let err = a.write_settings(&a.device_id(), &props).await.unwrap_err();
            assert!(matches!(&err, AdapterError::Protocol(m) if m.contains(why)), "{err:?} (wanted {why})");
        }
        assert_eq!(s.lock().unwrap().sent.len(), before, "no refused change reached the mouse");
    }

    #[tokio::test]
    async fn a_value_of_the_wrong_shape_is_refused() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let err = a
            .write_settings(&a.device_id(), &number(&[("polling", 500.0)]))
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("kind of value")), "{err:?}");
    }

    #[tokio::test]
    async fn the_read_request_goes_out_as_exactly_eight_bytes() {
        // The live read log's `SEND SET_FEATURE len=8` lines: the A0 request is eight bytes and
        // nothing else. hidapi would pad it to the collection's 262-byte feature report length.
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let sent = s.lock().unwrap().sent.clone();
        assert_eq!(sent.len(), 5);
        for request in &sent {
            assert_eq!(request.len(), 8, "the A0 read request is eight bytes: {request:02x?}");
            assert_eq!(request[0], 0xa0);
        }
    }

    #[tokio::test]
    async fn a_config_write_goes_out_as_its_research_length() {
        // Research 3.2: the apply header is ten bytes and report 0x04 is 56.
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        a.write_settings(&a.device_id(), &number(&[("dpi-stage-3", 2000.0)])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].len(), 10, "the apply header is ten bytes (research 3.2)");
        assert_eq!(sent[1].len(), 56, "report 0x04 is 56 bytes (research 3.2)");
        // And the polling report is nine bytes (research 3.2).
        let before = s.lock().unwrap().sent.len();
        a.write_settings(&a.device_id(), &choice(&[("polling", "500")])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent[1].len(), 9, "report 0x06 is nine bytes (research 3.2)");
    }

    #[tokio::test]
    async fn the_active_dpi_stage_is_refused_as_read_only() {
        // The mouse changes the active stage with its own DPI button, and the vendor app's device
        // model lists it as a runtime readout with no write report (research 2).
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(&a.device_id(), &choice(&[("dpi-active-stage", "3")]))
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("read-only")), "{err:?}");
        assert_eq!(s.lock().unwrap().sent.len(), before);
    }

    #[tokio::test]
    async fn a_stage_beyond_the_count_is_refused_instead_of_dropped() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        // The mouse has six stages in use; a seventh would be dropped by the device's mask.
        let err = a
            .write_settings(&a.device_id(), &number(&[("dpi-stage-7", 1600.0)]))
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("beyond the 6 stages")), "{err:?}");
        assert_eq!(s.lock().unwrap().sent.len(), before, "nothing reached the mouse");
        // With the count raised, the same write is accepted.
        a.write_settings(
            &a.device_id(),
            &number(&[("dpi-stage-count", 8.0), ("dpi-stage-7", 1600.0)]),
        )
        .await
        .unwrap();
        assert!(s.lock().unwrap().sent.len() > before);
    }

    #[tokio::test]
    async fn a_brightness_in_a_mode_that_ignores_it_is_refused_instead_of_dropped() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        // The mouse is in the static mode, which takes a brightness: this one is written.
        a.write_settings(&a.device_id(), &number(&[("led-brightness", 5.0)])).await.unwrap();
        assert!(s.lock().unwrap().sent.len() > before);
        // In a mode that ignores the brightness the vendor app sends the constant 8, so the
        // write is refused instead of dropped.
        let err = a
            .write_settings(
                &a.device_id(),
                &props_of(&[
                    ("led-mode", PropValue::Choice("breathing".into())),
                    ("led-brightness", PropValue::Number(5.0)),
                ]),
            )
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("does not take a brightness")), "{err:?}");
        // And the same, after the mouse was put into that mode.
        a.write_settings(&a.device_id(), &choice(&[("led-mode", "breathing")])).await.unwrap();
        let err = a
            .write_settings(&a.device_id(), &number(&[("led-brightness", 5.0)]))
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("does not take a brightness")), "{err:?}");
    }

    #[tokio::test]
    async fn two_writes_to_the_same_report_build_on_each_other() {
        // The device holds what the last write sent, so the second write must not drop the first.
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        a.write_settings(
            &a.device_id(),
            &props_of(&[("led-colour", PropValue::Color(Rgb8::new(1, 1, 1)))]),
        )
        .await
        .unwrap();
        let before = s.lock().unwrap().sent.len();
        a.write_settings(&a.device_id(), &number(&[("led-brightness", 5.0)])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        let light = codec::light_of(&sent[1]);
        assert_eq!(light.rgb, (1, 1, 1), "the colour the first write sent is still there");
        assert_eq!(light.brightness, Some(5));
    }

    #[tokio::test]
    async fn a_write_to_another_device_is_refused() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        a.set_writes_enabled(true);
        let other = DeviceId("attack-shark-x6:1d57:ffff".into());
        assert!(matches!(
            a.write_settings(&other, &choice(&[("polling", "500")])).await,
            Err(AdapterError::UnknownDevice(_))
        ));
        assert!(matches!(a.read_settings(&other).await, Err(AdapterError::UnknownDevice(_))));
        assert!(matches!(a.describe(&other).await, Err(AdapterError::UnknownDevice(_))));
    }

    #[tokio::test]
    async fn a_write_without_the_device_open_is_not_connected() {
        let (mut a, _) = rig();
        a.set_writes_enabled(true);
        assert!(matches!(
            a.write_settings(&a.device_id(), &choice(&[("polling", "500")])).await,
            Err(AdapterError::NotConnected)
        ));
    }
}
