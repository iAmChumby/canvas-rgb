//! The 8BitDo Ultimate 2 Wireless controller, seen through its 2.4G receiver, as a canvas-rgb
//! peripheral.
//!
//! Everything in here was decoded in `peripheral-hub`'s research (the report
//! `peripheral-hub/docs/research/8bitdo-ultimate-2.md`, read in full before this port); this
//! module is the canvas-rgb side of that protocol:
//!
//! - `codec.rs` is the port of the codec: pure bytes in, pure bytes out, no I/O anywhere in it.
//! - This file is the adapter: it finds the pad, says which personality it is showing, reads
//!   the profile image over the vendor channel, decodes it, and can encode a field write of a
//!   setting. **Config writes are off by default**: the write path is behind
//!   [`EightBitDoUltimate2::writes_enabled`], which is **false** until a caller turns it on, and
//!   nothing in this repository turns it on.
//!
//! # The three personalities
//!
//! The pad shows up on USB under one of three product ids, and which one it is decides what can
//! be read at all:
//!
//! | Personality | Product id | Vendor channel | Live input | Battery |
//! |---|---|---|---|---|
//! | XInput (the default) | `2DC8:310B` | yes, usage page `0xFF7A` | through the XInput API | four coarse levels, no percent |
//! | Idle receiver (no controller linked) | `2DC8:6013` | yes, usage page `0xFFA0` | none | none |
//! | DInput | `2DC8:6012` | no (third-party: the receiver's DInput descriptor has no `0x81` output report) | the HID gamepad collection | percent and the charging bit |
//!
//! The research saw only the XInput personality on this PC (2.2c: "No `6012` node has ever
//! enumerated on this PC"), and the DInput attempt of 2026-10-17 failed (2.2e). So the DInput
//! path here is built from the SDL driver the research quotes (7.1), not from a live device.
//!
//! # The value model
//!
//! A setting is a property, not a string. [`PeripheralAdapter::describe`] says what the pad has:
//! sections, properties, and for every property its range, its step or its list of options,
//! taken from the research. [`PeripheralAdapter::read_settings`] gives the current values in a
//! [`Props`] map keyed by [`PropertyDef::key`], and [`PeripheralAdapter::write_settings`] takes
//! the same map back.
//!
//! | Section | Properties |
//! |---|---|
//! | Profiles | `active-profile` (read-only), `profile-1-in-use` … `profile-3-in-use` (read-only) |
//! | Button remap (active profile) | `button-a` … `button-dpad-right` (read-only) |
//! | Profile 1 / 2 / 3 | `profile-N-stick-left-start` … `profile-N-trigger-right-end`, `profile-N-vibration-left`, `profile-N-vibration-right`, `profile-N-paddle-1` … `profile-N-paddle-4` |
//!
//! The pad holds three profiles and every profile has its own records, so the per-profile
//! sections are the real thing: the ranges, the motor strengths and the four extra buttons are
//! stored per profile.
//!
//! # The paddles
//!
//! The four extra buttons are the vendor's map slots 18..21 (`P1`..`P4`): two back paddles and
//! two fast bumpers. **Which physical button sits behind which slot is UNKNOWN** (research U4),
//! so the properties are named after the vendor's slots and not after a physical button.
//!
//! In the XInput personality the pad's XInput driver drops the paddles from every field: a press
//! moves the packet counter and nothing else (research 2.2d, `packet_watch.py` and
//! `xusb_raw.py`). The DInput personality carries the paddles as ordinary bits (research 7.1),
//! and there the adapter reports them from the gamepad report.
//!
//! **The counter on its own is not a paddle.** The 2026-10-27 runs (the pad on the desk) show
//! the counter taking steps of about 30 packets every poll, steadily, *whether or not a paddle is
//! pressed*: with the paddles pressed (the second run) the rhythm is the same as without
//! (the first). The counter is the pad's heartbeat. The paddle experiment therefore measures
//! that heartbeat over an idle window first ([`IDLE_WINDOW`]) and only calls a packet-only move
//! a **candidate** when it breaks the rhythm; the vendor's own capture of a paddle press on this
//! pad showed a step of +100 (research 2.2d), far outside the heartbeat. With nothing pressed,
//! the experiment says so plainly.
//!
//! # The stick and trigger ranges
//!
//! Each profile stores four stick bytes and four trigger bytes. The vendor stores a percentage as
//! `percent * 255 / 100` (research 1), so the adapter shows them as percentages of the travel.
//! What the two ends of a pair mean is not fully settled: for the triggers the research names
//! them the trigger's range (start = where it begins to move, end = full travel); for the sticks
//! the exact dead-zone semantics are UNKNOWN (research 1: "units, inner vs outer"), so they are
//! shown as the stored range and nothing more is claimed.
//!
//! # Config writes
//!
//! **Config writes are off by default.** [`EightBitDoUltimate2::writes_enabled`] is **false**
//! until a caller turns it on, and nothing in this repository turns it on. A *settings read* does
//! send output reports: the 36 chunk requests of the profile read. They ask the pad for what it
//! already holds and change no setting, but they are still 36 output reports, so the first
//! hardware run of this adapter needs an explicit go-ahead.
//!
//! When writes are on they still refuse what the research does not support: the button map of
//! the face buttons is read-only (the research's own writer covers the range records and the map
//! entries, and nothing else), the active profile is the pad's own button (research U5), and the
//! rumble records are read-only for the same reason as the map.
//!
//! The four paddles of the **active** profile can be written: the third-party layout documents
//! the map writer as one short request-1 chunk per entry (four bytes at the entry's offset)
//! followed by the vendor's own commit (request 6, argument `0x0123`), and the image read is
//! confirmed on the pad (2026-10-27). Only the active profile's paddles are written: the pad
//! applies the map of the profile it is on, so a paddle of another profile is refused instead of
//! being written into a record the pad is not using.
//!
//! # What the protocol does not carry
//!
//! - **The profile names.** The vendor's `byte[32]` has no documented encoding, and the spine's
//!   property model has no text kind, so a name is listed as unreported.
//! - **The LED themes.** The ring themes are colour words whose byte order is UNKNOWN.
//! - **The macros, the gyro mapping and the special-feature bits.** Their encodings are not
//!   settled (research U8).
//! - **A firmware version.** The identify reply carries one byte whose scale is UNKNOWN
//!   (research U3): it is reported as a code, never as `major.minor`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canvas_proto::device::DeviceId;
use canvas_proto::props::{PropKind, PropValue, PropertyDef, Props};
use canvas_proto::traits::AdapterError;

pub mod codec;

use self::codec::XState;

pub use canvas_proto::peripheral::{
    PeripheralEvent, PeripheralInfo, PeripheralKind, PeripheralSchema, PeripheralSection,
};
pub use canvas_proto::traits::PeripheralAdapter;

// ------------------------------------------------------------------------------------- identity

/// The vendor id of every 8BitDo device.
pub const VENDOR_ID: u16 = codec::VENDOR_ID;
/// The product id of the 2.4G receiver in its idle personality.
pub const PRODUCT_ID_IDLE: u16 = codec::PID_IDLE;
/// The product id of the pad in the XInput personality.
pub const PRODUCT_ID_XINPUT: u16 = codec::PID_XINPUT;
/// The product id of the pad in the DInput personality.
pub const PRODUCT_ID_DINPUT: u16 = codec::PID_DINPUT;

/// The name the product canvas-rgb shows.
pub const DISPLAY_NAME: &str = "8BitDo Ultimate 2 Wireless";
/// The manufacturer the device reports.
pub const MANUFACTURER: &str = "8BitDo";
/// The adapter name.
pub const ADAPTER_NAME: &str = "eightbitdo-ultimate-2";

/// The usage page of the vendor collection in the XInput personality.
pub const VENDOR_USAGE_PAGE_XINPUT: u16 = 0xFF7A;
/// The usage page of the vendor collection in the idle personality.
pub const VENDOR_USAGE_PAGE_IDLE: u16 = 0xFFA0;
/// The usage of the vendor collection.
pub const VENDOR_USAGE: u16 = 0x0001;
/// The usage page of the gamepad collection.
pub const GAMEPAD_USAGE_PAGE: u16 = 0x0001;
/// The usage of the gamepad collection.
pub const GAMEPAD_USAGE: u16 = 0x0005;

/// Which personality the pad (or its receiver) is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Personality {
    /// The 2.4G receiver with no controller linked (`2DC8:6013`).
    Idle,
    /// The pad in the XInput personality (`2DC8:310B`): the default, and the one this PC shows.
    XInput,
    /// The pad in the DInput personality (`2DC8:6012`).
    DInput,
}

impl Personality {
    /// The product id this personality enumerates under.
    pub const fn product_id(self) -> u16 {
        match self {
            Personality::Idle => codec::PID_IDLE,
            Personality::XInput => codec::PID_XINPUT,
            Personality::DInput => codec::PID_DINPUT,
        }
    }

    /// What the adapter calls this personality.
    pub const fn label(self) -> &'static str {
        match self {
            Personality::Idle => "2.4G receiver, idle",
            Personality::XInput => "XInput",
            Personality::DInput => "DInput",
        }
    }

    /// The usage page of the vendor collection, when this personality has one.
    pub const fn vendor_usage_page(self) -> Option<u16> {
        match self {
            Personality::Idle => Some(VENDOR_USAGE_PAGE_IDLE),
            Personality::XInput => Some(VENDOR_USAGE_PAGE_XINPUT),
            // The receiver's DInput descriptor has no `0x81` output report (research 5.1).
            Personality::DInput => None,
        }
    }
}

// ------------------------------------------------------------------------------------ transport

/// Reads the input reports of a HID collection.
pub trait InputTransport: Send + Sync {
    /// Reads one input report into `buf`. `Ok(0)` means nothing arrived within `timeout_ms`.
    fn read_input(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String>;
}

/// The vendor channel: output reports in, input reports out.
pub trait VendorTransport: InputTransport {
    /// Sends one output report (byte 0 is the report id).
    fn send_report(&mut self, data: &[u8]) -> Result<(), String>;
}

/// The collections the paddle experiment opens: a label and the handle.
pub type OpenedCollections = Vec<(String, Box<dyn InputTransport>)>;

/// Finds and opens the pad's collections.
pub trait Opener: Send + Sync {
    /// Every personality the device is showing, the pad's first.
    fn personalities(&self) -> Vec<Personality>;
    /// Opens the vendor collection of `personality`. `Ok(None)` when it has none.
    fn open_vendor(&self, personality: Personality) -> Result<Option<Box<dyn VendorTransport>>, String>;
    /// Opens the gamepad collection of `personality`. `Ok(None)` when it has none.
    fn open_gamepad(
        &self,
        personality: Personality,
    ) -> Result<Option<Box<dyn InputTransport>>, String>;
    /// Opens every HID collection of the device, for the read-only paddle experiment.
    fn open_collections(&self) -> Result<OpenedCollections, String>;
}

/// `HidDevice` is `Send` but not `Sync`; the mutex supplies `Sync` (it is never contended).
struct HidCollection(std::sync::Mutex<hidapi::HidDevice>);

impl InputTransport for HidCollection {
    fn read_input(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        dev.read_timeout(buf, timeout_ms).map_err(|e| e.to_string())
    }
}

/// The vendor collection: the same handle, with the output report on top.
struct HidVendor(std::sync::Mutex<hidapi::HidDevice>);

impl InputTransport for HidVendor {
    fn read_input(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        dev.read_timeout(buf, timeout_ms).map_err(|e| e.to_string())
    }
}

impl VendorTransport for HidVendor {
    fn send_report(&mut self, data: &[u8]) -> Result<(), String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        // The report goes out over the interrupt OUT endpoint. hidapi pads it to the collection's
        // `OutputReportByteLength`, which is the 64 bytes this channel uses, so the length it is
        // given is the length that goes out.
        dev.write(data).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// What the opener needs from a HID context: `hidapi::HidApi` in the product, a counting fake in
/// the tests.
trait HidSource: crate::attack_shark_x6::hid_cache::Scan {
    /// The path of the first collection with this product id, usage page and usage.
    fn find_path(&self, product_id: u16, usage_page: u16, usage: u16) -> Option<std::ffi::CString>;
    /// True when any collection has this product id.
    fn has_product(&self, product_id: u16) -> bool;
    /// Opens a vendor collection.
    fn open_vendor_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn VendorTransport>, String>;
    /// Opens a gamepad collection.
    fn open_gamepad_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn InputTransport>, String>;
    /// Opens every collection of the vendor that will open.
    fn open_all(&self) -> OpenedCollections;
}

impl HidSource for hidapi::HidApi {
    fn find_path(&self, product_id: u16, usage_page: u16, usage: u16) -> Option<std::ffi::CString> {
        self.device_list()
            .find(|d| {
                d.vendor_id() == VENDOR_ID
                    && d.product_id() == product_id
                    && d.usage_page() == usage_page
                    && d.usage() == usage
            })
            .map(|d| d.path().to_owned())
    }

