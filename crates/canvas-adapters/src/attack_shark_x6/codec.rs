//! The Attack Shark X6's configuration reports and its status events, as pure byte work.
//!
//! Ported from `peripheral-hub`'s `devices/attack-shark-x6/src/codec.rs`: the same layouts, the
//! same checksums, the same refusals. Only the guest ABI around it changed. peripheral-hub's
//! `Value` enum and the WASM guest's `SettingChange`/`SettingReading` plumbing have no
//! counterpart here: this module is purely byte-level, and the adapter in `mod.rs` turns its
//! numbers into the spine's properties.
//!
//! Sources, all in `peripheral-hub/docs/research/attack-shark-x6.md`: section 3.2 for the write
//! layouts and checksums (decoded from the vendor app's own packet builders), section 3.4 for the
//! events, section 3.5 for the read-back replies (the live log
//! `docs/research/raw/attack-shark/hid_readgrant_log.txt`). Where the research leaves a byte's
//! meaning UNKNOWN (U5: bytes 3, 4, 6, 7 and 49 of report 04) this module never interprets it: a
//! write copies those bytes from the frame the device last reported.
//!
//! Two layout facts the research log shows and the codecs rely on:
//! - A reply is the write layout with byte 1 raised by two (`declared length + 2`), and it can
//!   carry stale buffer content after the meaningful bytes, so only the first [`Kind::reply_len`]
//!   bytes are read.
//! - Report 0C answers with the request's parameter byte inserted as payload byte 2, which shifts
//!   its two (value, complement) pairs one byte to the right of the 0C write layout.
//!
//! Every function here is pure: bytes in, bytes out. Nothing in this file touches hardware.
//!
//! The codec knows nothing about properties, sections or the editor: it decodes a report into
//! numbers and names, and the adapter in `mod.rs` turns those into the properties the editor
//! renders.

/// Report id of the read request and of the receiver's grant answer.
pub const GRANT_REPORT_ID: u8 = 0xA0;

/// The apply header that starts every write burst (vendor constants `0xfe010a0c`, `0xfe01`;
/// research 3.2).
pub const APPLY_HEADER: [u8; 10] = [0x0C, 0x0A, 0x01, 0xFE, 0x01, 0xFE, 0x00, 0x00, 0x00, 0x00];

/// One of the X6's configuration reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Report `0x04`: the DPI stages, their colours and the active stage.
    Dpi,
    /// Report `0x05`: lighting, sleep and the key response time.
    Light,
    /// Report `0x06`: the polling rate.
    Poll,
    /// Report `0x08`: the button map.
    Buttons,
    /// Report `0x0C`: the profile header.
    Profile,
}

/// The order the adapter reads in: the order of the approved live test (research 3.5), without
/// report `0x0B`.
pub const READ_ORDER: [Kind; 5] = [Kind::Poll, Kind::Dpi, Kind::Light, Kind::Buttons, Kind::Profile];

impl Kind {
    /// The report id this kind travels under.
    pub const fn rid(self) -> u8 {
        match self {
            Kind::Dpi => 0x04,
            Kind::Light => 0x05,
            Kind::Poll => 0x06,
            Kind::Buttons => 0x08,
            Kind::Profile => 0x0C,
        }
    }

    /// The length byte of the read request: what the firmware is prepared to emit (research 3.5).
    pub const fn declared_len(self) -> u8 {
        match self {
            Kind::Dpi => 0x38,
            Kind::Light => 0x0F,
            Kind::Poll => 0x09,
            Kind::Buttons => 0x3B,
            Kind::Profile => 0x0A,
        }
    }

    /// Length of the write buffer, the vendor's `HidD_SetFeature` length (research 3.2). The
    /// profile has no write frame of its own here: the apply header is a constant.
    pub const fn write_len(self) -> usize {
        match self {
            Kind::Dpi => 56,
            Kind::Light => 15,
            Kind::Poll => 9,
            Kind::Buttons => 59,
            Kind::Profile => 10,
        }
    }

    /// How many leading bytes of a reply carry meaning. The log shows replies longer than this
    /// with stale buffer content after it; the checksum sits at the end of these bytes.
    pub const fn reply_len(self) -> usize {
        match self {
            Kind::Dpi => 52,
            Kind::Light => 13,
            Kind::Poll => 5,
            Kind::Buttons => 59,
            Kind::Profile => 7,
        }
    }

    /// The name the read request goes under.
    pub const fn request_id(self) -> &'static str {
        match self {
            Kind::Dpi => "read-04",
            Kind::Light => "read-05",
            Kind::Poll => "read-06",
            Kind::Buttons => "read-08",
            Kind::Profile => "read-0c",
        }
    }

    /// The name the reply goes under.
    pub const fn reply_id(self) -> &'static str {
        match self {
            Kind::Dpi => "config-04",
            Kind::Light => "config-05",
            Kind::Poll => "config-06",
            Kind::Buttons => "config-08",
            Kind::Profile => "config-0c",
        }
    }

    /// The name the write goes under.
    pub const fn write_id(self) -> &'static str {
        match self {
            Kind::Dpi => "write-04",
            Kind::Light => "write-05",
            Kind::Poll => "write-06",
            Kind::Buttons => "write-08",
            Kind::Profile => "write-0c",
        }
    }
}

/// The 8-byte read request for `kind`: `A0 <rid> <declared length> 00 01 00 00 00` (research 3.5).
pub fn read_request(kind: Kind) -> [u8; 8] {
    [GRANT_REPORT_ID, kind.rid(), kind.declared_len(), 0x00, 0x01, 0x00, 0x00, 0x00]
}

fn sum16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |a, b| a.wrapping_add(u16::from(*b)))
}

fn put_be16(frame: &mut [u8], at: usize, v: u16) {
    frame[at] = (v >> 8) as u8;
    frame[at + 1] = (v & 0xFF) as u8;
}

fn be16(frame: &[u8], at: usize) -> u16 {
    (u16::from(frame[at]) << 8) | u16::from(frame[at + 1])
}

/// Recompute the checksum of a write-layout frame in place (`Kind::Poll` and `Kind::Profile` carry
/// complements, not sums).
pub fn seal(kind: Kind, frame: &mut [u8]) {
    match kind {
        Kind::Dpi => {
            let c = sum16(&frame[3..50]);
            put_be16(frame, 50, c);
        }
        Kind::Light => {
            let c = sum16(&frame[3..11]);
            put_be16(frame, 11, c);
        }
        Kind::Buttons => {
            let c = sum16(&frame[3..57]);
            put_be16(frame, 57, c);
        }
        Kind::Poll => frame[4] = !frame[3],
        Kind::Profile => {}
    }
}

/// Judge a reply: long enough, the right report id, a header that echoes this request's declared
/// length plus two (a buffer the receiver has not refilled yet fails here), and a checksum that
/// matches.
pub fn check_reply(kind: Kind, f: &[u8]) -> Result<(), String> {
    let name = kind.reply_id();
    if f.len() < kind.reply_len() {
        return Err(format!("{name} reply is {} bytes, the layout needs {}", f.len(), kind.reply_len()));
    }
    if f[0] != kind.rid() {
        return Err(format!("{name} reply starts with {:#04x}, not {:#04x}", f[0], kind.rid()));
    }
    let want = kind.declared_len() + 2;
    if f[1] != want {
        return Err(format!(
            "{name} reply header is {:#04x}, not {:#04x}: the buffer was not refilled",
            f[1], want
        ));
    }
    let ok = match kind {
        Kind::Dpi => be16(f, 50) == sum16(&f[3..50]),
        Kind::Light => be16(f, 11) == sum16(&f[3..11]),
        Kind::Buttons => be16(f, 57) == sum16(&f[3..57]),
        Kind::Poll => f[4] == !f[3],
        Kind::Profile => f[4] == !f[3] && f[6] == !f[5],
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{name} reply fails its checksum"))
    }
}

/// The write-layout frame for `kind` made from a reply the device gave: meaningful bytes only,
/// zero padded to the write length, header length byte restored. Only Dpi, Light and Buttons are
/// patched; Poll is built whole.
pub fn write_frame_from_reply(kind: Kind, reply: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; kind.write_len()];
    let n = kind.reply_len().min(reply.len()).min(f.len());
    f[..n].copy_from_slice(&reply[..n]);
    f[1] = kind.declared_len();
    f
}

// -------------------------------------------------------------------- report 04: DPI stages

/// One DPI stage: its resolution when enabled, and its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    /// The resolution in DPI, or `None` when the stage is off.
    pub dpi: Option<u32>,
    /// The stage's colour.
    pub rgb: (u8, u8, u8),
}

/// How many DPI stages the mouse has.
pub const STAGE_COUNT: usize = 8;
/// The mouse counts DPI in steps of 50.
pub const DPI_STEP: u32 = 50;
/// The highest DPI the mouse takes.
pub const DPI_MAX: u32 = 26_000;

fn dpi_from_raw(lo: u8, hi: u8) -> u32 {
    (((u32::from(hi) << 8) | u32::from(lo)) + 1) * DPI_STEP
}

/// The stages in a 04 frame (reply or write layout; both put them at the same offsets).
pub fn stages_of(f: &[u8]) -> [Stage; STAGE_COUNT] {
    let mask = f[5];
    let mut out = [Stage { dpi: None, rgb: (0, 0, 0) }; STAGE_COUNT];
    for (i, s) in out.iter_mut().enumerate() {
        s.dpi = (mask & (1 << i) != 0).then(|| dpi_from_raw(f[8 + i], f[16 + i]));
        s.rgb = (f[25 + 3 * i], f[26 + 3 * i], f[27 + 3 * i]);
    }
    out
}

/// The resolution a stage's bytes hold, whether the stage is on or off.
///
/// A stage that is off holds zero in the device's bytes (the vendor app leaves a stage it skips
/// at zero), which this decodes as the range minimum. [`stages_of`] reports `None` for such a
/// stage; this reports the number the bytes hold.
pub fn stage_value(f: &[u8], index: usize) -> u32 {
    dpi_from_raw(f[8 + index], f[16 + index])
}

/// `800@#FF0000,1200@#00FF00,...,off@#FFFFFF`: eight entries, a resolution or `off`, `@`, the
/// stage colour. This is the value of the `dpi-stages` setting.
pub fn stages_to_text(stages: &[Stage; STAGE_COUNT]) -> String {
    stages
        .iter()
        .map(|s| {
            let dpi = s.dpi.map_or_else(|| "off".to_string(), |d| d.to_string());
            format!("{dpi}@#{:02X}{:02X}{:02X}", s.rgb.0, s.rgb.1, s.rgb.2)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The other end of [`stages_to_text`]. Refuses a list the mouse would not take.
pub fn parse_stages(text: &str) -> Result<[Stage; STAGE_COUNT], String> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() != STAGE_COUNT {
        return Err(format!("DPI stages need {STAGE_COUNT} entries, got {}", parts.len()));
    }
    let mut out = [Stage { dpi: None, rgb: (0, 0, 0) }; STAGE_COUNT];
    for (i, part) in parts.iter().enumerate() {
        let (dpi, colour) =
            part.split_once('@').ok_or_else(|| format!("stage {} has no colour: {part:?}", i + 1))?;
        out[i].dpi = if dpi.trim() == "off" {
            None
        } else {
            let d: u32 = dpi
                .trim()
                .parse()
                .map_err(|_| format!("stage {} has no resolution: {part:?}", i + 1))?;
            if !(DPI_STEP..=DPI_MAX).contains(&d) || !d.is_multiple_of(DPI_STEP) {
                return Err(format!(
                    "stage {} is {d} DPI; the mouse takes {DPI_STEP} to {DPI_MAX} in steps of {DPI_STEP}",
                    i + 1
                ));
            }
            Some(d)
        };
        out[i].rgb = parse_hex_colour(colour.trim())
            .ok_or_else(|| format!("stage {} colour is not #RRGGBB: {colour:?}", i + 1))?;
    }
    Ok(out)
}

fn parse_hex_colour(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = |r: std::ops::Range<usize>| u8::from_str_radix(&h[r], 16).ok();
    Some((v(0..2)?, v(2..4)?, v(4..6)?))
}

/// Active stage, 1-based, when it is 1..=8.
pub fn active_stage_of(f: &[u8]) -> Option<u8> {
    (1..=8).contains(&f[24]).then_some(f[24])
}

/// Store `stages` into a write-layout 04 frame. Bytes the research leaves UNKNOWN (3, 4, 6, 7, 49)
/// and the active stage stay as the device reported them. The active stage must stay enabled, and
/// one stage must stay on.
pub fn patch_stages(frame: &mut [u8], stages: &[Stage; STAGE_COUNT]) -> Result<(), String> {
    if stages.iter().all(|s| s.dpi.is_none()) {
        return Err("at least one DPI stage must stay on".into());
    }
    if let Some(active) = active_stage_of(frame) {
        if stages[usize::from(active) - 1].dpi.is_none() {
            return Err(format!(
                "stage {active} is the active stage; make another stage active with the DPI button before turning it off"
            ));
        }
    }
    let old = stages_of(frame);
    let mut mask = 0u8;
    for (i, s) in stages.iter().enumerate() {
        match s.dpi {
            Some(d) => {
                mask |= 1 << i;
                let raw = (d / DPI_STEP - 1) as u16;
                frame[8 + i] = (raw & 0xFF) as u8;
                frame[16 + i] = (raw >> 8) as u8;
            }
            None if old[i].dpi.is_some() => {
                // The vendor app leaves a stage it skips at zero.
                frame[8 + i] = 0;
                frame[16 + i] = 0;
            }
            None => {}
        }
        frame[25 + 3 * i] = s.rgb.0;
        frame[26 + 3 * i] = s.rgb.1;
        frame[27 + 3 * i] = s.rgb.2;
    }
    frame[5] = mask;
    seal(Kind::Dpi, frame);
    Ok(())
}

/// The mouse reported another active stage (its DPI button): keep the cached frame current so a
/// later write does not put the old stage back.
pub fn set_active_stage(frame: &mut [u8], stage: u8) {
    frame[24] = stage;
    seal(Kind::Dpi, frame);
}

// ------------------------------------------------------- report 05: lighting, sleep, debounce

/// The LED modes this build names. The vendor app offers twelve (research 3.2); the modes past
/// index 6 are UNKNOWN on the X6 firmware (research U6), so they are not named here.
pub const LED_MODES: [&str; 7] =
    ["off", "static", "breathing", "neon", "color-breathing", "static-dpi", "breathing-dpi"];

/// Mode indexes whose write carries the chosen brightness; every other mode sends the constant 8
/// (research 3.2).
///
/// Index 1 is the static mode, the only named mode that takes a brightness; index 9 is the mode
/// the research names but the vendor app does not.
pub fn mode_takes_brightness(mode: u8) -> bool {
    mode == 1 || mode == 9
}

/// What a 05 frame holds, each field `None` when its bytes are outside the range the vendor app
/// produces.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    /// The LED mode, when the mode index is one this build names.
    pub mode: Option<&'static str>,
    /// The LED colour.
    pub rgb: (u8, u8, u8),
    /// The brightness, 1..=8.
    pub brightness: Option<u8>,
    /// The vendor's slider value, 4..=8; the wire carries `9 - slider`.
    pub speed_slider: Option<u8>,
    /// Half minutes, 1..=60.
    pub sleep_half_min: Option<u8>,
    /// Minutes, 1..=60.
    pub deep_sleep_min: Option<u8>,
    /// Milliseconds, 4..=50 and even.
    pub key_response_ms: Option<u8>,
}

/// The fields of a 05 frame.
pub fn light_of(f: &[u8]) -> Light {
    let wire_speed = f[4] & 0x0F;
    let deep = (f[4] & 0xF0) | (f[5] >> 4);
    Light {
        mode: LED_MODES.get(usize::from(f[3] >> 4)).copied(),
        rgb: (f[6], f[7], f[8]),
        brightness: (1..=8).contains(&(f[5] & 0x0F)).then_some(f[5] & 0x0F),
        speed_slider: (1..=5).contains(&wire_speed).then(|| 9 - wire_speed),
        sleep_half_min: (1..=60).contains(&f[9]).then_some(f[9]),
        deep_sleep_min: (1..=60).contains(&deep).then_some(deep),
        key_response_ms: (2..=25).contains(&f[10]).then(|| f[10] * 2),
    }
}

/// The fields a write changes. `None` keeps what the device holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightChange {
    /// The LED mode index, into [`LED_MODES`].
    pub mode: Option<u8>,
    /// The LED colour.
    pub rgb: Option<(u8, u8, u8)>,
    /// The brightness, 1..=8.
    pub brightness: Option<u8>,
    /// The speed slider, 4..=8.
    pub speed_slider: Option<u8>,
    /// The sleep time in half minutes, 1..=60.
    pub sleep_half_min: Option<u8>,
    /// The deep sleep time in minutes, 1..=60.
    pub deep_sleep_min: Option<u8>,
    /// The key response time in milliseconds, 4..=50 and even.
    pub key_response_ms: Option<u8>,
}