    fn has_product(&self, product_id: u16) -> bool {
        self.device_list().any(|d| d.product_id() == product_id)
    }

    fn open_vendor_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn VendorTransport>, String> {
        let dev = self.open_path(path).map_err(|e| e.to_string())?;
        Ok(Box::new(HidVendor(std::sync::Mutex::new(dev))))
    }

    fn open_gamepad_at(&self, path: &std::ffi::CStr) -> Result<Box<dyn InputTransport>, String> {
        let dev = self.open_path(path).map_err(|e| e.to_string())?;
        Ok(Box::new(HidCollection(std::sync::Mutex::new(dev))))
    }

    fn open_all(&self) -> OpenedCollections {
        let mut out: OpenedCollections = Vec::new();
        for d in self.device_list().filter(|d| d.vendor_id() == VENDOR_ID) {
            let label = format!(
                "pid={:04x} if={} up={:04x} u={:04x}",
                d.product_id(),
                d.interface_number(),
                d.usage_page(),
                d.usage()
            );
            match self.open_path(d.path()) {
                Ok(dev) => out.push((label, Box::new(HidCollection(std::sync::Mutex::new(dev))))),
                // Windows holds the keyboard and mouse collections exclusively: the research saw
                // the same (2.2d: "Keyboard/mouse collections unreadable (Windows exclusive)").
                Err(e) => tracing::warn!("the collection {label} cannot be opened: {e}"),
            }
        }
        out
    }
}

/// Finds the pad's collections with hidapi.
///
/// One HID context serves every call (`crate::attack_shark_x6::hid_cache`): each step of a
/// connect used to build its own `HidApi`, which scans every HID device on the machine
/// (issue #7). Every method looks up through `with_retry`: a miss or a failed open on an old list
/// rescans once per ttl, so a replugged pad is found.
struct HidOpener<S: HidSource = hidapi::HidApi> {
    cache: crate::attack_shark_x6::hid_cache::HidCache<S>,
}

impl<S: HidSource> Default for HidOpener<S> {
    fn default() -> Self {
        Self { cache: Default::default() }
    }
}

impl<S: HidSource> Opener for HidOpener<S> {
    fn personalities(&self) -> Vec<Personality> {
        let found = self.cache.with_retry(|api| {
            let mut out = Vec::new();
            // The XInput personality: the vendor collection on the vendor page.
            if api.find_path(PRODUCT_ID_XINPUT, VENDOR_USAGE_PAGE_XINPUT, VENDOR_USAGE).is_some() {
                out.push(Personality::XInput);
            }
            // The DInput personality: the pad's own product id.
            if api.has_product(PRODUCT_ID_DINPUT) {
                out.push(Personality::DInput);
            }
            // The idle receiver: the vendor collection on the other vendor page.
            if api.find_path(PRODUCT_ID_IDLE, VENDOR_USAGE_PAGE_IDLE, VENDOR_USAGE).is_some() {
                out.push(Personality::Idle);
            }
            // No personality is a miss: it rescans once (per ttl) in case the pad just arrived.
            Ok((!out.is_empty()).then_some(out))
        });
        found.ok().flatten().unwrap_or_default()
    }

    fn open_vendor(
        &self,
        personality: Personality,
    ) -> Result<Option<Box<dyn VendorTransport>>, String> {
        // The DInput personality has no vendor collection of its own; the wired pad in DInput
        // mode may still have one on the vendor page.
        let pages: &[u16] = match personality.vendor_usage_page() {
            Some(page) => &[page, VENDOR_USAGE_PAGE_XINPUT][..],
            None => &[VENDOR_USAGE_PAGE_XINPUT][..],
        };
        self.cache.with_retry(|api| {
            for page in pages {
                if let Some(path) = api.find_path(personality.product_id(), *page, VENDOR_USAGE) {
                    return api.open_vendor_at(&path).map(Some);
                }
            }
            Ok(None)
        })
    }

    fn open_gamepad(
        &self,
        personality: Personality,
    ) -> Result<Option<Box<dyn InputTransport>>, String> {
        // Only the DInput personality has a gamepad collection to read: in the XInput
        // personality the pad is an Xbox 360 class device and its input comes through the XInput
        // API, and the HID view Windows shows for it (IG_00) carries the Xbox report, not the
        // DInput one.
        if personality != Personality::DInput {
            return Ok(None);
        }
        self.cache.with_retry(|api| {
            match api.find_path(personality.product_id(), GAMEPAD_USAGE_PAGE, GAMEPAD_USAGE) {
                None => Ok(None),
                Some(path) => api.open_gamepad_at(&path).map(Some),
            }
        })
    }

    fn open_collections(&self) -> Result<OpenedCollections, String> {
        // Nothing opened is a miss (stale paths all failed, or the pad just arrived): rescan once
        // per ttl and open again.
        let opened = self.cache.with_retry(|api| {
            let all = api.open_all();
            Ok((!all.is_empty()).then_some(all))
        })?;
        Ok(opened.unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------------- the XInput API

/// The XInput API, loaded at run time: the crate needs no import library for it, and a machine
/// without XInput simply has no pad to read.
#[cfg(windows)]
mod xinput {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    use super::codec::XState;

    /// `XINPUT_GAMEPAD`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Gamepad {
        pub buttons: u16,
        pub left_trigger: u8,
        pub right_trigger: u8,
        pub thumb_lx: i16,
        pub thumb_ly: i16,
        pub thumb_rx: i16,
        pub thumb_ry: i16,
    }

    /// `XINPUT_STATE`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct State {
        pub packet: u32,
        pub gamepad: Gamepad,
    }

    /// `XINPUT_BATTERY_INFORMATION`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct BatteryInformation {
        pub battery_type: u8,
        pub battery_state: u8,
    }

    type GetState = unsafe extern "system" fn(u32, *mut State) -> u32;
    type GetBattery = unsafe extern "system" fn(u32, u8, *mut BatteryInformation) -> u32;

    /// The loaded API.
    pub struct Api {
        #[allow(dead_code)]
        module: HMODULE,
        get_state: GetState,
        get_battery: Option<GetBattery>,
    }

    // The library handle and the function pointers are only read, never moved between threads
    // while a call is in flight.
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    fn load_library(name: &str) -> Option<HMODULE> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        (!handle.is_null()).then_some(handle)
    }

    impl Api {
        /// The API, loaded once. `None` when no XInput library is on the machine.
        pub fn load() -> Option<&'static Api> {
            static API: OnceLock<Option<Api>> = OnceLock::new();
            API.get_or_init(|| {
                // The order the research's own poll script uses.
                for name in ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll", "xinput1_1.dll"]
                {
                    let Some(module) = load_library(name) else { continue };
                    let Some(state) =
                        (unsafe { GetProcAddress(module, c"XInputGetState".as_ptr().cast()) })
                    else {
                        continue;
                    };
                    let get_state: GetState = unsafe { std::mem::transmute(state) };
                    let battery =
                        unsafe { GetProcAddress(module, c"XInputGetBatteryInformation".as_ptr().cast()) };
                    let get_battery = if battery.is_some() {
                        Some(unsafe { std::mem::transmute::<
                            windows_sys::Win32::Foundation::FARPROC,
                            GetBattery,
                        >(battery) })
                    } else {
                        None
                    };
                    return Some(Api { module, get_state, get_battery });
                }
                None
            })
            .as_ref()
        }

        /// The state of one slot, when a pad is in it.
        pub fn state(&self, slot: u32) -> Option<XState> {
            let mut state = State::default();
            let rc = unsafe { (self.get_state)(slot, &mut state) };
            if rc != 0 {
                return None;
            }
            Some(XState {
                packet: state.packet,
                buttons: state.gamepad.buttons,
                lt: state.gamepad.left_trigger,
                rt: state.gamepad.right_trigger,
                lx: state.gamepad.thumb_lx,
                ly: state.gamepad.thumb_ly,
                rx: state.gamepad.thumb_rx,
                ry: state.gamepad.thumb_ry,
            })
        }

        /// The battery of one slot: the type and the state, when the API reports one.
        pub fn battery(&self, slot: u32) -> Option<(u8, u8)> {
            let get = self.get_battery?;
            let mut info = BatteryInformation::default();
            let rc = unsafe { get(slot, 0, &mut info) };
            if rc != 0 {
                return None;
            }
            // 0 is unknown and 1 is disconnected: neither is a battery.
            if info.battery_type == 0 || info.battery_type == 1 {
                return None;
            }
            Some((info.battery_type, info.battery_state))
        }
    }
}

/// Everywhere else there is no XInput API.
#[cfg(not(windows))]
mod xinput {
    use super::codec::XState;

    /// A stand-in for the API that is never there.
    pub struct Api;

    impl Api {
        /// No XInput outside Windows.
        pub fn load() -> Option<&'static Api> {
            None
        }

        /// No pad outside Windows.
        pub fn state(&self, _slot: u32) -> Option<XState> {
            None
        }

        /// No battery outside Windows.
        pub fn battery(&self, _slot: u32) -> Option<(u8, u8)> {
            None
        }
    }
}

/// The XInput API, as the adapter uses it.
///
/// The real implementation talks to Windows; the tests put one in its place that answers
/// nothing, so a test never depends on what is plugged into the machine.
pub trait XInputSource: Send + Sync {
    /// The state of one slot, when a pad is in it.
    fn state(&self, slot: u32) -> Option<XState>;
    /// The battery of one slot: the type and the state.
    fn battery(&self, slot: u32) -> Option<(u8, u8)>;
}

/// The real XInput API.
struct SystemXInput;

impl XInputSource for SystemXInput {
    fn state(&self, slot: u32) -> Option<XState> {
        xinput::Api::load().and_then(|api| api.state(slot))
    }

    fn battery(&self, slot: u32) -> Option<(u8, u8)> {
        xinput::Api::load().and_then(|api| api.battery(slot))
    }
}

// ---------------------------------------------------------------------------------- property keys

const KEY_ACTIVE_PROFILE: &str = "active-profile";
const KEY_BUTTON_PREFIX: &str = "button-";
const KEY_PADDLE_PREFIX: &str = "paddle-";

/// The property key of a profile's "in use" mark.
fn in_use_key(profile: usize) -> String {
    format!("profile-{}-in-use", profile + 1)
}

/// The property key of a profile's paddle slot.
fn paddle_key(profile: usize, slot: usize) -> String {
    format!("profile-{}-paddle-{}", profile + 1, slot + 1)
}

/// The property key of a button-map slot.
fn button_key(slot: usize) -> String {
    format!("{KEY_BUTTON_PREFIX}{}", codec::MAP_SLOT_NAMES[slot])
}

/// The labels of the range rows, in the order of [`codec::RANGE_ROWS`].
const RANGE_LABELS: [&str; 8] = [
    "Left stick range start (%)",
    "Left stick range end (%)",
    "Right stick range start (%)",
    "Right stick range end (%)",
    "Left trigger range start (%)",
    "Left trigger range end (%)",
    "Right trigger range start (%)",
    "Right trigger range end (%)",
];