/// Store `change` into a write-layout 05 frame.
pub fn patch_light(frame: &mut [u8], change: &LightChange) -> Result<(), String> {
    let mut mode = frame[3] >> 4;
    let mut wire_speed = frame[4] & 0x0F;
    let mut deep = (frame[4] & 0xF0) | (frame[5] >> 4);
    let mut bright = frame[5] & 0x0F;
    if let Some(m) = change.mode {
        mode = m;
        frame[3] = (frame[3] & 0x0F) | (m << 4);
    }
    if let Some(s) = change.speed_slider {
        if !(4..=8).contains(&s) {
            return Err(format!("LED speed is {s}; the mouse takes 4 to 8"));
        }
        wire_speed = 9 - s;
    }
    if let Some(d) = change.deep_sleep_min {
        if !(1..=60).contains(&d) {
            return Err(format!("deep sleep is {d} min; the mouse takes 1 to 60"));
        }
        deep = d;
    }
    if let Some(b) = change.brightness {
        if !(1..=8).contains(&b) {
            return Err(format!("LED brightness is {b}; the mouse takes 1 to 8"));
        }
        bright = b;
    }
    // The vendor app sends the chosen brightness only in the modes that use it and the constant 8
    // otherwise.
    if (change.mode.is_some() || change.brightness.is_some()) && !mode_takes_brightness(mode) {
        bright = 8;
    }
    frame[4] = (deep & 0xF0) | (wire_speed & 0x0F);
    frame[5] = ((deep & 0x0F) << 4) | (bright & 0x0F);
    if let Some((r, g, b)) = change.rgb {
        frame[6] = r;
        frame[7] = g;
        frame[8] = b;
    }
    if let Some(s) = change.sleep_half_min {
        if !(1..=60).contains(&s) {
            return Err(format!(
                "sleep time is {} min; the mouse takes 0.5 to 30",
                f64::from(s) / 2.0
            ));
        }
        frame[9] = s;
    }
    if let Some(ms) = change.key_response_ms {
        if !(4..=50).contains(&ms) || !ms.is_multiple_of(2) {
            return Err(format!(
                "key response time is {ms} ms; the mouse takes 4 to 50 in steps of 2"
            ));
        }
        frame[10] = ms / 2;
    }
    seal(Kind::Light, frame);
    Ok(())
}

// ---------------------------------------------------------------------- report 06: polling

/// The polling rates the mouse takes, with the byte each one travels under.
///
/// The ids are the rates in Hz: they are the property keys the editor shows.
pub const POLLING: [(&str, u8); 4] = [("125", 8), ("250", 4), ("500", 2), ("1000", 1)];

/// The polling rate a 06 frame holds.
pub fn polling_of(f: &[u8]) -> Option<&'static str> {
    POLLING.iter().find(|(_, code)| *code == f[3]).map(|(id, _)| *id)
}

/// The whole 06 frame for a polling rate.
pub fn polling_frame(id: &str) -> Result<Vec<u8>, String> {
    let code = POLLING
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, c)| *c)
        .ok_or_else(|| format!("no polling rate {id:?}"))?;
    let mut f = vec![0u8; Kind::Poll.write_len()];
    f[0] = 0x06;
    f[1] = Kind::Poll.declared_len();
    f[2] = 0x01;
    f[3] = code;
    seal(Kind::Poll, &mut f);
    Ok(f)
}

// --------------------------------------------------------------------- report 08: buttons

/// Plain button actions: option id and the action byte (research 3.2, the table at `X6.exe` VA
/// `0x41d498`). The other action kinds (keyboard key, shortcut, macro) carry parameters and are
/// shown as `other`, never written.
pub const ACTIONS: [(&str, u8); 14] = [
    ("off", 0x01),
    ("left-click", 0x02),
    ("right-click", 0x03),
    ("middle-click", 0x04),
    ("back", 0x05),
    ("forward", 0x06),
    ("double-click", 0x07),
    ("fire", 0x08),
    ("scroll-up", 0x09),
    ("scroll-down", 0x0A),
    ("dpi-cycle", 0x0D),
    ("dpi-up", 0x0E),
    ("dpi-down", 0x0F),
    ("led-loop", 0x3C),
];

/// The name an action with parameters, or an unknown code, is shown under.
pub const OTHER_ACTION: &str = "other";
/// The action byte of a plain left click.
pub const LEFT_CLICK: u8 = 0x02;
/// How many button slots the firmware holds.
pub const SLOTS: usize = 18;

/// Which of the 18 slots each button row stands for. The six rows follow the factory map the
/// device reported (research 3.2: slot 0 left, 1 right, 2 middle, 3 DPI cycle, 6 forward, 7
/// backward); that these slots are the physical buttons is UNKNOWN (research U12).
///
/// The names are the vendor's own slot numbering, which skips the slots the vendor app does not
/// name: those are listed as unreported by the adapter.
pub const BUTTON_SLOTS: [(&str, usize); 6] = [
    ("button-1", 0),
    ("button-2", 1),
    ("button-3", 2),
    ("button-4", 3),
    ("button-7", 6),
    ("button-8", 7),
];

/// The slot a button setting stands for.
pub fn slot_of(setting: &str) -> Option<usize> {
    BUTTON_SLOTS.iter().find(|(s, _)| *s == setting).map(|(_, k)| *k)
}

fn slot_bytes(f: &[u8], k: usize) -> [u8; 3] {
    [f[3 + 3 * k], f[4 + 3 * k], f[5 + 3 * k]]
}

/// The action a slot holds.
pub fn action_of(f: &[u8], k: usize) -> &'static str {
    let [code, p1, p2] = slot_bytes(f, k);
    if p1 != 0 || p2 != 0 {
        return OTHER_ACTION;
    }
    ACTIONS
        .iter()
        .find(|(_, c)| *c == code)
        .map_or(OTHER_ACTION, |(id, _)| *id)
}

/// The action byte an action id travels under.
pub fn action_code(id: &str) -> Option<u8> {
    ACTIONS.iter().find(|(i, _)| *i == id).map(|(_, c)| *c)
}

/// Store one plain action in one slot of a write-layout 08 frame.
pub fn patch_button(frame: &mut [u8], slot: usize, action: &str) -> Result<(), String> {
    let code =
        action_code(action).ok_or_else(|| format!("{action:?} is not an action this app writes"))?;
    frame[3 + 3 * slot] = code;
    frame[4 + 3 * slot] = 0;
    frame[5 + 3 * slot] = 0;
    Ok(())
}

/// Whether the map still holds a left click. The vendor app refuses a map without one
/// (`one_click_text`); so does this codec.
pub fn has_left_click(f: &[u8]) -> bool {
    (0..SLOTS).any(|k| slot_bytes(f, k) == [LEFT_CLICK, 0, 0])
}

// -------------------------------------------------------------- status events (input report 03)

/// One status event from the status collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Battery event. `charging` is state 3; state 2 is "fully charged" (vendor app, never seen
    /// live). `level_byte` is set only when it exceeds 10, which would prove the level is a
    /// percent (research U3).
    Battery {
        /// The mouse is charging.
        charging: bool,
        /// The mouse is fully charged.
        fully_charged: bool,
        /// The raw level byte, when it is above 10.
        level_byte: Option<u8>,
    },
    /// The DPI button moved the mouse to this stage (1..=8). Decoded from the vendor app; not yet
    /// seen live.
    DpiStage(u8),
    /// Well formed, from the vendor channel, nothing this adapter shows.
    Ignored,
}

/// Decode one report from the status collection. `Err` is a malformed report (wrong size, wrong
/// report id, or a field outside the range the layout allows); a well-formed report about
/// something else is [`Event::Ignored`].
pub fn decode_event(d: &[u8]) -> Result<Event, String> {
    if d.len() != 5 {
        return Err(format!("event report is {} bytes, not 5", d.len()));
    }
    if d[0] != 0x03 {
        return Err(format!("event report starts with {:#04x}, not 0x03", d[0]));
    }
    // The vendor app drops every report whose second byte is not 0x10 (research 3.4).
    if d[1] != 0x10 {
        return Ok(Event::Ignored);
    }
    match d[2] {
        0x40 => match d[3] {
            1 => Ok(Event::Battery {
                charging: false,
                fully_charged: false,
                level_byte: (d[4] > 10).then_some(d[4]),
            }),
            2 => Ok(Event::Battery {
                charging: false,
                fully_charged: true,
                level_byte: (d[4] > 10).then_some(d[4]),
            }),
            3 => Ok(Event::Battery {
                charging: true,
                fully_charged: false,
                level_byte: (d[4] > 10).then_some(d[4]),
            }),
            other => Err(format!("battery event has state {other}; the layout allows 1 to 3")),
        },
        0x10 => {
            if (1..=8).contains(&d[3]) {
                Ok(Event::DpiStage(d[3]))
            } else {
                Err(format!("DPI event names stage {}; the mouse has 1 to 8", d[3]))
            }
        }
        _ => Ok(Event::Ignored),
    }
}

/// The bytes the live read of 2026-10-17 returned, and the frames the research encoder printed
/// for `pro.data` slot 5.
///
/// This is the only copy of the device's answers in this repository: both this module's tests and
/// the adapter's tests are checked against these, so no test holds a second reading of the
/// mouse. The sources are
/// `peripheral-hub/docs/research/raw/attack-shark/hid_readgrant_log.txt` (the live read) and
/// `.../encoded_from_prodata_slot5.txt` (the frames the research encoder printed).
pub mod evidence {
    /// The reply to the read request for report `0x04` (52 meaningful bytes).
    pub const REPLY_04: &[u8] = &[
        0x04, 0x3a, 0x01, 0x00, 0x00, 0x3f, 0x00, 0x01, 0x0f, 0x17, 0x1f, 0x3f, 0x6f, 0x07, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x02, 0xff, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0xff, 0xff, 0x40, 0x00, 0xff, 0xff,
        0xff, 0x01, 0x0e, 0x72,
    ];
    /// The reply to the read request for report `0x05` (13 meaningful bytes, then stale bytes).
    pub const REPLY_05: &[u8] = &[
        0x05, 0x11, 0x01, 0x10, 0x03, 0xa8, 0xef, 0x3f, 0xff, 0x01, 0x04, 0x02, 0xed, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x02, 0xff, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0xff, 0xff, 0x40, 0x00, 0xff, 0xff,
        0xff, 0x01, 0x0e, 0x72,
    ];
    /// The reply to the read request for report `0x06` (5 meaningful bytes).
    pub const REPLY_06: &[u8] = &[0x06, 0x0b, 0x01, 0x01, 0xfe];
    /// The reply to the read request for report `0x08` (59 meaningful bytes).
    pub const REPLY_08: &[u8] = &[
        0x08, 0x3d, 0x01, 0x02, 0x00, 0x00, 0x03, 0x00, 0x00, 0x04, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x0e,
        0x00, 0x00, 0x0f, 0x00, 0x00, 0x06, 0x00, 0x00, 0x05, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x01, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x94,
    ];
    /// The reply to the read request for report `0x0C` (7 meaningful bytes, then stale bytes).
    pub const REPLY_0C: &[u8] = &[
        0x0c, 0x0c, 0x01, 0x01, 0xfe, 0x01, 0xfe, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x0e,
        0x00, 0x00, 0x0f, 0x00, 0x00, 0x06, 0x00, 0x00, 0x05, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x01, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x94,
    ];
    /// The reply to the read request for report `0x0B`: version and identity, meaning UNKNOWN.
    pub const REPLY_0B: &[u8] = &[0x0b, 0x0a, 0x01, 0x10, 0x10, 0x83, 0x01, 0x01];

    /// The frame the research encoder printed for `pro.data` slot 5, report `0x04` (56 bytes).
    pub const ENCODED_04: &[u8] = &[
        0x04, 0x38, 0x01, 0x00, 0x00, 0x3f, 0x00, 0x01, 0x0f, 0x17, 0x1f, 0x3f, 0x6f, 0x07, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x01, 0xff, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0xff, 0xff, 0x40, 0x00, 0xff, 0xff,
        0xff, 0x01, 0x0e, 0x71, 0x00, 0x00, 0x00, 0x00,
    ];
    /// The frame the research encoder printed for `pro.data` slot 5, report `0x05` (15 bytes).
    pub const ENCODED_05: &[u8] =
        &[0x05, 0x0f, 0x01, 0x10, 0x03, 0xa8, 0xef, 0x3f, 0xff, 0x01, 0x04, 0x02, 0xed, 0x00, 0x00];
    /// The frame the research encoder printed for `pro.data` slot 5, report `0x06` (9 bytes).
    pub const ENCODED_06: &[u8] = &[0x06, 0x09, 0x01, 0x01, 0xfe, 0x00, 0x00, 0x00, 0x00];
    /// The frame the research encoder printed for `pro.data` slot 5, report `0x08` (59 bytes).
    pub const ENCODED_08: &[u8] = &[
        0x08, 0x3b, 0x01, 0x02, 0x00, 0x00, 0x03, 0x00, 0x00, 0x04, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x0e,
        0x00, 0x00, 0x0f, 0x00, 0x00, 0x06, 0x00, 0x00, 0x05, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x01, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x94,
    ];
    /// The frame the research encoder printed for `pro.data` slot 5, report `0x0C` (10 bytes).
    pub const ENCODED_0C: &[u8] = &[0x0c, 0x0a, 0x01, 0xfe, 0x01, 0xfe, 0x00, 0x00, 0x00, 0x00];
}

#[cfg(test)]
mod tests {
    use super::evidence::*;
    use super::*;

    // ---------------------------------------------------------------- the research evidence
    //
    // The bytes are in [`evidence`]: the live read of 2026-10-17 and the frames the research
    // encoder printed for `pro.data` slot 5, copied out of
    // `peripheral-hub/docs/research/raw/attack-shark/hid_readgrant_log.txt` and
    // `.../encoded_from_prodata_slot5.txt`. They are the only copy in this repository: the tests
    // hold no second reading of the device.