/// The property keys this adapter cannot write yet.
fn read_only() -> Vec<String> {
    let mut out = vec![KEY_ACTIVE_PROFILE.to_string()];
    for profile in 0..codec::PROFILES {
        out.push(in_use_key(profile));
        out.push(format!("profile-{}-vibration-left", profile + 1));
        out.push(format!("profile-{}-vibration-right", profile + 1));
    }
    for slot in 0..codec::EXTRA_BUTTON_FIRST {
        out.push(button_key(slot));
    }
    out
}

/// The profile and the map slot a paddle key stands for: `profile-2-paddle-3` is profile 2's
/// third paddle, which is the vendor's map slot 20.
fn paddle_slot(key: &str) -> Option<(usize, usize)> {
    let rest = key.strip_prefix("profile-")?;
    let (profile, suffix) = rest.split_once('-')?;
    let profile: usize = profile.parse().ok()?;
    let slot = suffix.strip_prefix(KEY_PADDLE_PREFIX)?.parse::<usize>().ok()?;
    if !(1..=codec::PROFILES).contains(&profile) || !(1..=4).contains(&slot) {
        return None;
    }
    Some((profile - 1, codec::EXTRA_BUTTON_FIRST + slot - 1))
}

/// The settings the pad holds that this adapter does not report as values.
fn unreported() -> Vec<String> {
    let mut out = Vec::new();
    for profile in 0..codec::PROFILES {
        let n = profile + 1;
        // The name is text, and the property model has no text kind.
        out.push(format!("profile-{n}-name"));
        // The ring themes are colour words whose byte order is UNKNOWN (research 5.4).
        out.push(format!("profile-{n}-ring-theme"));
        // The macro records, the gyro mapping and the special-feature bits: their encodings are
        // not settled (research U8).
        out.push(format!("profile-{n}-macros"));
        out.push(format!("profile-{n}-gyro-mapping"));
        out.push(format!("profile-{n}-special-features"));
        // The XInput rumble range: the motor strengths are the ones the vendor shows.
        out.push(format!("profile-{n}-xinput-rumble-range"));
    }
    // The linear/tactile trigger switch is on the pad, not in the image.
    out.push("trigger-mode".into());
    // The identify reply carries one byte whose scale is UNKNOWN (research U3).
    out.push("firmware-version".into());
    // The XInput API has no percent and no charging flag.
    out.push("battery-percent".into());
    out
}

/// Why a property cannot be written yet.
fn read_only_reason(key: &str) -> &'static str {
    if key == KEY_ACTIVE_PROFILE {
        "the pad changes the active profile with its own button, and no write for it is described \
         (research 5.4)"
    } else if key.starts_with(KEY_BUTTON_PREFIX) {
        "the research's own writer covers the stick and trigger range records only, and a map \
         write would need an image read that has never run on the pad (research 2.2c and 8)"
    } else if key.contains("vibration") {
        "the research's own writer covers the stick and trigger range records only (research 5.3)"
    } else if key.contains("in-use") {
        "this is the vendor app's mark of a record in use, not a setting"
    } else {
        "this adapter cannot write it yet"
    }
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

fn refuse(message: impl Into<String>) -> AdapterError {
    AdapterError::Protocol(message.into())
}

/// A HID I/O failure (an unplugged pad), as opposed to a pad that is merely silent.
fn io_failure(message: impl Into<String>) -> AdapterError {
    AdapterError::Io(std::io::Error::other(message.into()))
}

fn join_error(e: tokio::task::JoinError) -> AdapterError {
    refuse(format!("the pad blocking task failed: {e}"))
}

// ------------------------------------------------------------------------------------- the timing

/// The quiet times the pad needs between its calls.
#[derive(Clone, Copy, Debug)]
struct Pace {
    /// How long one poll waits for a reply.
    poll_ms: i32,
    /// How long to wait between two chunks of the profile read.
    chunk_pause: Duration,
    /// How long to wait between a field write and the commit (research 5.3: about 100 ms).
    write_pause: Duration,
    /// How long the vendor's write wrapper waits after `Reportstate(0)` and before
    /// `Reportstate(1)` (30 ms, `Ultimate2_4Advance2.excerpt.cs`, the static write wrappers).
    session_pause: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            poll_ms: 100,
            chunk_pause: Duration::from_millis(30),
            write_pause: Duration::from_millis(100),
            session_pause: Duration::from_millis(30),
        }
    }
}

/// How many times a chunk is asked for before the read gives up.
const ATTEMPTS: u32 = 3;
/// How many times a reply is polled for one attempt.
const POLLS: u32 = 4;

/// How long the paddle experiment measures the pad's idle heartbeat before it starts counting
/// candidates.
///
/// Three seconds: the pad's own events come about every two seconds (research 3.4 for the mouse,
/// the same dongle rhythm here), so three seconds covers one and a half of them.
pub const IDLE_WINDOW: Duration = Duration::from_secs(3);

// ------------------------------------------------------------------------------- the profile read

/// Reads the whole profile image: 36 chunks of 45 bytes.
///
/// Blocking: call it from `spawn_blocking`. A chunk that is not answered ends the read with the
/// reason the pad gave.
fn read_image(vendor: &mut dyn VendorTransport, pace: Pace) -> Result<Vec<u8>, AdapterError> {
    let mut img = Vec::with_capacity(codec::IMAGE_LEN);
    while img.len() < codec::IMAGE_LEN {
        let offset = img.len();
        let len = codec::CHUNK.min(codec::IMAGE_LEN - offset);
        let request = codec::read_request(offset).map_err(refuse)?;
        let mut last = String::from("no reply");
        let mut chunk = None;
        for attempt in 1..=ATTEMPTS {
            vendor
                .send_report(&request)
                .map_err(|e| io_failure(format!("the read request for chunk {offset:#x} failed: {e}")))?;
            for _ in 0..POLLS {
                let mut buf = [0u8; codec::REPORT_LEN];
                let n = vendor
                    .read_input(&mut buf, pace.poll_ms)
                    .map_err(|e| io_failure(format!("the reply to chunk {offset:#x} failed: {e}")))?;
                if n == 0 {
                    continue;
                }
                match codec::parse_read_reply(&buf[..n], len, offset) {
                    Ok(data) => {
                        chunk = Some(data.to_vec());
                        break;
                    }
                    Err(why) => last = why,
                }
            }
            if chunk.is_some() {
                break;
            }
            tracing::warn!(offset = offset, attempt, "{}", last);
        }
        let Some(data) = chunk else {
            return Err(refuse(format!(
                "the pad did not answer the read request for chunk {offset:#x} of the profile \
                 image: {last}"
            )));
        };
        img.extend_from_slice(&data);
        if img.len() < codec::IMAGE_LEN {
            std::thread::sleep(pace.chunk_pause);
        }
    }
    Ok(img)
}

// ------------------------------------------------------------------------------ the write session

/// Waits for the pad's acknowledgement of request word `want` and checks it.
///
/// Blocking. A reply that answers another request word, or no reply at all, is an error: the
/// caller stops there.
fn await_ack(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    want: u16,
    what: &str,
) -> Result<(), AdapterError> {
    for _ in 0..POLLS {
        let mut buf = [0u8; codec::REPORT_LEN];
        let n = vendor
            .read_input(&mut buf, pace.poll_ms)
            .map_err(|e| refuse(format!("the reply to the {what} failed: {e}")))?;
        if n == 0 {
            continue;
        }
        return codec::parse_ack_reply(&buf[..n], want).map_err(|why| {
            refuse(format!(
                "the pad's reply to the {what} was wrong: {why}; reply: {}",
                codec::hex(&buf[..n])
            ))
        });
    }
    Err(refuse(format!("the pad did not reply to the {what}")))
}

/// Throws away anything the pad has already queued (a late answer to `Reportstate`), so it
/// cannot be mistaken for the reply to the write.
fn drain_input(vendor: &mut dyn VendorTransport, pace: Pace) {
    for _ in 0..8 {
        let mut buf = [0u8; codec::REPORT_LEN];
        match vendor.read_input(&mut buf, pace.poll_ms) {
            Ok(n) if n > 0 => {
                tracing::info!(raw = %codec::hex(&buf[..n]), "a queued pad report is dropped before the write");
            }
            _ => break,
        }
    }
}