    fn reply(kind: Kind) -> Vec<u8> {
        match kind {
            Kind::Dpi => REPLY_04.to_vec(),
            Kind::Light => REPLY_05.to_vec(),
            Kind::Poll => REPLY_06.to_vec(),
            Kind::Buttons => REPLY_08.to_vec(),
            Kind::Profile => REPLY_0C.to_vec(),
        }
    }

    /// The write-layout form of a cached frame, made into the reply form the device would give
    /// (header raised by two).
    fn as_reply(kind: Kind, mut f: Vec<u8>) -> Vec<u8> {
        f.truncate(kind.reply_len());
        f[1] += 2;
        f
    }

    fn hex(s: &str) -> Vec<u8> {
        let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        assert!(clean.len().is_multiple_of(2), "odd number of hex digits in {s:?}");
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    // -------------------------------------------------------- the live read decodes to the research values

    #[test]
    fn every_logged_reply_passes_its_checks() {
        for kind in READ_ORDER {
            check_reply(kind, &reply(kind)).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        }
    }

    #[test]
    fn the_live_read_holds_exactly_the_research_values() {
        let dpi = reply(Kind::Dpi);
        // DPI 800/1200/1600/3200/5600/26000, the six colours the vendor app wrote, stages 7 and 8
        // off (mask 0x3f).
        assert_eq!(
            stages_of(&dpi).map(|s| s.dpi),
            [Some(800), Some(1200), Some(1600), Some(3200), Some(5600), Some(26000), None, None]
        );
        assert_eq!(
            stages_to_text(&stages_of(&dpi)),
            "800@#FF0000,1200@#00FF00,1600@#0000FF,3200@#FFFF00,5600@#00FFFF,26000@#FF00FF,off@#FF4000,off@#FFFFFF"
        );
        assert_eq!(
            active_stage_of(&dpi),
            Some(2),
            "the DPI button moved the mouse to stage 2 after the last Apply (research 3.5)"
        );

        let light = reply(Kind::Light);
        let l = light_of(&light);
        assert_eq!(l.rgb, (0xEF, 0x3F, 0xFF), "LED #EF3FFF");
        assert_eq!(l.mode, Some("static"));
        assert_eq!(l.brightness, Some(8));
        assert_eq!(l.speed_slider, Some(6), "vendor slider 6 = wire 3");
        assert_eq!(l.key_response_ms, Some(8), "8 ms");
        assert_eq!(l.sleep_half_min, Some(1), "0.5 min");
        assert_eq!(l.deep_sleep_min, Some(10));

        assert_eq!(polling_of(&reply(Kind::Poll)), Some("1000"), "1000 Hz");

        let buttons = reply(Kind::Buttons);
        for (setting, action) in [
            ("button-1", "left-click"),
            ("button-2", "right-click"),
            ("button-3", "middle-click"),
            ("button-4", "dpi-cycle"),
            ("button-7", "forward"),
            ("button-8", "back"),
        ] {
            let slot = slot_of(setting).unwrap_or_else(|| panic!("{setting} is not a button row"));
            assert_eq!(action_of(&buttons, slot), action, "{setting}");
        }
        assert_eq!(reply(Kind::Profile)[3], 1, "profile 1 of 1");
    }

    #[test]
    fn every_report_the_adapter_reads_decodes_to_something_from_the_live_bytes() {
        // The adapter turns each report into properties; the codec only has to hand it the
        // numbers. Every report must yield at least one.
        assert!(stages_of(&reply(Kind::Dpi)).iter().any(|s| s.dpi.is_some()));
        assert!(light_of(&reply(Kind::Light)).mode.is_some());
        assert!(polling_of(&reply(Kind::Poll)).is_some());
        for ((setting, slot), action) in
            BUTTON_SLOTS.iter().zip(["left-click", "right-click", "middle-click", "dpi-cycle", "forward", "back"])
        {
            assert_eq!(action_of(&reply(Kind::Buttons), *slot), action, "{setting}");
        }
        assert!(active_stage_of(&reply(Kind::Dpi)).is_some());
    }

    #[test]
    fn trailing_stale_bytes_in_a_reply_are_ignored() {
        // The log shows the 05 and 0C replies followed by the previous report's bytes.
        let mut f = reply(Kind::Light);
        assert!(f.len() > Kind::Light.reply_len());
        let clean = light_of(&f);
        for b in f.iter_mut().skip(Kind::Light.reply_len()) {
            *b = 0xA5;
        }
        assert_eq!(light_of(&f), clean);
        check_reply(Kind::Light, &f).unwrap();
    }

    // ------------------------------------------------ the vendor profile re-encodes to the recorded bytes

    /// `pro.data` slot 5 as parsed in `raw/pro_data_parse_slot5.txt`.
    mod slot5 {
        pub const DPI_OVER_50: [i32; 8] = [16, 24, 32, 64, 112, 520, 0, 0];
        pub const ENABLED: [bool; 8] = [true, true, true, true, true, true, false, false];
        pub const COLOURS: [(u8, u8, u8); 8] = [
            (0xFF, 0, 0),
            (0, 0xFF, 0),
            (0, 0, 0xFF),
            (0xFF, 0xFF, 0),
            (0, 0xFF, 0xFF),
            (0xFF, 0, 0xFF),
            (0xFF, 0x40, 0),
            (0xFF, 0xFF, 0xFF),
        ];
        pub const ACTIVE_ZERO_BASED: u8 = 0;
        pub const ANGLE_SNAP: u8 = 0;
        pub const RIPPLE: u8 = 0;
        pub const MOTION_SYNC: u8 = 1;
        /// Function type per key slot (byte 1 of each 0x4f8 record) and the action byte the vendor
        /// table (VA 0x41d498) gives it.
        pub const SLOT_TYPES: [u8; 18] =
            [1, 2, 3, 6, 7, 8, 4, 5, 0x34, 0, 0, 0, 0, 0, 0, 0, 0x0c, 0x0b];

        pub fn action_of_type(t: u8) -> &'static str {
            match t {
                0 => "off",
                1 => "left-click",
                2 => "right-click",
                3 => "middle-click",
                4 => "forward",
                5 => "back",
                6 => "dpi-cycle",
                7 => "dpi-up",
                8 => "dpi-down",
                0x0b => "scroll-up",
                0x0c => "scroll-down",
                0x34 => "led-loop",
                other => panic!("type {other:#x} is not in this profile"),
            }
        }
    }

    /// Build the four config frames from the slot-5 values with this module's patch functions on
    /// blank frames: nothing is copied from the device or from the research output.
    fn frames_from_slot5() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut dpi = vec![0u8; Kind::Dpi.write_len()];
        dpi[..3].copy_from_slice(&[0x04, 0x38, 0x01]);
        dpi[3] = slot5::ANGLE_SNAP;
        dpi[4] = slot5::RIPPLE;
        dpi[6] = slot5::ANGLE_SNAP;
        dpi[7] = slot5::MOTION_SYNC;
        dpi[49] = 0x01;
        dpi[24] = slot5::ACTIVE_ZERO_BASED + 1;
        let mut stages = [Stage { dpi: None, rgb: (0, 0, 0) }; STAGE_COUNT];
        for (i, stage) in stages.iter_mut().enumerate() {
            *stage = Stage {
                dpi: slot5::ENABLED[i].then(|| slot5::DPI_OVER_50[i] as u32 * 50),
                rgb: slot5::COLOURS[i],
            };
        }
        patch_stages(&mut dpi, &stages).unwrap();

        let mut light = vec![0u8; Kind::Light.write_len()];
        light[..3].copy_from_slice(&[0x05, 0x0F, 0x01]);
        patch_light(
            &mut light,
            &LightChange {
                mode: Some(1),
                rgb: Some((0xEF, 0x3F, 0xFF)),
                brightness: Some(8),
                speed_slider: Some(6),
                sleep_half_min: Some(1),
                deep_sleep_min: Some(10),
                key_response_ms: Some(8),
            },
        )
        .unwrap();

        let poll = polling_frame("1000").unwrap();

        let mut buttons = vec![0u8; Kind::Buttons.write_len()];
        buttons[..3].copy_from_slice(&[0x08, 0x3B, 0x01]);
        for (k, t) in slot5::SLOT_TYPES.iter().enumerate() {
            patch_button(&mut buttons, k, slot5::action_of_type(*t)).unwrap();
        }
        seal(Kind::Buttons, &mut buttons);
        (dpi, light, poll, buttons)
    }

    #[test]
    fn the_slot5_profile_re_encodes_to_the_research_encoders_frames_byte_for_byte() {
        let (dpi, light, poll, buttons) = frames_from_slot5();
        assert_eq!(dpi, ENCODED_04, "report 04");
        assert_eq!(light, ENCODED_05, "report 05");
        assert_eq!(poll, ENCODED_06, "report 06");
        assert_eq!(buttons, ENCODED_08, "report 08");
        assert_eq!(APPLY_HEADER.to_vec(), ENCODED_0C, "apply header");
    }

    #[test]
    fn the_re_encoded_profile_differs_from_the_device_only_in_the_active_stage_and_its_checksum() {
        let (dpi, light, poll, buttons) = frames_from_slot5();
        let mut differences = Vec::new();
        for (kind, frame) in
            [(Kind::Dpi, dpi), (Kind::Light, light), (Kind::Poll, poll), (Kind::Buttons, buttons)]
        {
            // The reply header carries the declared length plus two (research 3.5); everything else
            // is compared as is.
            let ours = as_reply(kind, frame);
            let theirs = reply(kind);
            for i in 0..kind.reply_len() {
                if ours[i] != theirs[i] {
                    differences.push((kind.rid(), i, ours[i], theirs[i]));
                }
            }
        }
        assert_eq!(
            differences,
            vec![(0x04, 24, 0x01, 0x02), (0x04, 51, 0x71, 0x72)],
            "report 04: active stage 1 against 2 and the checksum low byte that follows from it; nothing else in 04, 05, 06 or 08"
        );
    }

    // ------------------------------------------------------------- round trips: encode, then decode

    fn cached(kind: Kind) -> Vec<u8> {
        write_frame_from_reply(kind, &reply(kind))
    }

    #[test]
    fn the_cached_write_frame_is_the_device_frame_with_the_write_header() {
        for kind in [Kind::Dpi, Kind::Light, Kind::Buttons] {
            let c = cached(kind);
            assert_eq!(c.len(), kind.write_len());
            assert_eq!(c[1], kind.declared_len());
            assert_eq!(
                as_reply(kind, c),
                reply(kind)[..kind.reply_len()].to_vec(),
                "{kind:?}: header apart, nothing changed"
            );
        }
    }

    #[test]
    fn every_dpi_value_the_mouse_takes_round_trips_on_every_stage() {
        let mut count = 0;
        for stage in 0..STAGE_COUNT {
            for dpi in (50..=26_000).step_by(50) {
                let mut f = cached(Kind::Dpi);
                let mut stages = stages_of(&f);
                stages[stage].dpi = Some(dpi);
                patch_stages(&mut f, &stages).unwrap();
                let r = as_reply(Kind::Dpi, f);
                check_reply(Kind::Dpi, &r).unwrap();
                assert_eq!(stages_of(&r)[stage].dpi, Some(dpi));
                count += 1;
            }
        }
        assert_eq!(count, 8 * 520);
    }

    #[test]
    fn stage_colours_and_stage_switches_round_trip_and_the_unknown_bytes_survive() {
        let original = cached(Kind::Dpi);
        for rgb in [(0, 0, 0), (255, 255, 255), (1, 2, 3), (0x80, 0x7F, 0xFE)] {
            for stage in 0..STAGE_COUNT {
                let mut f = original.clone();
                let mut stages = stages_of(&f);
                stages[stage].rgb = rgb;
                patch_stages(&mut f, &stages).unwrap();
                assert_eq!(stages_of(&as_reply(Kind::Dpi, f.clone()))[stage].rgb, rgb);
                for keep in [3, 4, 6, 7, 24, 49] {
                    assert_eq!(
                        f[keep], original[keep],
                        "byte {keep} (UNKNOWN meaning or runtime state) is copied, not rewritten"
                    );
                }
            }
        }
        // Turning a stage on and off.
        let mut f = original.clone();
        let mut stages = stages_of(&f);
        stages[6].dpi = Some(1000);
        patch_stages(&mut f, &stages).unwrap();
        assert_eq!(f[5], 0x7F);
        assert_eq!(stages_of(&as_reply(Kind::Dpi, f.clone()))[6].dpi, Some(1000));
        stages[6].dpi = None;
        patch_stages(&mut f, &stages).unwrap();
        assert_eq!(f[5], 0x3F);
        assert_eq!((f[14], f[22]), (0, 0), "a stage turned off is left at zero, as the vendor app leaves it");
    }

    #[test]
    fn only_the_bytes_of_the_changed_field_and_the_checksum_change() {
        let original = cached(Kind::Light);
        // The two checksum bytes (11 and 12) move with the sum; every other differing byte must be
        // the field's own.
        let diff = |f: &[u8]| -> Vec<usize> {
            (0..f.len()).filter(|i| f[*i] != original[*i] && !(11..=12).contains(i)).collect()
        };
        let mut f = original.clone();
        patch_light(&mut f, &LightChange { rgb: Some((1, 2, 3)), ..Default::default() }).unwrap();
        assert_eq!(diff(&f), [6, 7, 8]);
        let mut f = original.clone();
        patch_light(&mut f, &LightChange { key_response_ms: Some(20), ..Default::default() }).unwrap();
        assert_eq!(diff(&f), [10]);
        let mut f = original.clone();
        patch_light(&mut f, &LightChange { sleep_half_min: Some(60), ..Default::default() }).unwrap();
        assert_eq!(diff(&f), [9]);
        // Nothing changed, nothing differs.
        let mut f = original.clone();
        patch_light(&mut f, &LightChange::default()).unwrap();
        assert!(diff(&f).is_empty());
    }

    #[test]
    fn every_lighting_field_round_trips_over_its_whole_range() {
        for (index, id) in LED_MODES.iter().enumerate() {
            let mut f = cached(Kind::Light);
            patch_light(&mut f, &LightChange { mode: Some(index as u8), ..Default::default() }).unwrap();
            let r = as_reply(Kind::Light, f);
            check_reply(Kind::Light, &r).unwrap();
            assert_eq!(light_of(&r).mode, Some(*id));
        }
        for speed in 4..=8 {
            let mut f = cached(Kind::Light);
            patch_light(&mut f, &LightChange { speed_slider: Some(speed), ..Default::default() }).unwrap();
            assert_eq!(light_of(&as_reply(Kind::Light, f)).speed_slider, Some(speed));
        }
        for half in 1..=60 {
            let mut f = cached(Kind::Light);
            patch_light(&mut f, &LightChange { sleep_half_min: Some(half), ..Default::default() }).unwrap();
            assert_eq!(light_of(&as_reply(Kind::Light, f)).sleep_half_min, Some(half));
        }
        for deep in 1..=60 {
            let mut f = cached(Kind::Light);
            patch_light(&mut f, &LightChange { deep_sleep_min: Some(deep), ..Default::default() }).unwrap();
            let l = light_of(&as_reply(Kind::Light, f));
            assert_eq!(l.deep_sleep_min, Some(deep));
            assert_eq!(l.speed_slider, Some(6), "the speed that shares byte 4 and the brightness that shares byte 5 survive");
            assert_eq!(l.brightness, Some(8));
        }
        for ms in (4..=50).step_by(2) {
            let mut f = cached(Kind::Light);
            patch_light(&mut f, &LightChange { key_response_ms: Some(ms), ..Default::default() }).unwrap();
            assert_eq!(light_of(&as_reply(Kind::Light, f)).key_response_ms, Some(ms));
        }
        for rgb in [(0, 0, 0), (255, 255, 255), (0xEF, 0x3F, 0xFF), (1, 128, 254)] {
            let mut f = cached(Kind::Light);
            patch_light(&mut f, &LightChange { rgb: Some(rgb), ..Default::default() }).unwrap();
            assert_eq!(light_of(&as_reply(Kind::Light, f)).rgb, rgb);
        }
    }

    #[test]
    fn brightness_is_stored_in_static_mode_and_the_constant_8_goes_out_in_every_other_mode() {
        // The vendor app's rule (research 3.2, byte 5): the chosen brightness only for mode index
        // 1 (and 9).
        for brightness in 1..=8 {
            let mut f = cached(Kind::Light);
            patch_light(&mut f, &LightChange { mode: Some(1), brightness: Some(brightness), ..Default::default() })
                .unwrap();
            assert_eq!(
                light_of(&as_reply(Kind::Light, f)).brightness,
                Some(brightness),
                "static keeps {brightness}"
            );
            for mode in [0, 2, 3, 4, 5, 6] {
                let mut f = cached(Kind::Light);
                patch_light(&mut f, &LightChange { mode: Some(mode), brightness: Some(brightness), ..Default::default() })
                    .unwrap();
                assert_eq!(
                    light_of(&as_reply(Kind::Light, f)).brightness,
                    Some(8),
                    "mode {mode} sends 8 whatever was chosen"
                );
            }
        }
    }

    #[test]
    fn every_polling_rate_round_trips() {
        for (id, code) in POLLING {
            let f = polling_frame(id).unwrap();
            assert_eq!(f, [0x06, 0x09, 0x01, code, !code, 0, 0, 0, 0]);
            let r = as_reply(Kind::Poll, f);
            check_reply(Kind::Poll, &r).unwrap();
            assert_eq!(polling_of(&r), Some(id));
        }
    }

    #[test]
    fn every_plain_action_round_trips_on_every_button_row() {
        for (setting, slot) in BUTTON_SLOTS {
            for (action, code) in ACTIONS {
                let mut f = cached(Kind::Buttons);
                patch_button(&mut f, slot, action).unwrap();
                seal(Kind::Buttons, &mut f);
                assert_eq!(f[3 + 3 * slot], code);
                let r = as_reply(Kind::Buttons, f.clone());
                check_reply(Kind::Buttons, &r).unwrap();
                assert_eq!(action_of(&r, slot), action, "{setting}");
                // No other slot moved.
                let before = cached(Kind::Buttons);
                for k in (0..SLOTS).filter(|k| *k != slot) {
                    assert_eq!(
                        f[3 + 3 * k..6 + 3 * k],
                        before[3 + 3 * k..6 + 3 * k],
                        "slot {k}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_action_with_parameters_or_an_unknown_code_shows_as_another_action_and_is_never_written() {
        let mut f = cached(Kind::Buttons);
        f[3 + 3 * 3] = 0x11; // keyboard key
        f[4 + 3 * 3] = 0x01;
        f[5 + 3 * 3] = 0x1B;
        seal(Kind::Buttons, &mut f);
        assert_eq!(action_of(&f, 3), OTHER_ACTION);
        f[3 + 3 * 3] = 0x77;
        f[4 + 3 * 3] = 0;
        f[5 + 3 * 3] = 0;
        assert_eq!(action_of(&f, 3), OTHER_ACTION, "an unknown code is not named");
        assert!(patch_button(&mut f, 3, OTHER_ACTION).is_err());
        assert!(patch_button(&mut f, 3, "launch-missiles").is_err());
    }

    #[test]
    fn the_button_map_keeps_a_left_click() {
        let mut f = cached(Kind::Buttons);
        assert!(has_left_click(&f));
        patch_button(&mut f, 0, "right-click").unwrap();
        assert!(!has_left_click(&f), "slot 0 was the only left click");
        patch_button(&mut f, 5, "left-click").unwrap();
        assert!(has_left_click(&f), "a left click on another slot counts");
    }

    // ------------------------------------------------------------------------- refusals

    #[test]
    fn bad_stage_lists_are_refused_with_a_reason() {
        let good = "800@#FF0000,1200@#00FF00,1600@#0000FF,3200@#FFFF00,5600@#00FFFF,26000@#FF00FF,off@#FF4000,off@#FFFFFF";
        assert_eq!(stages_to_text(&parse_stages(good).unwrap()), good);
        for (bad, why) in [
            ("800@#FF0000", "entries"),
            (&good.replace("800@", "75@"), "50 to 26000"),
            (&good.replace("26000@", "26050@"), "50 to 26000"),
            (&good.replace("800@", "0@"), "50 to 26000"),
            (&good.replace("800@#FF0000", "800"), "no colour"),
            (&good.replace("#FF0000", "red"), "#RRGGBB"),
            (&good.replace("#FF0000", "#FF00"), "#RRGGBB"),
            (&good.replace("800@", "fast@"), "resolution"),
        ] {
            let e = parse_stages(bad).expect_err(bad);
            assert!(e.contains(why), "{bad:?}: {e}");
        }
    }

    #[test]
    fn a_stage_table_with_nothing_on_or_the_active_stage_off_is_refused() {
        let mut f = cached(Kind::Dpi);
        let mut stages = stages_of(&f);
        stages[1].dpi = None; // stage 2 is the active one in the live read
        let e = patch_stages(&mut f, &stages).unwrap_err();
        assert!(e.contains("active"), "{e}");
        let none = [Stage { dpi: None, rgb: (0, 0, 0) }; STAGE_COUNT];
        assert!(patch_stages(&mut f, &none).unwrap_err().contains("at least one"));
        assert_eq!(f, cached(Kind::Dpi), "a refused patch left the frame alone");
    }

    #[test]
    fn out_of_range_lighting_values_are_refused() {
        for change in [
            LightChange { speed_slider: Some(3), ..Default::default() },
            LightChange { speed_slider: Some(9), ..Default::default() },
            LightChange { brightness: Some(0), ..Default::default() },
            LightChange { brightness: Some(9), ..Default::default() },
            LightChange { deep_sleep_min: Some(0), ..Default::default() },
            LightChange { deep_sleep_min: Some(61), ..Default::default() },
            LightChange { sleep_half_min: Some(0), ..Default::default() },
            LightChange { sleep_half_min: Some(61), ..Default::default() },
            LightChange { key_response_ms: Some(3), ..Default::default() },
            LightChange { key_response_ms: Some(51), ..Default::default() },
            LightChange { key_response_ms: Some(9), ..Default::default() },
        ] {
            let mut f = cached(Kind::Light);
            assert!(patch_light(&mut f, &change).is_err(), "{change:?}");
        }
        assert!(polling_frame("4000").is_err(), "4000 Hz is U10: not offered");
    }

    #[test]
    fn a_reply_is_accepted_only_when_it_is_this_reports_fresh_checksummed_answer() {
        for kind in READ_ORDER {
            let good = reply(kind);
            // Too short.
            assert!(check_reply(kind, &good[..kind.reply_len() - 1]).is_err(), "{kind:?} short");
            // Another report's bytes in the buffer.
            let mut other = good.clone();
            other[0] ^= 0x01;
            assert!(
                check_reply(kind, &other).unwrap_err().contains("report id")
                    || check_reply(kind, &other).is_err()
            );
            // A buffer the receiver has not refilled: the header does not echo this request's
            // length plus two.
            let mut stale = good.clone();
            stale[1] = stale[1].wrapping_sub(2);
            assert!(check_reply(kind, &stale).unwrap_err().contains("refilled"), "{kind:?}");
            // A flipped data byte breaks the checksum or the complement.
            let mut corrupt = good.clone();
            corrupt[if kind == Kind::Profile { 4 } else { 3 }] ^= 0x10;
            assert!(check_reply(kind, &corrupt).unwrap_err().contains("checksum"), "{kind:?}");
        }
    }

    #[test]
    fn the_read_requests_are_the_ones_the_live_test_sent() {
        // `SEND SET_FEATURE len=8` lines of the research log, minus report 0B.
        assert_eq!(read_request(Kind::Poll), [0xa0, 0x06, 0x09, 0x00, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(read_request(Kind::Dpi), [0xa0, 0x04, 0x38, 0x00, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(read_request(Kind::Light), [0xa0, 0x05, 0x0f, 0x00, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(read_request(Kind::Buttons), [0xa0, 0x08, 0x3b, 0x00, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(read_request(Kind::Profile), [0xa0, 0x0c, 0x0a, 0x00, 0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn the_identity_report_is_read_but_never_interpreted() {
        // Report 0B: version and identity, meaning UNKNOWN (research 3.5).
        assert_eq!(REPLY_0B.len(), 8);
        assert_eq!(REPLY_0B[0], 0x0b);
    }

    // -------------------------------------------------------------------------- status events

    #[test]
    fn the_live_events_decode() {
        // The listen of 2026-10-17: discharging, level byte 0x0a; with the cable, charging.
        assert_eq!(
            decode_event(&hex("03 10 40 01 0a")).unwrap(),
            Event::Battery { charging: false, fully_charged: false, level_byte: None }
        );
        assert_eq!(
            decode_event(&hex("03 10 40 03 0a")).unwrap(),
            Event::Battery { charging: true, fully_charged: false, level_byte: None }
        );
        assert_eq!(
            decode_event(&hex("03 10 40 02 0a")).unwrap(),
            Event::Battery { charging: false, fully_charged: true, level_byte: None }
        );
        // A level byte above 10 would settle U3 (percent); it is surfaced, not interpreted.
        assert_eq!(
            decode_event(&hex("03 10 40 01 5a")).unwrap(),
            Event::Battery { charging: false, fully_charged: false, level_byte: Some(0x5A) }
        );
        // The DPI button (decoded from the vendor app; not yet seen live).
        for stage in 1..=8 {
            assert_eq!(decode_event(&[3, 0x10, 0x10, stage, 0]).unwrap(), Event::DpiStage(stage));
        }
        // Well formed but not ours to show.
        assert_eq!(decode_event(&hex("03 10 50 00 04")).unwrap(), Event::Ignored);
        assert_eq!(
            decode_event(&hex("03 85 40 01 0a")).unwrap(),
            Event::Ignored,
            "the vendor app drops reports whose second byte is not 0x10"
        );
    }

    #[test]
    fn malformed_events_are_errors() {
        for bad in [
            "",
            "03",
            "03 10 40 01",
            "03 10 40 01 0a 00",
            "04 10 40 01 0a",
            "03 10 40 00 0a",
            "03 10 40 04 0a",
            "03 10 10 00 00",
            "03 10 10 09 00",
        ] {
            assert!(decode_event(&hex(bad)).is_err(), "{bad:?}");
        }
    }

    /// SplitMix64: the generator peripheral-hub's scenario format uses (fixed algorithm, no
    /// platform dependence).
    struct SplitMix64(u64);

    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
    }

    /// The fuzz count peripheral-hub's codec test used: 4 000 000 reports took about a second in
    /// a debug build there.
    const FUZZ_REPORTS: u32 = 4_000_000;

    #[test]
    fn fixed_seed_fuzz_of_status_reports_never_panics_and_only_decodes_what_the_layout_allows() {
        let mut rng = SplitMix64(0x5EED_0000_0000_0006);
        let (mut ok, mut err, mut ignored) = (0u32, 0u32, 0u32);
        let started = std::time::Instant::now();
        for _ in 0..FUZZ_REPORTS {
            let r = rng.next();
            // Lengths 0..=9, biased to 5; bytes biased toward the values the layout names so the
            // deep branches run.
            let len = match r & 7 {
                0 => (r >> 8) as usize % 10,
                _ => 5,
            };
            let mut data = Vec::with_capacity(len);
            for i in 0..len {
                let x = rng.next();
                let b = match (i, x & 3) {
                    (0, 0..=2) => 0x03,
                    (1, 0..=2) => 0x10,
                    (2, 0..=1) => 0x40,
                    (2, 2) => 0x10,
                    (3, 0..=1) => (x >> 8) as u8 % 5,
                    _ => (x >> 16) as u8,
                };
                data.push(b);
            }
            match decode_event(&data) {
                Ok(Event::Battery { .. }) => {
                    ok += 1;
                    assert!(
                        data.len() == 5
                            && data[0] == 3
                            && data[1] == 0x10
                            && data[2] == 0x40
                            && (1..=3).contains(&data[3]),
                        "{data:02x?}"
                    );
                }
                Ok(Event::DpiStage(s)) => {
                    ok += 1;
                    assert!(
                        data.len() == 5
                            && data[0] == 3
                            && data[1] == 0x10
                            && data[2] == 0x10
                            && (1..=8).contains(&s),
                        "{data:02x?}"
                    );
                }
                Ok(Event::Ignored) => {
                    ignored += 1;
                    assert!(data.len() == 5 && data[0] == 3, "{data:02x?}");
                }
                Err(_) => err += 1,
            }
        }
        println!(
            "fuzz: {FUZZ_REPORTS} reports in {:?}: {ok} decoded, {ignored} ignored, {err} malformed",
            started.elapsed()
        );
        assert!(ok > 1000 && err > 1000 && ignored > 1000, "every branch ran: {ok} {ignored} {err}");
    }

    // ------------------------------------- the button checksum covers all 18 slots, the last one included

    /// Slot 17 holds a keyboard-key action (code 0x11, modifier, usage): a non-plain action whose
    /// last byte (frame byte 56) is the last byte the checksum covers. A checksum that stops one
    /// byte early passes every frame whose slot 17 is plain (all zero there) and fails this one.
    #[test]
    fn the_button_checksum_covers_the_last_byte_of_slot_17() {
        let mut frame = write_frame_from_reply(Kind::Buttons, &reply(Kind::Buttons));
        let at = 3 + 3 * 17;
        assert_eq!(at + 2, 56, "slot 17 ends at frame byte 56");
        frame[at..at + 3].copy_from_slice(&[0x11, 0x02, 0x1E]);
        seal(Kind::Buttons, &mut frame);
        // The checksum, computed here from the layout in the research (16-bit big-endian sum of
        // bytes 3..=56 at 57..59).
        let want: u32 = frame[3..=56].iter().map(|b| u32::from(*b)).sum();
        assert_eq!(u32::from(frame[57]) << 8 | u32::from(frame[58]), want & 0xFFFF);
        // The device would answer this frame with the header raised by two; the reply check
        // accepts it.
        check_reply(Kind::Buttons, &as_reply(Kind::Buttons, frame.clone()))
            .expect("a frame with a keyboard action in slot 17 is a valid reply");
        // And it rejects the same frame with that last byte changed but the checksum left alone.
        let mut tampered = as_reply(Kind::Buttons, frame.clone());
        tampered[56] ^= 0x01;
        assert!(check_reply(Kind::Buttons, &tampered).is_err(), "byte 56 is under the checksum");
        // Slot 17 is not one of the six rows, and the rows still decode from this frame.
        assert_eq!(action_of(&frame, 17), OTHER_ACTION);
        assert_eq!(action_of(&as_reply(Kind::Buttons, frame), 0), "left-click");
    }
}