/// One config write, bracketed the way the vendor's write wrappers bracket it
/// (`Ultimate2_4Advance2.excerpt.cs`, the static write wrappers): `Reportstate(0)`, 30 ms, the
/// write chunk(s), about 100 ms (research 5.3), the commit, 30 ms, `Reportstate(1)`.
///
/// All packets use the no-length framing (research 5.2). The pad's reply is read after every
/// write chunk (request word 1) and after the commit (request word 6); the commit is never sent
/// unless every write chunk got a good reply. `Reportstate(1)` is always sent last, even after a
/// failure. `packets` is the plan: the write chunks, then the commit.
fn write_session(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    packets: &[Vec<u8>],
) -> Result<(), AdapterError> {
    let Some((commit, writes)) = packets.split_last() else {
        return Ok(());
    };
    let outcome = (|| -> Result<(), AdapterError> {
        vendor
            .send_report(&codec::report_state_request(0))
            .map_err(|e| refuse(format!("Reportstate(0) failed: {e}")))?;
        std::thread::sleep(pace.session_pause);
        drain_input(vendor, pace);
        for (i, packet) in writes.iter().enumerate() {
            tracing::info!(raw = %codec::hex(packet), "the pad write goes out");
            vendor
                .send_report(packet)
                .map_err(|e| refuse(format!("the pad write failed: {e}")))?;
            await_ack(vendor, pace, codec::REQ_WRITE, &format!("write chunk {}", i + 1))?;
        }
        std::thread::sleep(pace.write_pause);
        tracing::info!(raw = %codec::hex(commit), "the pad commit goes out");
        vendor
            .send_report(commit)
            .map_err(|e| refuse(format!("the commit failed: {e}")))?;
        await_ack(vendor, pace, codec::REQ_COMMIT, "commit").map_err(|e| {
            refuse(format!(
                "{e}; the write chunk(s) went out and the pad may hold them uncommitted"
            ))
        })
    })();
    std::thread::sleep(pace.session_pause);
    let closing = vendor
        .send_report(&codec::report_state_request(1))
        .map_err(|e| refuse(format!("Reportstate(1) failed: {e}")));
    match (outcome, closing) {
        (Err(e), Err(c)) => Err(refuse(format!("{e}; also {c}"))),
        (Err(e), Ok(())) => Err(e),
        (Ok(()), Err(c)) => Err(c),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// The image as properties.
///
/// The pad holds three profiles and every profile has its own records, so the image is decoded
/// for all three. The button map is the active profile's: the research's writer covers the range
/// records only, so the map of the other profiles is not shown.
fn props_from_image(img: &[u8]) -> Result<Props, String> {
    let active = codec::active_profile(img).unwrap_or(0);
    let mut props = Props::new();
    props.insert(KEY_ACTIVE_PROFILE.into(), PropValue::Choice((active + 1).to_string()));
    for profile in 0..codec::PROFILES {
        let decoded = codec::decode_profile(img, profile)?;
        props.insert(in_use_key(profile), PropValue::Bool(decoded.in_use));
        // The ranges: the stored byte as a percentage of the travel.
        for (row, (_, _, _)) in codec::RANGE_ROWS.iter().enumerate() {
            let bytes = if row < 4 { decoded.stick } else { decoded.trigger };
            let percent = codec::byte_to_percent(bytes[row % 4]);
            props.insert(
                codec::range_key(profile, row_suffix(row)),
                PropValue::Number(percent),
            );
        }
        // The motor strengths. The vendor stores them as 0.0..1.0 in 0.2 steps, so they are shown
        // as the percent that is: 0, 20, 40, 60, 80 or 100. A value outside that is not a
        // strength the editor could show, so it is held inside the range.
        for (side, strength) in ["left", "right"].into_iter().zip(decoded.vibration) {
            let percent = if strength.is_finite() {
                ((f64::from(strength) * 100.0) / 20.0).round() * 20.0
            } else {
                0.0
            };
            props.insert(
                format!("profile-{}-vibration-{side}", profile + 1),
                PropValue::Number(percent.clamp(0.0, 100.0)),
            );
        }
        // The four extra buttons: the target each one produces.
        for slot in 0..4 {
            let mask = decoded.map[codec::EXTRA_BUTTON_FIRST + slot];
            props.insert(
                paddle_key(profile, slot),
                PropValue::Choice(codec::button_target(mask).to_string()),
            );
        }
    }
    // The button map of the active profile.
    let decoded = codec::decode_profile(img, active)?;
    for slot in 0..codec::MAP_SLOTS {
        if slot >= codec::EXTRA_BUTTON_FIRST {
            // The four extra buttons are in the per-profile sections.
            continue;
        }
        props.insert(
            button_key(slot),
            PropValue::Choice(codec::button_target(decoded.map[slot]).to_string()),
        );
    }
    Ok(props)
}

/// The suffix of the n-th range row.
fn row_suffix(row: usize) -> &'static str {
    codec::RANGE_ROWS[row].0
}

// -------------------------------------------------------------------------------- the adapter

/// The 8BitDo Ultimate 2 Wireless controller.
pub struct EightBitDoUltimate2 {
    opener: Arc<dyn Opener>,
    xinput: Arc<dyn XInputSource>,
    personality: Option<Personality>,
    /// The personality the open handles were opened for.
    bound: Option<Personality>,
    vendor: Option<Box<dyn VendorTransport>>,
    gamepad: Option<Box<dyn InputTransport>>,
    /// False until a caller turns config writes on. Nothing is written to the hardware while it
    /// is false.
    writes_enabled: bool,
    pace: Pace,
    /// The image as last read.
    image: Option<Vec<u8>>,
    /// The last XInput state, so the extra-button signature can be seen.
    last_xinput: Option<XState>,
}

impl Default for EightBitDoUltimate2 {
    fn default() -> Self {
        Self::new()
    }
}

impl EightBitDoUltimate2 {
    /// The adapter for the 8BitDo Ultimate 2 Wireless.
    pub fn new() -> Self {
        Self {
            opener: Arc::new(HidOpener::<hidapi::HidApi>::default()),
            xinput: Arc::new(SystemXInput),
            personality: None,
            bound: None,
            vendor: None,
            gamepad: None,
            writes_enabled: false,
            pace: Pace::default(),
            image: None,
            last_xinput: None,
        }
    }

    /// Turns config writes on or off. They are off until a caller says otherwise.
    pub fn set_writes_enabled(&mut self, on: bool) {
        self.writes_enabled = on;
    }

    /// The stable id of this pad.
    pub fn device_id(&self) -> DeviceId {
        DeviceId(format!("{ADAPTER_NAME}:{VENDOR_ID:04x}"))
    }

    /// The personality the pad is showing, when the adapter has found it.
    pub fn personality(&self) -> Option<Personality> {
        self.personality
    }

    /// The profile image as the last read brought it back, all 1592 bytes.
    ///
    /// The probe writes this out so the bytes can be looked at off the device.
    pub fn image(&self) -> Option<&[u8]> {
        self.image.as_deref()
    }

    /// The packets a batch of changes would put on the wire, without sending them.
    ///
    /// The probe prints these before it writes, so the exact bytes that go out are on the record
    /// before they go out.
    ///
    /// The result is the whole session in order: `Reportstate(0)`, the write chunk(s), the
    /// commit, `Reportstate(1)`.
    pub fn plan_packets(&self, props: &Props) -> Result<Vec<Vec<u8>>, AdapterError> {
        let mut packets = vec![codec::report_state_request(0)];
        packets.extend(self.plan(props)?);
        packets.push(codec::report_state_request(1));
        Ok(packets)
    }

    fn check_id(&self, id: &DeviceId) -> Result<(), AdapterError> {
        if *id == self.device_id() {
            Ok(())
        } else {
            Err(AdapterError::UnknownDevice(id.clone()))
        }
    }

    fn info(&self) -> PeripheralInfo {
        let personality = self.personality.unwrap_or(Personality::XInput);
        PeripheralInfo {
            id: self.device_id(),
            name: format!("{DISPLAY_NAME} ({})", personality.label()),
            vendor: MANUFACTURER.into(),
            kind: PeripheralKind::GameController,
            adapter: ADAPTER_NAME.into(),
            serial: String::new(),
            location: format!("hid:{VENDOR_ID:04x}:{:04x}", personality.product_id()),
            connected: self.vendor.is_some() || self.gamepad.is_some(),
        }
    }

    /// What the pad has: its sections, its properties and their ranges.
    pub fn schema(&self) -> PeripheralSchema {
        let mut sections = Vec::new();

        // Profiles: which one is active, and which ones the pad holds.
        let mut profiles = vec![PropertyDef {
            key: KEY_ACTIVE_PROFILE.into(),
            label: "Active profile".into(),
            kind: PropKind::Choice {
                options: (1..=codec::PROFILES).map(|n| n.to_string()).collect(),
                default: "1".into(),
            },
        }];
        for profile in 0..codec::PROFILES {
            profiles.push(PropertyDef {
                key: in_use_key(profile),
                label: format!("Profile {} in use", profile + 1),
                kind: PropKind::Bool { default: false },
            });
        }
        sections.push(PeripheralSection { id: "profiles".into(), title: "Profiles".into(), properties: profiles });

        // The button map of the active profile. The paddles are in the profile sections.
        let mut buttons = Vec::new();
        for slot in 0..codec::EXTRA_BUTTON_FIRST {
            buttons.push(PropertyDef {
                key: button_key(slot),
                label: codec::MAP_SLOT_LABELS[slot].into(),
                kind: PropKind::Choice { options: codec::target_options(), default: "none".into() },
            });
        }
        sections.push(PeripheralSection {
            id: "buttons".into(),
            title: "Button remap (active profile)".into(),
            properties: buttons,
        });

        // One section per profile: its ranges, its motors and its four extra buttons.
        for profile in 0..codec::PROFILES {
            let n = profile + 1;
            let mut properties = Vec::new();
            for (row, (suffix, _, _)) in codec::RANGE_ROWS.iter().enumerate() {
                properties.push(PropertyDef {
                    key: codec::range_key(profile, suffix),
                    label: format!("{} (profile {n})", RANGE_LABELS[row]),
                    kind: PropKind::Number {
                        min: 0.0,
                        max: 100.0,
                        step: 1.0,
                        default: 0.0,
                    },
                });
            }
            for (side, label) in [("left", "Left motor"), ("right", "Right motor")] {
                properties.push(PropertyDef {
                    key: format!("profile-{n}-vibration-{side}"),
                    label: format!("{label} strength (%, profile {n})"),
                    // The vendor stores the motor strength as 0.0..1.0 in steps of 0.2, so the
                    // percent it is shown as moves in steps of 20.
                    kind: PropKind::Number { min: 0.0, max: 100.0, step: 20.0, default: 0.0 },
                });
            }
            for slot in 0..4 {
                properties.push(PropertyDef {
                    key: paddle_key(profile, slot),
                    label: format!("{} (profile {n})", codec::MAP_SLOT_LABELS[codec::EXTRA_BUTTON_FIRST + slot]),
                    kind: PropKind::Choice { options: codec::target_options(), default: "none".into() },
                });
            }
            sections.push(PeripheralSection {
                id: format!("profile-{n}"),
                title: format!("Profile {n}"),
                properties,
            });
        }

        PeripheralSchema {
            device: self.device_id(),
            kind: PeripheralKind::GameController,
            sections,
            read_only: read_only(),
            unreported: unreported(),
        }
    }

    /// Reads the pad's settings: the whole profile image, 36 chunks.
    ///
    /// The read sends the vendor's chunk requests. They change no setting, but they are 36
    /// output reports, so the first run of this on hardware needs an explicit go-ahead.
    pub async fn read_settings(&mut self, device: &DeviceId) -> Result<Props, AdapterError> {
        self.read_props(device).await
    }

    /// The read itself: the whole profile image, 36 chunks.
    async fn read_props(&mut self, device: &DeviceId) -> Result<Props, AdapterError> {
        self.check_id(device)?;
        self.rebind_if_personality_changed().await?;
        let Some(mut vendor) = self.vendor.take() else {
            return Err(self.not_connected());
        };
        let pace = self.pace;
        let (vendor, result) = tokio::task::spawn_blocking(move || {
            let result = read_image(&mut *vendor, pace);
            (vendor, result)
        })
        .await
        .map_err(join_error)?;
        self.vendor = Some(vendor);
        let image = match result {
            Ok(image) => image,
            Err(e) => {
                // An I/O failure means the handle is dead (unplugged pad): close it, so the next
                // read opens a fresh one. Silence from an asleep pad keeps the handle.
                if matches!(e, AdapterError::Io(_)) {
                    self.vendor = None;
                    self.gamepad = None;
                    self.image = None;
                    self.bound = None;
                }
                return Err(e);
            }
        };
        let props = props_from_image(&image).map_err(refuse)?;
        self.image = Some(image);
        Ok(props)
    }

    /// The handle is bound to the personality present when it was opened. When the pad has
    /// since changed personality (the receiver was idle at boot and the pad linked later),
    /// that handle hears nothing: close it and open the current one.
    async fn rebind_if_personality_changed(&mut self) -> Result<(), AdapterError> {
        if self.vendor.is_none() && self.gamepad.is_none() {
            return Ok(());
        }
        let opener = self.opener.clone();
        let present = tokio::task::spawn_blocking(move || opener.personalities().first().copied())
            .await
            .map_err(join_error)?;
        if present.is_some() && present != self.bound {
            self.vendor = None;
            self.gamepad = None;
            self.image = None;
            self.bound = None;
            self.connect().await?;
        }
        Ok(())
    }

    /// Why the adapter cannot talk to the pad.
    fn not_connected(&self) -> AdapterError {
        match self.personality {
            Some(Personality::DInput) => refuse(
                "the DInput personality has no vendor channel (the receiver's DInput descriptor \
                 has no 0x81 output report, research 5.1), so the profile image cannot be read",
            ),
            Some(Personality::Idle) => refuse(
                "the receiver is idle: no controller is linked, so it cannot answer the profile \
                 read",
            ),
            _ => AdapterError::NotConnected,
        }
    }

    /// Writes the given settings back to the pad.
    ///
    /// The stick and trigger ranges and the four paddles of the **active** profile can be
    /// written: the research's own writer covers the range records, and the third-party layout
    /// documents the map writer as one short request-1 chunk per entry, followed by the vendor's
    /// own commit (request 6, argument `0x0123`). The button map of the other profiles, the
    /// active profile, the rumble records and the map rows of the face buttons stay read-only.
    async fn write_props(&mut self, device: &DeviceId, props: &Props) -> Result<(), AdapterError> {
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
                return Err(refuse(format!("the pad has no setting called {key}")));
            };
            if !schema.is_writable(key) {
                return Err(refuse(format!("{key} is read-only: {}", read_only_reason(key))));
            }
            check_value(def, value)?;
        }
        self.check_writable_profile(props)?;
        let packets = self.plan(props)?;
        let Some(mut vendor) = self.vendor.take() else {
            return Err(self.not_connected());
        };
        let pace = self.pace;
        let (vendor, result) = tokio::task::spawn_blocking(move || {
            let result = write_session(&mut *vendor, pace, &packets);
            (vendor, result)
        })
        .await
        .map_err(join_error)?;
        self.vendor = Some(vendor);
        result
    }

    /// The pad applies the map of the profile it is on, so only that profile's paddles are
    /// written. A paddle of another profile is refused instead of being written into a record
    /// the pad is not using.
    fn check_writable_profile(&self, props: &Props) -> Result<(), AdapterError> {
        let Some(image) = self.image.as_ref() else {
            return Ok(());
        };
        let active = codec::active_profile(image).unwrap_or(0);
        for key in props.keys() {
            let Some((profile, _)) = paddle_slot(key) else { continue };
            if profile != active {
                return Err(refuse(format!(
                    "{key} belongs to profile {}, but the pad is on profile {}: only the active \
                     profile's paddles are written",
                    profile + 1,
                    active + 1
                )));
            }
        }
        Ok(())
    }

    /// The packets a batch of changes needs: one field write per record or map entry, then the
    /// commit.
    fn plan(&self, props: &Props) -> Result<Vec<Vec<u8>>, AdapterError> {
        let Some(image) = self.image.as_ref() else {
            return Err(refuse("the profile has not been read yet; read the device first"));
        };
        let mut records: Vec<(usize, Vec<u8>)> = Vec::new();
        for (key, value) in props {
            if let Some((profile, slot)) = paddle_slot(key) {
                let PropValue::Choice(target) = value else {
                    return Err(refuse(format!("{key} does not take that kind of value")));
                };
                let (at, entry) = codec::patched_map_entry(image, profile, slot, target)
                    .map_err(refuse)?;
                if !records.iter().any(|(a, _)| *a == at) {
                    records.push((at, entry));
                }
                continue;
            }
            let PropValue::Number(n) = value else {
                return Err(refuse(format!("{key} does not take that kind of value")));
            };
            let (at, record) = codec::patched_record(image, key, *n).map_err(refuse)?;
            if !records.iter().any(|(a, _)| *a == at) {
                records.push((at, record));
            }
        }
        let mut packets = Vec::new();
        for (at, record) in &records {
            packets.push(codec::write_request(*at, record).map_err(refuse)?);
        }
        packets.push(codec::commit_request());
        Ok(packets)
    }

    /// Reads what the pad reports outside its settings: the battery, and the paddles in the
    /// DInput personality.
    ///
    /// In the XInput personality the battery comes from the XInput API, which has four coarse
    /// levels and no percent, so no percentage is ever reported.
    pub async fn read_events(&mut self, timeout_ms: i32) -> Result<Vec<PeripheralEvent>, AdapterError> {
        let mut out = Vec::new();
        for slot in 0..4 {
            let Some(now) = self.xinput.state(slot) else { continue };
            // The XInput driver's packet counter moves on its own: the pad's heartbeat moves it
            // about every two seconds whatever is pressed (the 2026-10-27 runs). A moved counter
            // therefore says nothing about the paddles, so there is no notice here: the paddle
            // experiment measures the heartbeat and is the only thing that can say something.
            self.last_xinput = Some(now);
        }
        if let Some((kind, level)) = self.xinput.battery(0) {
            out.push(PeripheralEvent::Battery { level: None, charging: false });
            out.push(PeripheralEvent::Notice {
                message: format!(
                    "xinput-battery={} type={}",
                    codec::battery_level_name(level),
                    codec::battery_type_name(kind)
                ),
            });
        }
        if let Some(mut gamepad) = self.gamepad.take() {
            let (gamepad, events) = tokio::task::spawn_blocking(move || {
                let mut buf = [0u8; 64];
                let mut out = Vec::new();
                match gamepad.read_input(&mut buf, timeout_ms) {
                    Ok(0) => {}
                    Ok(n) => match codec::decode_dinput(&buf[..n]) {
                        Ok(report) => {
                            if let Some(level) = report.battery {
                                out.push(PeripheralEvent::Battery {
                                    level: Some(level),
                                    charging: report.charging,
                                });
                            }
                            if report.left_paddle() {
                                out.push(PeripheralEvent::Notice {
                                    message: "left-paddle".into(),
                                });
                            }
                            if report.right_paddle() {
                                out.push(PeripheralEvent::Notice {
                                    message: "right-paddle".into(),
                                });
                            }
                            if report.l4() {
                                out.push(PeripheralEvent::Notice { message: "l4".into() });
                            }
                            if report.r4() {
                                out.push(PeripheralEvent::Notice { message: "r4".into() });
                            }
                        }
                        Err(why) => tracing::warn!("the pad sent a malformed gamepad report: {why}"),
                    },
                    Err(e) => tracing::warn!("the pad gamepad read failed: {e}"),
                }
                (gamepad, out)
            })
            .await
            .map_err(join_error)?;
            self.gamepad = Some(gamepad);
            out.extend(events);
        }
        Ok(out)
    }

    /// The read-only paddle experiment of the research (section 6, experiment E1).
    ///
    /// It sends nothing: it reads the XInput state of every slot and the input reports of every
    /// HID collection the pad shows, and says what moved.
    ///
    /// The first [`IDLE_WINDOW`] seconds are the idle window: nothing is pressed then, so the
    /// pad's own heartbeat is measured over it. After that a packet-only move (the counter moved
    /// and nothing else did) is a **candidate** only when it breaks that rhythm: the heartbeat
    /// moves the counter steadily whether or not a paddle is pressed, so the signature on its
    /// own is a false positive (the 2026-10-27 runs show the same ~31-step rhythm with and
    /// without a press).
    pub async fn paddle_probe(&mut self, seconds: f64) -> Result<PaddleProbe, AdapterError> {
        let opener = self.opener.clone();
        let (collections, mut probe) = tokio::task::spawn_blocking(move || {
            let collections = opener.open_collections().unwrap_or_default();
            let probe = PaddleProbe::default();
            (collections, probe)
        })
        .await
        .map_err(join_error)?;
        let mut collections = collections;
        let start = std::time::Instant::now();
        let idle_until = start + IDLE_WINDOW;
        let deadline = start + Duration::from_secs_f64(seconds.max(0.0));
        let mut samples: Vec<codec::Sample> = Vec::new();
        let mut last: Vec<Option<XState>> = vec![None; 4];
        let mut previous: Vec<Option<Vec<u8>>> = vec![None; collections.len()];
        while std::time::Instant::now() < deadline {
            let at = std::time::Instant::now();
            let idle = at < idle_until;
            for (slot, last_slot) in last.iter_mut().enumerate() {
                let Some(now) = self.xinput.state(slot as u32) else { continue };
                if last_slot.is_none_or(|prev| prev != now) {
                    samples.push(codec::Sample { state: now, idle });
                }
                *last_slot = Some(now);
            }
            for (i, (label, transport)) in collections.iter_mut().enumerate() {
                let mut buf = [0u8; 64];
                match transport.read_input(&mut buf, 1) {
                    Ok(0) => {}
                    Ok(n) => {
                        let bytes = buf[..n].to_vec();
                        if previous[i].as_ref() != Some(&bytes) {
                            probe.hid_changes.push((label.clone(), codec::hex(&bytes)));
                            previous[i] = Some(bytes);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("a collection read failed: {e}");
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        for (label, _) in &collections {
            probe.opened.push(label.clone());
        }
        probe.xinput = codec::analyse_run(&samples);
        Ok(probe)
    }
}

/// What the read-only paddle experiment saw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaddleProbe {
    /// The collections the experiment could open.
    pub opened: Vec<String>,
    /// What the XInput slots showed: the pad's own heartbeat, the buttons and triggers it
    /// reported, and the packet-only moves that broke the heartbeat's rhythm.
    pub xinput: codec::XInputRun,
    /// The changes the HID collections showed: the collection and the bytes it showed.
    pub hid_changes: Vec<(String, String)>,
}

#[async_trait]
impl PeripheralAdapter for EightBitDoUltimate2 {
    fn name(&self) -> &'static str {
        ADAPTER_NAME
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        if self.vendor.is_some() || self.gamepad.is_some() {
            return Ok(());
        }
        let opener = self.opener.clone();
        let personalities = tokio::task::spawn_blocking(move || opener.personalities())
            .await
            .map_err(join_error)?;
        let Some(personality) = personalities.first().copied() else {
            return Err(AdapterError::NotConnected);
        };
        self.personality = Some(personality);
        let opener = self.opener.clone();
        let (vendor, gamepad) = tokio::task::spawn_blocking(move || {
            let vendor = opener.open_vendor(personality).map_err(AdapterError::Protocol)?;
            let gamepad = opener.open_gamepad(personality).map_err(AdapterError::Protocol)?;
            Ok::<_, AdapterError>((vendor, gamepad))
        })
        .await
        .map_err(join_error)??;
        self.vendor = vendor;
        self.gamepad = gamepad;
        self.bound = Some(personality);
        if self.vendor.is_none() && self.gamepad.is_none() {
            return Err(self.not_connected());
        }
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        self.vendor = None;
        self.gamepad = None;
        self.bound = None;
        self.image = None;
        self.last_xinput = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.vendor.is_some() || self.gamepad.is_some()
    }

    async fn discover(&mut self) -> Result<Vec<PeripheralInfo>, AdapterError> {
        let opener = self.opener.clone();
        let personalities = tokio::task::spawn_blocking(move || opener.personalities())
            .await
            .map_err(join_error)?;
        let Some(personality) = personalities.first().copied() else {
            return Ok(Vec::new());
        };
        self.personality = Some(personality);
        Ok(vec![self.info()])
    }

    async fn describe(&mut self, device: &DeviceId) -> Result<PeripheralSchema, AdapterError> {
        self.check_id(device)?;
        Ok(self.schema())
    }

    async fn read_settings(&mut self, device: &DeviceId) -> Result<Props, AdapterError> {
        self.read_props(device).await
    }

    async fn write_settings(&mut self, device: &DeviceId, props: &Props) -> Result<(), AdapterError> {
        self.write_props(device, props).await
    }

    fn writes_enabled(&self) -> bool {
        self.writes_enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, VecDeque};
    use std::sync::Mutex;

    /// What the fake pad holds.
    #[derive(Default)]
    struct Shared {
        /// The personalities the fake shows.
        personalities: Vec<Personality>,
        /// The image the fake answers the profile read from.
        image: Vec<u8>,
        /// Every report the adapter sent, in order.
        sent: Vec<Vec<u8>>,
        /// The replies the fake has ready, in order. `None` means the fake answers from the image.
        queued: VecDeque<Option<Vec<u8>>>,
        /// The gamepad reports the fake hands over.
        gamepad: VecDeque<Vec<u8>>,
        /// The collections the fake can open.
        collections: Vec<String>,
        opens: u32,
        /// Makes every vendor read fail with an I/O error, as an unplugged pad does.
        fail_io: bool,
        /// How the fake answers a write chunk (request word 1).
        write_ack: Ack,
        /// How the fake answers the commit (request word 6).
        commit_ack: Ack,
    }

    /// How the fake pad answers a write or commit request.
    #[derive(Clone, Copy, Default, PartialEq)]
    enum Ack {
        /// A good acknowledgement of the request word.
        #[default]
        Good,
        /// An acknowledgement of another request word (a read's).
        WrongWord,
        /// No reply.
        Silent,
    }

    struct FakeOpener(Arc<Mutex<Shared>>);
    /// A vendor handle bound to the collection of one personality; it only hears the pad while
    /// that personality is the one present.
    struct FakeVendor(Arc<Mutex<Shared>>, Personality);
    struct FakeGamepad(Arc<Mutex<Shared>>);

    /// An XInput API that has nothing in it, so a test never depends on what is plugged in.
    struct FakeXInput;

    impl XInputSource for FakeXInput {
        fn state(&self, _slot: u32) -> Option<XState> {
            None
        }

        fn battery(&self, _slot: u32) -> Option<(u8, u8)> {
            None
        }
    }

    impl InputTransport for FakeVendor {
        fn read_input(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            let mut s = self.0.lock().unwrap();
            if s.fail_io {
                return Err("the device is gone".into());
            }
            if s.personalities.first() != Some(&self.1) {
                return Ok(0);
            }
            let queued = s.queued.pop_front();
            match queued {
                Some(Some(bytes)) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    Ok(n)
                }
                Some(None) => Ok(0),
                None => {
                    // Answer the last request by its request word: reads from the image, writes
                    // and commits as the test set the fake up; everything else is not answered.
                    let request = s.sent.last().cloned().unwrap_or_default();
                    if request.len() < 18 || request[0] != codec::OUT_ID || request[1] != 0x04 {
                        return Ok(0);
                    }
                    let word = u16::from_le_bytes([request[2], request[3]]);
                    let ack = match word {
                        codec::REQ_WRITE => Some(s.write_ack),
                        codec::REQ_COMMIT => Some(s.commit_ack),
                        _ => None,
                    };
                    if let Some(ack) = ack {
                        let reply = match ack {
                            Ack::Silent => return Ok(0),
                            Ack::Good => codec::ack_frame(word),
                            Ack::WrongWord => codec::ack_frame(codec::REQ_READ),
                        };
                        let n = reply.len().min(buf.len());
                        buf[..n].copy_from_slice(&reply[..n]);
                        return Ok(n);
                    }
                    if word != codec::REQ_READ {
                        return Ok(0);
                    }
                    let len = (u32::from_le_bytes(request[6..10].try_into().unwrap()) & 0xFFFF) as usize;
                    let offset = u32::from_le_bytes(request[14..18].try_into().unwrap()) as usize;
                    if offset >= s.image.len() {
                        return Ok(0);
                    }
                    let end = (offset + len).min(s.image.len());
                    let data = &s.image[offset..end];
                    let reply = codec::reply_frame(len, true, offset as u32, data);
                    let n = reply.len().min(buf.len());
                    buf[..n].copy_from_slice(&reply[..n]);
                    Ok(n)
                }
            }
        }
    }

    impl VendorTransport for FakeVendor {
        fn send_report(&mut self, data: &[u8]) -> Result<(), String> {
            let mut s = self.0.lock().unwrap();
            s.sent.push(data.to_vec());
            // A write request (request word 1) carries bytes into the image at its offset. The
            // fake is a small model of the pad, so a re-read shows what was written.
            if data.len() >= codec::DATA_AT
                && data[0] == codec::OUT_ID
                && u16::from_le_bytes([data[2], data[3]]) == codec::REQ_WRITE
            {
                let word = u32::from_le_bytes(data[6..10].try_into().unwrap());
                let len = (word & 0xFFFF) as usize;
                let offset = u32::from_le_bytes(data[14..18].try_into().unwrap()) as usize;
                if codec::DATA_AT + len <= data.len() && offset + len <= s.image.len() {
                    s.image[offset..offset + len]
                        .copy_from_slice(&data[codec::DATA_AT..codec::DATA_AT + len]);
                }
            }
            Ok(())
        }
    }

    impl InputTransport for FakeGamepad {
        fn read_input(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            let mut s = self.0.lock().unwrap();
            match s.gamepad.pop_front() {
                None => Ok(0),
                Some(report) => {
                    let n = report.len().min(buf.len());
                    buf[..n].copy_from_slice(&report[..n]);
                    Ok(n)
                }
            }
        }
    }

    impl Opener for FakeOpener {
        fn personalities(&self) -> Vec<Personality> {
            self.0.lock().unwrap().personalities.clone()
        }

        fn open_vendor(
            &self,
            personality: Personality,
        ) -> Result<Option<Box<dyn VendorTransport>>, String> {
            let mut s = self.0.lock().unwrap();
            if s.personalities.is_empty() {
                return Ok(None);
            }
            s.opens += 1;
            Ok(Some(Box::new(FakeVendor(self.0.clone(), personality))))
        }

        fn open_gamepad(
            &self,
            personality: Personality,
        ) -> Result<Option<Box<dyn InputTransport>>, String> {
            let s = self.0.lock().unwrap();
            if s.personalities.is_empty() || personality != Personality::DInput {
                return Ok(None);
            }
            Ok(Some(Box::new(FakeGamepad(self.0.clone()))))
        }

        fn open_collections(&self) -> Result<Vec<(String, Box<dyn InputTransport>)>, String> {
            let s = self.0.lock().unwrap();
            Ok(s
                .collections
                .iter()
                .map(|label| (label.clone(), Box::new(FakeGamepad(self.0.clone())) as Box<dyn InputTransport>))
                .collect())
        }
    }

    /// The adapter over a fake pad that answers with the synthetic image.
    fn rig() -> (EightBitDoUltimate2, Arc<Mutex<Shared>>) {
        let shared = Arc::new(Mutex::new(Shared {
            personalities: vec![Personality::XInput],
            image: codec::synthetic_image(),
            ..Default::default()
        }));
        let mut adapter = EightBitDoUltimate2::new();
        adapter.opener = Arc::new(FakeOpener(shared.clone()));
        adapter.xinput = Arc::new(FakeXInput);
        adapter.pace = Pace {
            poll_ms: 1,
            chunk_pause: Duration::ZERO,
            write_pause: Duration::ZERO,
            session_pause: Duration::ZERO,
        };
        (adapter, shared)
    }

    fn number(pairs: &[(&str, f64)]) -> Props {
        pairs.iter().map(|(k, v)| ((*k).to_string(), PropValue::Number(*v))).collect()
    }

    fn choice(pairs: &[(&str, &str)]) -> Props {
        pairs.iter().map(|(k, v)| ((*k).to_string(), PropValue::Choice((*v).into()))).collect()
    }

    // ------------------------------------------------------------------------------ discovery

    #[tokio::test]
    async fn discover_is_empty_when_the_pad_is_not_there() {
        let (mut a, s) = rig();
        s.lock().unwrap().personalities.clear();
        assert!(a.discover().await.unwrap().is_empty());
        assert!(matches!(a.connect().await, Err(AdapterError::NotConnected)));
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn discover_names_the_pad_and_says_which_personality_is_present() {
        for (personality, name) in [
            (Personality::XInput, "XInput"),
            (Personality::Idle, "2.4G receiver, idle"),
            (Personality::DInput, "DInput"),
        ] {
            let (mut a, s) = rig();
            s.lock().unwrap().personalities = vec![personality];
            let found = a.discover().await.unwrap();
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].id, DeviceId("eightbitdo-ultimate-2:2dc8".into()));
            assert_eq!(found[0].name, format!("8BitDo Ultimate 2 Wireless ({name})"));
            assert_eq!(found[0].kind, PeripheralKind::GameController);
            assert_eq!(found[0].adapter, "eightbitdo-ultimate-2");
            assert_eq!(found[0].location, format!("hid:2dc8:{:04x}", personality.product_id()));
            assert!(!found[0].connected);
            assert_eq!(s.lock().unwrap().opens, 0, "the enumeration opens nothing");
            assert_eq!(a.name(), "eightbitdo-ultimate-2");
        }
    }

    #[tokio::test]
    async fn connect_opens_the_collections_and_disconnect_closes_them() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        assert!(a.is_connected());
        assert_eq!(s.lock().unwrap().opens, 1);
        a.connect().await.unwrap();
        assert_eq!(s.lock().unwrap().opens, 1, "a second connect is a no-op");
        a.disconnect().await.unwrap();
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn a_handle_opened_in_the_idle_personality_is_replaced_when_the_pad_wakes() {
        let (mut a, s) = rig();
        s.lock().unwrap().personalities = vec![Personality::Idle];
        a.connect().await.unwrap();
        // The pad links up: the receiver now shows the XInput personality.
        s.lock().unwrap().personalities = vec![Personality::XInput];
        let id = a.device_id();
        let props = a.read_settings(&id).await;
        assert!(props.is_ok(), "the read must reopen on the new personality: {props:?}");
        assert_eq!(s.lock().unwrap().opens, 2, "one open per personality");
    }

    #[tokio::test]
    async fn a_silent_pad_keeps_its_handle() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        s.lock().unwrap().image.clear(); // the fake stops answering: every poll returns 0 bytes
        let id = a.device_id();
        assert!(a.read_settings(&id).await.is_err());
        assert!(a.is_connected(), "an asleep pad is silence, not a dead handle");
        assert_eq!(s.lock().unwrap().opens, 1, "no reopen churn");
    }

    #[tokio::test]
    async fn a_hid_failure_closes_the_handle_so_the_next_read_reopens() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        s.lock().unwrap().fail_io = true;
        let id = a.device_id();
        assert!(a.read_settings(&id).await.is_err());
        assert!(!a.is_connected(), "a dead handle must not stay open");
        s.lock().unwrap().fail_io = false;
        a.connect().await.unwrap();
        assert!(a.read_settings(&id).await.is_ok());
        assert_eq!(s.lock().unwrap().opens, 2);
    }

    // -------------------------------------------------------------------------------- describe

    #[tokio::test]
    async fn the_schema_has_the_sections_the_pad_has() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        assert_eq!(schema.kind, PeripheralKind::GameController);
        let ids: Vec<&str> = schema.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["profiles", "buttons", "profile-1", "profile-2", "profile-3"]);
        assert_eq!(schema.sections[0].title, "Profiles");
        assert_eq!(schema.sections[2].title, "Profile 1");
    }

    #[tokio::test]
    async fn the_schema_carries_the_real_ranges_steps_and_options() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        let kind = |key: &str| schema.property(key).unwrap().kind.clone();
        assert_eq!(
            kind("active-profile"),
            PropKind::Choice { options: vec!["1".into(), "2".into(), "3".into()], default: "1".into() }
        );
        assert_eq!(kind("profile-1-in-use"), PropKind::Bool { default: false });
        // The ranges are percentages of the travel.
        assert_eq!(
            kind("profile-1-stick-left-start"),
            PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 0.0 }
        );
        assert_eq!(
            kind("profile-2-trigger-right-end"),
            PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 0.0 }
        );
        // The motor strengths are 0 to 100 in steps of 20: the vendor stores 0.0..1.0 in 0.2 steps.
        assert_eq!(
            kind("profile-3-vibration-left"),
            PropKind::Number { min: 0.0, max: 100.0, step: 20.0, default: 0.0 }
        );
        // A button row offers the named targets and "another action".
        let PropKind::Choice { options, .. } = kind("button-a") else { panic!("button-a is a choice") };
        assert!(options.contains(&"a".to_string()));
        assert!(options.contains(&"other".to_string()));
        // Every target the sources name, and "another action" on top.
        assert_eq!(options.len(), codec::TARGETS.len() + 1);
        for (id, _) in codec::TARGETS {
            assert!(options.contains(&id.to_string()), "{id} is missing");
        }
        let PropKind::Choice { options, .. } = kind("profile-1-paddle-1") else {
            panic!("profile-1-paddle-1 is a choice")
        };
        assert!(options.contains(&"none".to_string()));
    }

    #[tokio::test]
    async fn the_active_profile_the_map_and_the_motors_are_read_only() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        assert!(!schema.is_writable("active-profile"));
        assert!(!schema.is_writable("profile-1-in-use"));
        for slot in 0..codec::EXTRA_BUTTON_FIRST {
            assert!(!schema.is_writable(&button_key(slot)), "{slot} must be read-only");
        }
        for profile in 0..codec::PROFILES {
            assert!(!schema.is_writable(&format!("profile-{}-vibration-left", profile + 1)));
        }
        // The ranges are the only things this adapter writes.
        for key in [
            "profile-1-stick-left-start",
            "profile-1-trigger-right-end",
            "profile-3-stick-right-start",
        ] {
            assert!(schema.is_writable(key), "{key} must be writable");
        }
        // The paddles of the active profile can be written: the map writer is the vendor's own
        // shape (one short chunk per entry, then the commit).
        for profile in 0..codec::PROFILES {
            for slot in 0..4 {
                assert!(
                    schema.is_writable(&paddle_key(profile, slot)),
                    "{} must be writable",
                    paddle_key(profile, slot)
                );
            }
        }
    }

    #[tokio::test]
    async fn the_settings_this_build_cannot_name_are_listed_as_unreported() {
        let (mut a, _) = rig();
        let schema = a.describe(&a.device_id()).await.unwrap();
        for key in ["profile-1-name", "profile-2-ring-theme", "profile-3-macros", "trigger-mode", "firmware-version"] {
            assert!(schema.unreported.contains(&key.to_string()), "{key} is not listed");
        }
    }

    // -------------------------------------------------------------------------------- the read

    #[tokio::test]
    async fn the_read_returns_the_values_the_image_holds() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        // The synthetic image has the active profile 3.
        assert_eq!(props.get("active-profile"), Some(&PropValue::Choice("3".into())));
        assert_eq!(props.get("profile-1-in-use"), Some(&PropValue::Bool(true)));
        assert_eq!(props.get("profile-2-in-use"), Some(&PropValue::Bool(false)));
        // Profile 1: the stick bytes 10, 120, 11, 121 as percentages.
        assert_eq!(
            props.get("profile-1-stick-left-start"),
            Some(&PropValue::Number(codec::byte_to_percent(10)))
        );
        assert_eq!(
            props.get("profile-1-stick-right-end"),
            Some(&PropValue::Number(codec::byte_to_percent(121)))
        );
        assert_eq!(
            props.get("profile-1-trigger-left-start"),
            Some(&PropValue::Number(codec::byte_to_percent(5)))
        );
        // The motors: 0.8 and 0.4 as the percent the vendor stores them in.
        assert_eq!(props.get("profile-1-vibration-left"), Some(&PropValue::Number(80.0)));
        assert_eq!(props.get("profile-1-vibration-right"), Some(&PropValue::Number(40.0)));
        // The four extra buttons of profile 1: A, none, Start and an unnamed mask.
        assert_eq!(props.get("profile-1-paddle-1"), Some(&PropValue::Choice("a".into())));
        assert_eq!(props.get("profile-1-paddle-2"), Some(&PropValue::Choice("none".into())));
        assert_eq!(props.get("profile-1-paddle-3"), Some(&PropValue::Choice("start".into())));
        assert_eq!(props.get("profile-1-paddle-4"), Some(&PropValue::Choice("other".into())));
        // The button map of the active profile (3): its map record is not in use, so every slot
        // holds nothing.
        assert_eq!(props.get("button-a"), Some(&PropValue::Choice("none".into())));
    }

    #[tokio::test]
    async fn describe_and_read_settings_agree_on_the_keys() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        let schema = a.describe(&a.device_id()).await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        let schema_keys: BTreeSet<&str> = schema.properties().map(|p| p.key.as_str()).collect();
        let read_keys: BTreeSet<&str> = props.keys().map(|k| k.as_str()).collect();
        assert_eq!(schema_keys, read_keys, "every property the pad has is read back");
        // Nothing is invented: a profile name is text and the property model has no text kind.
        assert!(schema.property("profile-1-name").is_none());
        assert!(!props.contains_key("profile-1-name"));
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
                (PropKind::Bool { .. }, PropValue::Bool(_)) => {}
                other => panic!("{} has the wrong kind of value: {other:?}", def.key),
            }
        }
    }

    #[tokio::test]
    async fn the_read_sends_the_thirty_six_chunk_requests_of_the_image() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let sent = s.lock().unwrap().sent.clone();
        assert_eq!(sent.len(), 36);
        for request in &sent {
            assert_eq!(request.len(), 64, "every report goes out at its full length");
            assert_eq!(request[0], 0x81);
            assert_eq!(request[1], 0x04);
            assert_eq!(u16::from_le_bytes([request[2], request[3]]), 2, "request word 2 is a read");
        }
        // The offsets walk the image in order.
        let mut at = 0usize;
        for request in &sent {
            assert_eq!(u32::from_le_bytes(request[14..18].try_into().unwrap()) as usize, at);
            at += (u32::from_le_bytes(request[6..10].try_into().unwrap()) & 0xFFFF) as usize;
        }
        assert_eq!(at, codec::IMAGE_LEN);
    }

    #[tokio::test]
    async fn a_pad_that_answers_nothing_is_an_error() {
        let (mut a, s) = rig();
        s.lock().unwrap().image = Vec::new();
        a.connect().await.unwrap();
        let err = a.read_settings(&a.device_id()).await.unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("did not answer")),
            "{err:?}"
        );
    }

    // ------------------------------------------------------------------------------- the events

    #[tokio::test]
    async fn the_dinput_personality_reports_the_battery_and_the_paddles() {
        let (mut a, s) = rig();
        s.lock().unwrap().personalities = vec![Personality::DInput];
        let mut report = vec![0u8; 34];
        report[0] = 0x04;
        report[8] = 0x20; // the left paddle
        report[14] = 0x80 | 42; // charging, 42 %
        s.lock().unwrap().gamepad.push_back(report);
        a.connect().await.unwrap();
        let events = a.read_events(10).await.unwrap();
        assert_eq!(
            events,
            vec![
                PeripheralEvent::Battery { level: Some(42), charging: true },
                PeripheralEvent::Notice { message: "left-paddle".into() },
            ]
        );
    }

    // ------------------------------------------------------------------------------- the writes

    #[tokio::test]
    async fn writes_are_off_until_a_caller_turns_them_on() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(
                &a.device_id(),
                &number(&[("profile-1-stick-left-start", 20.0)]),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AdapterError::Unsupported(_)), "{err:?}");
        assert_eq!(s.lock().unwrap().sent.len(), before, "nothing reached the pad");
        assert!(!a.writes_enabled());
    }

    #[tokio::test]
    async fn a_write_sends_the_field_write_and_then_the_commit() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        assert!(a.writes_enabled());
        a.write_settings(&a.device_id(), &number(&[("profile-1-stick-left-start", 20.0)]))
            .await
            .unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 4, "Reportstate(0), the write, the commit, Reportstate(1)");
        assert_eq!(sent[0], codec::report_state_request(0));
        assert_eq!(sent[1].len(), 64);
        assert_eq!(u16::from_le_bytes([sent[1][2], sent[1][3]]), 1, "request word 1 is a write");
        assert_eq!(sent[2], codec::commit_request());
        assert_eq!(sent[3], codec::report_state_request(1));
        // The byte that went out is the percentage the caller asked for.
        let field = codec::field_of("profile-1-stick-left-start").unwrap();
        let at = u32::from_le_bytes(sent[1][14..18].try_into().unwrap()) as usize;
        assert_eq!(at, field.record_at);
        assert_eq!(sent[1][codec::DATA_AT + field.byte_in_record], codec::percent_to_byte(20.0));
    }

    #[tokio::test]
    async fn a_write_without_a_prior_read_is_refused() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.set_writes_enabled(true);
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(&a.device_id(), &number(&[("profile-1-stick-left-start", 20.0)]))
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
            .write_settings(&a.device_id(), &number(&[("profile-9-stick-left-start", 20.0)]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("no setting called")),
            "{err:?}"
        );
        let err = a
            .write_settings(&a.device_id(), &choice(&[("profile-1-name", "Racing")]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("no setting called")),
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
        for (key, value) in [
            ("active-profile", PropValue::Choice("2".into())),
            ("button-a", PropValue::Choice("b".into())),
            ("profile-1-vibration-left", PropValue::Number(0.5)),
        ] {
            let err = a
                .write_settings(&a.device_id(), &Props::from([(key.to_string(), value)]))
                .await
                .unwrap_err();
            assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("read-only")), "{key}: {err:?}");
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
            (number(&[("profile-1-stick-left-start", 101.0)]), "takes 0 to 100"),
            (number(&[("profile-1-stick-left-start", -1.0)]), "takes 0 to 100"),
            (number(&[("profile-1-stick-left-start", 10.5)]), "steps of 1"),
        ] {
            let err = a.write_settings(&a.device_id(), &props).await.unwrap_err();
            assert!(matches!(&err, AdapterError::Protocol(m) if m.contains(why)), "{err:?} (wanted {why})");
        }
        assert_eq!(s.lock().unwrap().sent.len(), before, "no refused change reached the pad");
    }

    #[tokio::test]
    async fn a_value_of_the_wrong_shape_is_refused() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        let err = a
            .write_settings(
                &a.device_id(),
                &choice(&[("profile-1-stick-left-start", "20")]),
            )
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("kind of value")), "{err:?}");
    }

    #[tokio::test]
    async fn a_write_to_another_device_is_refused() {
        let (mut a, _) = rig();
        a.connect().await.unwrap();
        let other = DeviceId("eightbitdo-ultimate-2:2dc9".into());
        assert!(matches!(
            a.write_settings(&other, &number(&[("profile-1-stick-left-start", 20.0)])).await,
            Err(AdapterError::UnknownDevice(_))
        ));
        assert!(matches!(a.read_settings(&other).await, Err(AdapterError::UnknownDevice(_))));
        assert!(matches!(a.describe(&other).await, Err(AdapterError::UnknownDevice(_))));
    }

    #[tokio::test]
    async fn the_unchanged_value_write_leaves_every_byte_alone() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        let props = a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        let change: Props = props
            .iter()
            .filter(|(k, _)| k.starts_with("profile-1-stick") || k.starts_with("profile-1-trigger"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        a.write_settings(&a.device_id(), &change).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        // Two records changed (the stick one and the trigger one), so Reportstate(0), two writes,
        // the commit and Reportstate(1).
        assert_eq!(sent.len(), 5);
        let image = codec::synthetic_image();
        for packet in &sent[1..3] {
            let at = u32::from_le_bytes(packet[14..18].try_into().unwrap()) as usize;
            let len = (u32::from_le_bytes(packet[6..10].try_into().unwrap()) & 0xFFFF) as usize;
            assert_eq!(&packet[codec::DATA_AT..codec::DATA_AT + len], &image[at..at + len]);
        }
    }

    // ---------------------------------------------------------------------------- the paddles

    #[tokio::test]
    async fn a_paddle_write_sends_the_entry_and_the_commit_and_the_pad_holds_it() {
        // The synthetic image has profile 3 as the active profile, so profile 3's paddles are the
        // ones the pad applies.
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let image_before = s.lock().unwrap().image.clone();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        a.write_settings(&a.device_id(), &choice(&[("profile-3-paddle-1", "a")])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 4, "Reportstate(0), the entry, the commit, Reportstate(1)");

        // The write request: request word 1, the entry's offset, four bytes of data.
        let at = codec::map_entry_offset(2, 18).expect("profile 3's first paddle");
        let write = &sent[1];
        assert_eq!(write.len(), codec::REPORT_LEN);
        assert_eq!(u16::from_le_bytes([write[2], write[3]]), 1, "request word 1 is a write");
        assert_eq!(u32::from_le_bytes(write[14..18].try_into().unwrap()) as usize, at);
        let word = u32::from_le_bytes(write[6..10].try_into().unwrap());
        assert_eq!((word & 0xFFFF) as usize, 4, "four bytes: one target mask");
        assert_eq!(word >> 16, u32::from(codec::crc16(&[0x00, 0x20, 0x00, 0x00])), "the CRC of the entry");
        assert_eq!(&write[codec::DATA_AT..codec::DATA_AT + 4], &[0x00, 0x20, 0x00, 0x00]);

        // The commit: request 6 with the argument 0x0123, the vendor's own packet.
        assert_eq!(sent[2], codec::commit_request());
        assert_eq!(u16::from_le_bytes([sent[2][2], sent[2][3]]), 6);
        assert_eq!(u16::from_le_bytes([sent[2][4], sent[2][5]]), 0x0123);

        // The pad holds it: the re-read shows the new mapping and nothing else moved.
        let props = a.read_settings(&a.device_id()).await.unwrap();
        assert_eq!(props.get("profile-3-paddle-1"), Some(&PropValue::Choice("a".into())));
        assert_eq!(props.get("profile-3-paddle-2"), Some(&PropValue::Choice("none".into())));
        let image_after = s.lock().unwrap().image.clone();
        let moved: Vec<usize> = (0..image_after.len())
            .filter(|i| image_before[*i] != image_after[*i])
            .collect();
        assert!(
            moved.iter().all(|i| (at..at + 4).contains(i)),
            "only the entry moved: {moved:?}"
        );
        assert_eq!(&image_after[at..at + 4], &[0x00, 0x20, 0x00, 0x00], "the mask of A");
    }

    #[tokio::test]
    async fn a_paddle_of_a_profile_the_pad_is_not_on_is_refused() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        let err = a
            .write_settings(&a.device_id(), &choice(&[("profile-1-paddle-1", "a")]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("active profile")),
            "{err:?}"
        );
        assert_eq!(s.lock().unwrap().sent.len(), before, "nothing reached the pad");
    }

    #[tokio::test]
    async fn two_paddles_of_the_active_profile_go_out_as_two_chunks_and_one_commit() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        a.write_settings(
            &a.device_id(),
            &choice(&[("profile-3-paddle-1", "a"), ("profile-3-paddle-2", "b")]),
        )
        .await
        .unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 5, "Reportstate(0), two entries, the commit, Reportstate(1)");
        let first = u32::from_le_bytes(sent[1][14..18].try_into().unwrap()) as usize;
        let second = u32::from_le_bytes(sent[2][14..18].try_into().unwrap()) as usize;
        assert_eq!(first, codec::map_entry_offset(2, 18).unwrap());
        assert_eq!(second, codec::map_entry_offset(2, 19).unwrap());
        assert_eq!(sent[3], codec::commit_request());
        assert_eq!(sent[4], codec::report_state_request(1));
    }

    #[tokio::test]
    async fn a_paddle_target_this_build_does_not_name_is_refused() {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        let before = s.lock().unwrap().sent.len();
        a.set_writes_enabled(true);
        let err = a
            .write_settings(&a.device_id(), &choice(&[("profile-3-paddle-1", "other")]))
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("not a target")), "{err:?}");
        assert_eq!(s.lock().unwrap().sent.len(), before);
    }

    // ------------------------------------------------------------------ the write session

    fn hx(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// Pad out a packet's leading bytes to a 64-byte report.
    fn report(lead: &str) -> Vec<u8> {
        let mut v = hx(lead);
        v.resize(codec::REPORT_LEN, 0);
        v
    }

    async fn ready() -> (EightBitDoUltimate2, Arc<Mutex<Shared>>) {
        let (mut a, s) = rig();
        a.connect().await.unwrap();
        a.read_settings(&a.device_id()).await.unwrap();
        a.set_writes_enabled(true);
        (a, s)
    }

    #[tokio::test]
    async fn the_whole_session_is_byte_exact() {
        // Profile 3 is active in the synthetic image; its first paddle sits at map_entry_offset.
        let (mut a, s) = ready().await;
        let before = s.lock().unwrap().sent.len();
        a.write_settings(&a.device_id(), &choice(&[("profile-3-paddle-1", "a")])).await.unwrap();
        let sent = s.lock().unwrap().sent[before..].to_vec();
        let at = codec::map_entry_offset(2, 18).unwrap() as u32;
        let data = [0x00u8, 0x20, 0x00, 0x00];
        let mut write = report("81 04 01 00 00 00");
        write[6..10].copy_from_slice(&(4u32 | (u32::from(codec::crc16(&data)) << 16)).to_le_bytes());
        write[10..14].copy_from_slice(&0x638u32.to_le_bytes());
        write[14..18].copy_from_slice(&at.to_le_bytes());
        write[18..22].copy_from_slice(&data);
        let expected = vec![
            report("81 04 07 00 00 00"),
            write,
            report("81 04 06 00 23 01"),
            report("81 04 07 00 01 00"),
        ];
        assert_eq!(sent, expected);
        // No packet uses the length-byte form.
        assert!(sent.iter().all(|p| p[1] == 0x04));
    }

    #[tokio::test]
    async fn a_wrong_reply_to_the_write_stops_before_the_commit_but_closes_the_bracket() {
        let (mut a, s) = ready().await;
        s.lock().unwrap().write_ack = Ack::WrongWord;
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(&a.device_id(), &choice(&[("profile-3-paddle-1", "a")]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AdapterError::Protocol(m) if m.contains("wrong") && m.contains("request word 2")),
            "{err:?}"
        );
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 3, "Reportstate(0), the write, Reportstate(1): no commit");
        assert_eq!(sent[0], codec::report_state_request(0));
        assert_eq!(sent[2], codec::report_state_request(1));
        assert!(sent.iter().all(|p| *p != codec::commit_request()));
    }

    #[tokio::test]
    async fn no_reply_to_the_write_stops_before_the_commit_but_closes_the_bracket() {
        let (mut a, s) = ready().await;
        s.lock().unwrap().write_ack = Ack::Silent;
        let before = s.lock().unwrap().sent.len();
        let err = a
            .write_settings(&a.device_id(), &choice(&[("profile-3-paddle-1", "a")]))
            .await
            .unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("did not reply")), "{err:?}");
        let sent = s.lock().unwrap().sent[before..].to_vec();
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2], codec::report_state_request(1));
        assert!(sent.iter().all(|p| *p != codec::commit_request()));
    }

    #[tokio::test]
    async fn a_bad_commit_reply_is_an_error_and_the_bracket_still_closes() {
        for ack in [Ack::WrongWord, Ack::Silent] {
            let (mut a, s) = ready().await;
            s.lock().unwrap().commit_ack = ack;
            let before = s.lock().unwrap().sent.len();
            let err = a
                .write_settings(&a.device_id(), &choice(&[("profile-3-paddle-1", "a")]))
                .await
                .unwrap_err();
            assert!(
                matches!(&err, AdapterError::Protocol(m) if m.contains("commit") && m.contains("uncommitted")),
                "{err:?}"
            );
            let sent = s.lock().unwrap().sent[before..].to_vec();
            assert_eq!(sent.len(), 4, "the commit went out, then Reportstate(1)");
            assert_eq!(sent[2], codec::commit_request());
            assert_eq!(sent[3], codec::report_state_request(1));
        }
    }

    #[tokio::test]
    async fn a_stale_reply_queued_before_the_write_is_dropped_not_mistaken_for_its_answer() {
        let (mut a, s) = ready().await;
        // A late answer to Reportstate(0): request word 7.
        s.lock().unwrap().queued.push_back(Some(codec::ack_frame(7)));
        a.write_settings(&a.device_id(), &choice(&[("profile-3-paddle-1", "a")])).await.unwrap();
    }

    // ---------------------------------------------------------------------------- the fake device

    #[test]
    fn the_fake_pad_answers_the_read_requests_from_its_image() {
        // The fake and the codec agree on the request layout: a request carries the offset it
        // asks for and the length of the chunk.
        let request = codec::read_request(0x2D).unwrap();
        assert_eq!(u32::from_le_bytes(request[14..18].try_into().unwrap()), 0x2D);
        let len = (u32::from_le_bytes(request[6..10].try_into().unwrap()) & 0xFFFF) as usize;
        assert_eq!(len, codec::CHUNK);
        let image = codec::synthetic_image();
        let reply = codec::reply_frame(len, true, 0x2D, &image[0x2D..0x2D + len]);
        assert_eq!(codec::parse_read_reply(&reply, len, 0x2D).unwrap(), &image[0x2D..0x2D + len]);
    }

    #[tokio::test]
    async fn the_read_only_paddle_probe_never_sends_anything() {
        // The probe opens collections and reads them; it never writes a report.
        let (mut a, s) = rig();
        s.lock().unwrap().collections = vec!["pid=310b if=2 up=ff7a u=0001".into()];
        let probe = a.paddle_probe(0.0).await.unwrap();
        assert_eq!(probe.opened, vec!["pid=310b if=2 up=ff7a u=0001".to_string()]);
        assert!(s.lock().unwrap().sent.is_empty(), "the probe sent nothing");
    }

    // ------------------------------------------------------ the HID context (issue #7)

    use crate::attack_shark_x6::hid_cache::{HidCache, Scan};
    use std::cell::Cell;

    thread_local! {
        static SCANS: Cell<u32> = const { Cell::new(0) };
        static STALE_OPENS: Cell<u32> = const { Cell::new(0) };
        static PLUGGED: Cell<bool> = const { Cell::new(true) };
    }

    struct CountingApi {
        plugged: bool,
    }

    impl Scan for CountingApi {
        fn first() -> Result<Self, String> {
            SCANS.with(|s| s.set(s.get() + 1));
            Ok(CountingApi { plugged: PLUGGED.with(Cell::get) })
        }

        fn rescan(&mut self) -> Result<(), String> {
            SCANS.with(|s| s.set(s.get() + 1));
            self.plugged = PLUGGED.with(Cell::get);
            Ok(())
        }
    }

    fn stale_open() -> bool {
        STALE_OPENS.with(|s| s.replace(s.get().saturating_sub(1)) > 0)
    }

    struct NullVendor;

    impl InputTransport for NullVendor {
        fn read_input(&mut self, _buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            Ok(0)
        }
    }

    impl VendorTransport for NullVendor {
        fn send_report(&mut self, _data: &[u8]) -> Result<(), String> {
            Ok(())
        }
    }

    impl HidSource for CountingApi {
        fn find_path(&self, product_id: u16, _page: u16, _usage: u16) -> Option<std::ffi::CString> {
            (self.plugged && product_id == PRODUCT_ID_DINPUT)
                .then(|| std::ffi::CString::new("fake/8bitdo").unwrap())
        }

        fn has_product(&self, product_id: u16) -> bool {
            self.plugged && product_id == PRODUCT_ID_DINPUT
        }

        fn open_vendor_at(&self, _p: &std::ffi::CStr) -> Result<Box<dyn VendorTransport>, String> {
            if stale_open() {
                return Err("the path is gone".into());
            }
            Ok(Box::new(NullVendor))
        }

        fn open_gamepad_at(&self, _p: &std::ffi::CStr) -> Result<Box<dyn InputTransport>, String> {
            if stale_open() {
                return Err("the path is gone".into());
            }
            Ok(Box::new(NullVendor))
        }

        fn open_all(&self) -> OpenedCollections {
            if !self.plugged || stale_open() {
                return Vec::new();
            }
            vec![("fake".to_string(), Box::new(NullVendor) as Box<dyn InputTransport>)]
        }
    }

    fn opener() -> HidOpener<CountingApi> {
        SCANS.with(|s| s.set(0));
        STALE_OPENS.with(|s| s.set(0));
        PLUGGED.with(|s| s.set(true));
        HidOpener { cache: HidCache::with_ttl(Duration::from_secs(3600)) }
    }

    fn scans() -> u32 {
        SCANS.with(Cell::get)
    }

    #[test]
    fn a_stale_vendor_path_is_rescanned_once_and_opens_on_the_retry() {
        let o = opener();
        assert!(!o.personalities().is_empty());
        STALE_OPENS.with(|s| s.set(1));
        assert!(o.open_vendor(Personality::DInput).unwrap().is_some());
        assert_eq!(scans(), 2, "the first scan and one forced rescan");
    }

    #[test]
    fn a_stale_gamepad_path_is_rescanned_once_and_opens_on_the_retry() {
        let o = opener();
        assert!(!o.personalities().is_empty());
        STALE_OPENS.with(|s| s.set(1));
        assert!(o.open_gamepad(Personality::DInput).unwrap().is_some());
        assert_eq!(scans(), 2);
    }

    #[test]
    fn the_collections_are_opened_again_after_one_rescan_when_none_opened() {
        let o = opener();
        assert!(!o.personalities().is_empty());
        STALE_OPENS.with(|s| s.set(1));
        assert_eq!(o.open_collections().unwrap().len(), 1);
        assert_eq!(scans(), 2);
    }

    #[test]
    fn a_missing_pad_is_rescanned_at_most_once_per_ttl() {
        let o = opener();
        PLUGGED.with(|s| s.set(false));
        for _ in 0..5 {
            assert!(o.personalities().is_empty());
            assert!(o.open_vendor(Personality::DInput).unwrap().is_none());
            assert!(o.open_gamepad(Personality::DInput).unwrap().is_none());
        }
        assert_eq!(scans(), 2, "the first scan and one forced rescan, not one per call");
    }

    #[test]
    fn a_pad_plugged_in_after_the_last_scan_is_found_inside_the_ttl() {
        let o = opener();
        PLUGGED.with(|s| s.set(false));
        assert!(o.personalities().is_empty());
        PLUGGED.with(|s| s.set(true));
        assert!(!o.personalities().is_empty());
        assert_eq!(scans(), 2);
    }
}
