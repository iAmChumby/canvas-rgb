//! The 8BitDo Ultimate 2's vendor channel and the profile image it holds, as pure byte work.
//!
//! Ported from `peripheral-hub`'s `devices/8bitdo-ultimate-2/src/vendor.rs` and `image.rs`: the
//! same framing, the same CRC, the same refusals. peripheral-hub's WASM guest ABI has no
//! counterpart here: this module is purely byte-level, and the adapter in `mod.rs` turns its
//! numbers into the spine's properties.
//!
//! Sources, all in `peripheral-hub/docs/research/8bitdo-ultimate-2.md`:
//! - section 5.1 and 5.2: the transport (interrupt endpoints, output report `0x81`, input report
//!   `0x02`, 64 bytes on the wire) and the framing (`81 04 <body>`);
//! - section 5.3: the section-04 body header, the chunked read, the per-field write, the commit
//!   and the CRC-16/MODBUS per chunk;
//! - section 5.4: the 1592-byte image (`custom_config_record_t_u2`) and where every record sits;
//! - section 2.2d and the 2026-10-17 capture: the identify request and the shape of its reply;
//! - section 7.1: the DInput input report (the paddles, the fast bumpers and the battery).
//!
//! **None of the reply rules has been seen on the pad.** The profile read has never run
//! (research 2.2c: the auto-mode classifier refused the script), so every rule about a reply is
//! static: the layout the vendor's own writer uses, cross-checked against the vendor's builder at
//! RVA `0x400e80` and the third-party analysis the research quotes. The research's errata says
//! the same: treat the request header as UNKNOWN until a read has run on hardware.
//!
//! Every function here is pure: bytes in, bytes out. Nothing in this file touches hardware.

use std::fmt::Write as _;

// ------------------------------------------------------------------------------------- identity

/// The vendor id of every 8BitDo device.
pub const VENDOR_ID: u16 = 0x2DC8;
/// The product id of the 2.4G receiver in its idle personality: no controller is linked.
pub const PID_IDLE: u16 = 0x6013;
/// The product id of the pad in the XInput personality: the default, and the one this PC sees.
pub const PID_XINPUT: u16 = 0x310B;
/// The product id of the pad in the DInput personality.
pub const PID_DINPUT: u16 = 0x6012;

// ------------------------------------------------------------------------------- the vendor channel

/// The report id of every output report on the vendor channel.
pub const OUT_ID: u8 = 0x81;
/// The report id of every input report on the vendor channel.
pub const IN_ID: u8 = 0x02;
/// Wire length of a report: the id plus 63 data bytes (the descriptor's report count is `0x3F`).
pub const REPORT_LEN: usize = 64;
/// Size of the profile image (`0x638`), the vendor's chunk-loop constant.
pub const IMAGE_LEN: usize = 0x638;
/// Data bytes per read chunk (the vendor's chunk size for the profile image).
pub const CHUNK: usize = 0x2D;
/// Byte index of the chunk data in a request or a reply (the id, the class byte, then the header).
pub const DATA_AT: usize = 0x12;
/// Filler for the data bytes of a read request. The vendor sends uninitialised stack bytes
/// there; the third-party tool and `profile_read.py` send this constant, and the CRC in the
/// request covers it.
pub const READ_FILL: u8 = 0xCC;

/// Request word 1: write one chunk.
pub const REQ_WRITE: u16 = 1;
/// Request word 2: read one chunk.
pub const REQ_READ: u16 = 2;
/// Request word 3: poll the key state.
pub const REQ_POLL_KEY: u16 = 3;
/// Request word 6: commit what was written.
pub const REQ_COMMIT: u16 = 6;
/// Request word 7: `Reportstate`, the vendor's "a host has claimed the device" handshake.
pub const REQ_REPORT_STATE: u16 = 7;
/// The commit argument (research 5.3): `0x0123`.
pub const COMMIT_ARG: u16 = 0x0123;

/// CRC-16/MODBUS: init `0xFFFF`, reflected, polynomial `0xA001` (research 5.3).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// The section-04 body: the report id, the class byte, the 16-byte header and the data.
fn frame(req: u16, arg: u16, len: u32, crc: u16, total: u32, offset: u32, data: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; REPORT_LEN];
    p[0] = OUT_ID;
    p[1] = 0x04;
    p[2..4].copy_from_slice(&req.to_le_bytes());
    p[4..6].copy_from_slice(&arg.to_le_bytes());
    p[6..10].copy_from_slice(&(len | (u32::from(crc) << 16)).to_le_bytes());
    p[10..14].copy_from_slice(&total.to_le_bytes());
    p[14..18].copy_from_slice(&offset.to_le_bytes());
    p[DATA_AT..DATA_AT + data.len()].copy_from_slice(data);
    p
}

/// The read request for the chunk at `offset` (its length is `min(CHUNK, IMAGE_LEN - offset)`).
pub fn read_request(offset: usize) -> Result<Vec<u8>, String> {
    if offset >= IMAGE_LEN {
        return Err(format!("offset {offset:#x} is past the image"));
    }
    let len = CHUNK.min(IMAGE_LEN - offset);
    let filler = vec![READ_FILL; len];
    Ok(frame(REQ_READ, 0, len as u32, crc16(&filler), IMAGE_LEN as u32, offset as u32, &filler))
}

/// The write request for one stored field: `data` goes to image offset `offset` (at most one chunk).
pub fn write_request(offset: usize, data: &[u8]) -> Result<Vec<u8>, String> {
    if data.is_empty() || data.len() > CHUNK || offset + data.len() > IMAGE_LEN {
        return Err(format!("{} bytes at {offset:#x} do not fit one chunk of the image", data.len()));
    }
    Ok(frame(REQ_WRITE, 0, data.len() as u32, crc16(data), IMAGE_LEN as u32, offset as u32, data))
}

/// The commit that follows a field write: `81 04 06 00 23 01` + zeros (request word 6, the
/// argument `0x0123`).
///
/// The NO-length form. Research 5.2 [V]: once the vendor app has classified a `310B` pad as an
/// Ultimate 2 (`SelectPlatform` sets the DLL's current PID to `0x6012`), config traffic is
/// `81 04 <payload>` with no length byte; the length-byte form `81 11 04 ...` (research 5.3's
/// "wrapped" commit) is only what the connect-time probes use before that decision. The profile
/// read already uses `81 04` and is confirmed on the test pad, so the commit uses it too.
pub fn commit_request() -> Vec<u8> {
    frame(REQ_COMMIT, COMMIT_ARG, 0, 0, 0, 0, &[])
}

/// The identify request the vendor app sends (`81 05 00 21 01`, the trace's line 4).
pub fn identify_request() -> Vec<u8> {
    let mut p = vec![0u8; REPORT_LEN];
    p[..5].copy_from_slice(&[OUT_ID, 0x05, 0x00, 0x21, 0x01]);
    p
}

/// The `Reportstate` packet inside a config session: `81 04 07 00 <state> 00` + zeros.
///
/// The native `Reportstate` goes through the same framing function as every other config packet
/// (research 5.2, RVA `0x4015f0` into `0x4050f0`), so once the pad is classified as an Ultimate 2
/// it is the NO-length form. The vendor's write wrappers send `Reportstate(0)` before and
/// `Reportstate(1)` after each write (`Ultimate2_4Advance2.excerpt.cs`, the static write
/// wrappers, lines ~620-760), always after `SelectPlatform` has classified the pad.
pub fn report_state_request(state: u16) -> Vec<u8> {
    let mut p = vec![0u8; REPORT_LEN];
    p[0] = OUT_ID;
    p[1] = 0x04;
    p[2..4].copy_from_slice(&REQ_REPORT_STATE.to_le_bytes());
    p[4..6].copy_from_slice(&state.to_le_bytes());
    p
}

/// The `Reportstate` packet in the connect-time length form (`81 11 04 07 00 01`, the trace's
/// line 6), seen before the vendor app has classified the pad. The adapter never sends it.
pub fn report_state_connect_request(state: u16) -> Vec<u8> {
    let mut p = vec![0u8; REPORT_LEN];
    p[0] = OUT_ID;
    p[1] = 0x11;
    p[2] = 0x04;
    p[3..5].copy_from_slice(&REQ_REPORT_STATE.to_le_bytes());
    p[5..7].copy_from_slice(&state.to_le_bytes());
    p
}

/// Check the pad's acknowledgement of a write or commit request: the reply must be report `0x02`
/// with class `04`, carry the `04 00` header copy and answer request word `want` (1 for a write
/// chunk, 6 for the commit). This is the same header the read reply carries (the read loop's
/// rule), so it is confirmed for reads on the test pad; the pad's write and commit replies have
/// never been seen, so no other field is checked.
pub fn parse_ack_reply(frame: &[u8], want: u16) -> Result<(), String> {
    if frame.len() < 6 {
        return Err(format!("reply is {} bytes, an acknowledgement needs 6", frame.len()));
    }
    if frame[0] != IN_ID || frame[1] != 0x04 {
        return Err(format!("reply starts {:02x} {:02x}, not {IN_ID:02x} 04", frame[0], frame[1]));
    }
    if u16::from_le_bytes([frame[2], frame[3]]) != 4 {
        return Err("reply does not carry the 04 00 header copy".into());
    }
    let req = u16::from_le_bytes([frame[4], frame[5]]);
    if req != want {
        return Err(format!("reply answers request word {req}, not {want}"));
    }
    Ok(())
}

/// A pad acknowledgement as the fake pad sends it: `02 04 04 00 <request word>` + zeros.
pub fn ack_frame(request_word: u16) -> Vec<u8> {
    let mut f = vec![0u8; REPORT_LEN];
    f[0] = IN_ID;
    f[1] = 0x04;
    f[2..4].copy_from_slice(&4u16.to_le_bytes());
    f[4..6].copy_from_slice(&request_word.to_le_bytes());
    f
}

/// Check a reply to a read request for `expect_len` bytes at `expect_offset` and return the chunk
/// data.
///
/// A reply is accepted only when it has the id and the `04 00` header copy, echoes request word 2
/// and the offset asked for, states the image's total size and the length asked for, and carries a
/// matching CRC. The echo checks are the vendor DLL's own rule (research 5.3): a reply that names
/// another offset belongs to another request and is not this chunk.
pub fn parse_read_reply(
    frame: &[u8],
    expect_len: usize,
    expect_offset: usize,
) -> Result<&[u8], String> {
    if frame.len() < DATA_AT + expect_len {
        return Err(format!(
            "reply is {} bytes, a {expect_len}-byte chunk needs {}",
            frame.len(),
            DATA_AT + expect_len
        ));
    }
    if frame[0] != IN_ID || frame[1] != 0x04 {
        return Err(format!("reply starts {:02x} {:02x}, not {IN_ID:02x} 04", frame[0], frame[1]));
    }
    if u16::from_le_bytes([frame[2], frame[3]]) != 4 {
        return Err("reply does not carry the 04 00 header copy".into());
    }
    let req = u16::from_le_bytes([frame[4], frame[5]]);
    if req != REQ_READ {
        return Err(format!("reply answers request word {req}, not {REQ_READ}"));
    }
    let total = u32::from_le_bytes([frame[10], frame[11], frame[12], frame[13]]);
    if total as usize != IMAGE_LEN {
        return Err(format!("reply states a total of {total} bytes, not {IMAGE_LEN}"));
    }
    let offset = u32::from_le_bytes([frame[14], frame[15], frame[16], frame[17]]);
    if offset as usize != expect_offset {
        return Err(format!("reply echoes offset {offset}, {expect_offset} was asked for"));
    }
    let word = u32::from_le_bytes([frame[6], frame[7], frame[8], frame[9]]);
    let (len, crc) = ((word & 0xFFFF) as usize, (word >> 16) as u16);
    if len != expect_len {
        return Err(format!("reply states {len} bytes, {expect_len} were asked for"));
    }
    let data = &frame[DATA_AT..DATA_AT + len];
    let calc = crc16(data);
    // In the XInput personality (310b) the pad echoes the CRC our read request carried (the CRC
    // of the `READ_FILL` filler) instead of a CRC of its reply: seen live on 2026-10-27, chunk 0
    // answered 0x4f5b = CRC-16/MODBUS of 45 bytes of 0xCC. The research lists only the
    // CRC-capable personalities (6012, 6013, ...) as checking chunk data (research 5.3).
    let echo = crc16(&vec![READ_FILL; len]);
    if calc != crc && crc != echo {
        let raw: Vec<String> = frame.iter().map(|b| format!("{b:02x}")).collect();
        return Err(format!(
            "chunk CRC {crc:#06x} does not match the data ({calc:#06x}); reply: {}",
            raw.join(" ")
        ));
    }
    Ok(data)
}

/// What the identify reply states.
///
/// The reply seen on 2026-10-17 is `02 22 65 00 ...`; byte 2 is the only varying byte of the
/// logged reply that the research ties to a version (`0x65` also sits at byte 18 of the idle
/// query reply). Its scale is UNKNOWN, so it is a code and never a `major.minor`.
pub fn parse_identify_reply(frame: &[u8]) -> Result<u8, String> {
    if frame.len() < 4 {
        return Err(format!("identify reply is {} bytes", frame.len()));
    }
    if frame[0] != IN_ID || frame[1] != 0x22 {
        return Err(format!("identify reply starts {:02x} {:02x}, not {IN_ID:02x} 22", frame[0], frame[1]));
    }
    Ok(frame[2])
}

// -------------------------------------------------------------------------------- the profile image

/// The vendor app's "this record is customised" mark (`Ultimate2_4Advance2UI.enableFlag`).
pub const ENABLED: u32 = 0x2020_0911;

/// Offset of the three per-profile "in use" marks.
pub const FLAGS: usize = 0x00;
/// Offset of the stored gamepad mode.
pub const MODE: usize = 0x10;
/// Offset of the active profile (`cur_slot`).
pub const CUR_SLOT: usize = 0x12;
/// Offset of the three profile names.
pub const NAME: usize = 0x14;
/// Offset of the three rumble records.
pub const RUMBLE: usize = 0x74;
/// Offset of the three joystick records.
pub const JOYSTICK: usize = 0x98;
/// Offset of the three trigger records.
pub const TRIGGER: usize = 0xB0;
/// Offset of the three special-feature records.
pub const SPECIAL: usize = 0xC8;
/// Offset of the three button-map records.
pub const MAP_KEY: usize = 0xE0;
/// Offset of the three macro records.
pub const MACRO: usize = 0x1F4;
/// Offset of the three XInput rumble records.
pub const X_RUMBLE: usize = 0x47C;
/// Offset of the three gyro records.
pub const SIXAXIS: usize = 0x494;
/// Offset of the three light-tracing theme records.
pub const DYNAMIC: usize = 0x4B8;
/// Offset of the three fire-ring theme records.
pub const HOTWHEEL: usize = 0x4DC;
/// Offset of the three custom theme records.
pub const SINGLE: usize = 0x50C;

/// The size of a profile name.
pub const NAME_LEN: usize = 32;
/// The size of a stick or trigger range record: a flag word and four bytes.
pub const RANGE_RECORD_LEN: usize = 8;
/// The size of a rumble record: a flag word and two float32 strengths.
pub const RUMBLE_RECORD_LEN: usize = 12;
/// The size of a button-map record: a flag word and 22 target masks.
pub const MAP_RECORD_LEN: usize = 0x5C;
/// How many profiles the pad holds.
pub const PROFILES: usize = 3;
/// The first map slot of the four extra buttons (`readkey_map`: 18 P1, 19 P2, 20 P3, 21 P4).
pub const EXTRA_BUTTON_FIRST: usize = 18;
/// How many map slots a profile has.
pub const MAP_SLOTS: usize = 22;

/// The sizes of the records of `custom_config_record_t_u2`, in image order. The sum is the image
/// size the vendor's chunk loop uses (`0x638`), and the offsets are the research's table (5.4).
pub const LAYOUT: [(&str, usize); 16] = [
    ("flag[3]", 12),
    ("crc_value", 4),
    ("gamepad_mode", 2),
    ("cur_slot", 2),
    ("file_name[3]", 96),
    ("rumble[3]", 36),
    ("joystick[3]", 24),
    ("trigger[3]", 24),
    ("special_feature[3]", 24),
    ("map_key[3]", 276),
    ("record_macro_fun[3]", 648),
    ("x_rumble[3]", 24),
    ("sixaxis[3]", 36),
    ("dynamic[3]", 36),
    ("hotwheel[3]", 48),
    ("single[3]", 300),
];

/// The offset of a record by its name in [`LAYOUT`].
pub fn offset_of(name: &str) -> Option<usize> {
    let mut at = 0usize;
    for (n, size) in LAYOUT {
        if n == name {
            return Some(at);
        }
        at += size;
    }
    None
}

fn u16_at(img: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([img[at], img[at + 1]])
}

fn u32_at(img: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([img[at], img[at + 1], img[at + 2], img[at + 3]])
}

fn f32_at(img: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([img[at], img[at + 1], img[at + 2], img[at + 3]])
}

/// The stick and trigger range rows: the suffix of the property key, the record it lives in and
/// the byte inside that record.
pub const RANGE_ROWS: [(&str, usize, usize); 8] = [
    ("stick-left-start", JOYSTICK, 4),
    ("stick-left-end", JOYSTICK, 5),
    ("stick-right-start", JOYSTICK, 6),
    ("stick-right-end", JOYSTICK, 7),
    ("trigger-left-start", TRIGGER, 4),
    ("trigger-left-end", TRIGGER, 5),
    ("trigger-right-start", TRIGGER, 6),
    ("trigger-right-end", TRIGGER, 7),
];

/// The property key of one of a profile's range rows.
pub fn range_key(profile: usize, suffix: &str) -> String {
    format!("profile-{}-{suffix}", profile + 1)
}

/// The profile and the row suffix a range key stands for.
pub fn split_range_key(key: &str) -> Option<(usize, &str)> {
    let rest = key.strip_prefix("profile-")?;
    let (profile, suffix) = rest.split_once('-')?;
    let profile: usize = profile.parse().ok()?;
    if !(1..=PROFILES).contains(&profile) {
        return None;
    }
    Some((profile - 1, suffix))
}

/// Where a writable row lives: the record to send (its offset and length in the image) and the
/// byte inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// Where the record starts in the image.
    pub record_at: usize,
    /// Which byte of the record holds the value.
    pub byte_in_record: usize,
}

/// The field a range key stands for, for `profile` (0-based).
pub fn field_of(key: &str) -> Option<Field> {
    let (profile, suffix) = split_range_key(key)?;
    let (_, base, byte) = RANGE_ROWS.iter().find(|(id, _, _)| *id == suffix)?;
    Some(Field { record_at: base + RANGE_RECORD_LEN * profile, byte_in_record: *byte })
}

/// A stored range byte as a percentage of the full travel.
///
/// The vendor stores a percentage as `percent * 255 / 100` (research 1: the trigger range), so a
/// byte is a percentage of the travel, rounded to the nearest percent.
pub fn byte_to_percent(byte: u8) -> f64 {
    (f64::from(byte) * 100.0 / 255.0).round()
}

/// A percentage as the byte the vendor stores.
pub fn percent_to_byte(percent: f64) -> u8 {
    (percent * 255.0 / 100.0).round().clamp(0.0, 255.0) as u8
}

/// The named button targets and the key mask each travels under (research 5.4).
///
/// The masks are the vendor's own `KeyMap` values (`AdvanceSuper.Mode.KeyMap` in the decompile,
/// the table the third-party layout quotes). Two of them are exchanged by the vendor app at run
/// time: the app's constructor starts with R2 at `0x20000` and Home at `0x8000`, and
/// `AutoR2AndHome` calls `changeKey`, which swaps them to the values below (research 5.4: "R2
/// `0x8000` (after V2's R2/Home swap fix-up; raw constants in source start with R2 `0x20000`,
/// Home `0x8000`), Home `0x20000`"). The table below is the one both the research and the
/// third-party layout give as the effective one, so `0x8000` is R2 (the right trigger) and
/// `0x20000` is Home.
///
/// The masks that belong to other products are left out: the stick directions (`0x80000`,
/// `0x100000`, `0x8000010` and friends) and `Record` are HitBox-only and store zero on the
/// Ultimate 2 (the third-party layout: "on the Ultimate 2 (`6012`) the stick ids and Record
/// store 0 too"), and `BT_CON` (`0x40000`) is named by neither source in words this build can
/// use. A mask that is not in the table is shown as "another action" and is never written.
pub const TARGETS: [(&str, u32); 27] = [
    ("none", 0),
    ("start", 0x1),
    ("left-stick-click", 0x2),
    ("right-stick-click", 0x4),
    ("select", 0x8),
    ("x", 0x10),
    ("y", 0x20),
    ("dpad-right", 0x40),
    ("dpad-left", 0x80),
    ("dpad-down", 0x100),
    ("dpad-up", 0x200),
    ("left-bumper", 0x400),
    ("right-bumper", 0x800),
    ("b", 0x1000),
    ("a", 0x2000),
    ("left-trigger", 0x4000),
    ("right-trigger", 0x8000),
    ("menu", 0x10000),
    ("home", 0x20000),
    ("paddle-3", 0x200000),
    ("screenshot", 0x400000),
    ("turbo-single", 0x800000),
    ("turbo-auto", 0x1000000),
    ("paddle-1", 0x2000000),
    ("paddle-2", 0x4000000),
    ("paddle-4", 0x40000000),
    ("swap", 0x8000000),
];

/// The name a mask with parameters, an unknown mask, or a swapped one, is shown under.
pub const OTHER_TARGET: &str = "other";

/// The key mask of a button slot, decoded into a choice option.
pub fn button_target(mask: u32) -> &'static str {
    TARGETS.iter().find(|(_, m)| *m == mask).map_or(OTHER_TARGET, |(id, _)| *id)
}

/// The key mask a named target travels under.
pub fn target_mask(target: &str) -> Option<u32> {
    TARGETS.iter().find(|(id, _)| *id == target).map(|(_, m)| *m)
}

/// The choices a button row offers: the named targets and "another action".
pub fn target_options() -> Vec<String> {
    let mut out: Vec<String> = TARGETS.iter().map(|(id, _)| (*id).to_string()).collect();
    out.push(OTHER_TARGET.to_string());
    out
}

/// The map slots, in the vendor's order (research 5.4, `readkey_map`): 0 A, 1 B, 2 X, 3 Y, 4 L1,
/// 5 R1, 6 L2, 7 R2, 8 L3, 9 R3, 10 Select, 11 Start, 12 Menu/Share, 13 Home, 14..17 the D-pad,
/// 18..21 the four extra buttons (P1..P4).
pub const MAP_SLOT_NAMES: [&str; MAP_SLOTS] = [
    "a", "b", "x", "y", "l1", "r1", "l2", "r2", "l3", "r3", "select", "start", "menu", "home",
    "dpad-up", "dpad-down", "dpad-left", "dpad-right", "p1", "p2", "p3", "p4",
];

/// What the editor shows for a map slot.
pub const MAP_SLOT_LABELS: [&str; MAP_SLOTS] = [
    "A", "B", "X", "Y", "L1", "R1", "L2", "R2", "L3", "R3", "Select", "Start", "Menu / Share",
    "Home", "D-pad up", "D-pad down", "D-pad left", "D-pad right", "Paddle 1 (P1)",
    "Paddle 2 (P2)", "Paddle 3 (P3)", "Paddle 4 (P4)",
];

/// The mode the pad stores (`GAMEPAD_MODE`): 0 Switch Pro, 1 DInput, 2 Mac, 3 XInput, 4 Extend.
pub fn mode_name(mode: u16) -> Option<&'static str> {
    Some(match mode {
        0 => "Switch Pro",
        1 => "DInput",
        2 => "Mac",
        3 => "XInput",
        4 => "Extend",
        _ => return None,
    })
}

/// What one profile holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// The vendor app marks this profile's slot as in use.
    pub in_use: bool,
    /// The profile's name, when the bytes hold plain text.
    pub name: Option<String>,
    /// The stick range: left start, left end, right start, right end, as stored bytes.
    pub stick: [u8; 4],
    /// The trigger range: left start, left end, right start, right end, as stored bytes.
    pub trigger: [u8; 4],
    /// The two motor strengths, as the pad stores them (0.0..1.0).
    pub vibration: [f32; 2],
    /// The 22 map slots: the target key mask of each button.
    pub map: [u32; MAP_SLOTS],
    /// The vendor app marks the stick record as in use.
    pub stick_in_use: bool,
    /// The vendor app marks the trigger record as in use.
    pub trigger_in_use: bool,
    /// The vendor app marks the rumble record as in use.
    pub rumble_in_use: bool,
    /// The vendor app marks the button-map record as in use.
    pub map_in_use: bool,
}

/// A profile name: the bytes up to the first NUL, as UTF-8 with no control characters. The
/// vendor's `byte[32]` has no documented encoding, so anything that is not plain UTF-8 is not
/// reported.
fn name_of(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let s = std::str::from_utf8(&bytes[..end]).ok()?;
    if s.is_empty() || s.chars().any(char::is_control) {
        return None;
    }
    Some(s.to_string())
}

/// Decode one profile (0..=2) out of the image.
pub fn decode_profile(img: &[u8], profile: usize) -> Result<Profile, String> {
    if img.len() != IMAGE_LEN {
        return Err(format!("the image is {} bytes, not {IMAGE_LEN}", img.len()));
    }
    if profile >= PROFILES {
        return Err(format!("profile index {profile} is past the three profiles"));
    }
    let stick_at = JOYSTICK + RANGE_RECORD_LEN * profile;
    let trigger_at = TRIGGER + RANGE_RECORD_LEN * profile;
    let rumble_at = RUMBLE + RUMBLE_RECORD_LEN * profile;
    let map_at = MAP_KEY + MAP_RECORD_LEN * profile;
    let mut map = [0u32; MAP_SLOTS];
    for (i, slot) in map.iter_mut().enumerate() {
        *slot = u32_at(img, map_at + 4 + 4 * i);
    }
    Ok(Profile {
        in_use: u32_at(img, FLAGS + 4 * profile) == ENABLED,
        name: {
            let at = NAME + NAME_LEN * profile;
            name_of(&img[at..at + NAME_LEN])
        },
        stick: [
            img[stick_at + 4],
            img[stick_at + 5],
            img[stick_at + 6],
            img[stick_at + 7],
        ],
        trigger: [
            img[trigger_at + 4],
            img[trigger_at + 5],
            img[trigger_at + 6],
            img[trigger_at + 7],
        ],
        vibration: [f32_at(img, rumble_at + 4), f32_at(img, rumble_at + 8)],
        map,
        stick_in_use: u32_at(img, stick_at) == ENABLED,
        trigger_in_use: u32_at(img, trigger_at) == ENABLED,
        rumble_in_use: u32_at(img, rumble_at) == ENABLED,
        map_in_use: u32_at(img, map_at) == ENABLED,
    })
}

/// The active profile the image names (0..=2), when it names one that exists.
pub fn active_profile(img: &[u8]) -> Option<usize> {
    if img.len() < IMAGE_LEN {
        return None;
    }
    let slot = usize::from(u16_at(img, CUR_SLOT));
    (slot < PROFILES).then_some(slot)
}

/// The mode the image stores, when it stores one the research names.
pub fn stored_mode(img: &[u8]) -> Option<&'static str> {
    if img.len() < IMAGE_LEN {
        return None;
    }
    mode_name(u16_at(img, MODE))
}

/// The bytes of the record that holds `key`, with the byte replaced by `value`, ready for a field
/// write.
///
/// With the value the image already holds this returns the record exactly as read (the
/// unchanged-value write). A record the vendor app does not mark as in use is never written: the
/// values in it are not known to be in effect.
pub fn patched_record(img: &[u8], key: &str, value: f64) -> Result<(usize, Vec<u8>), String> {
    if img.len() != IMAGE_LEN {
        return Err(format!("the image is {} bytes, not {IMAGE_LEN}", img.len()));
    }
    let f = field_of(key).ok_or_else(|| format!("setting {key} cannot be written"))?;
    if !(0.0..=100.0).contains(&value) {
        return Err(format!("{key} is {value}%; it takes 0 to 100"));
    }
    if u32_at(img, f.record_at) != ENABLED {
        return Err(format!(
            "the record behind {key} is not marked in use on the pad, so it is not written"
        ));
    }
    let mut rec = img[f.record_at..f.record_at + RANGE_RECORD_LEN].to_vec();
    let current = rec[f.byte_in_record];
    let wanted = percent_to_byte(value);
    // Writing the value the pad already holds must not move a single byte: the percentage a byte
    // decodes to does not always come back to the same byte.
    if wanted != current && byte_to_percent(current) == value.round() {
        // keep the byte as it is
    } else {
        rec[f.byte_in_record] = wanted;
    }
    Ok((f.record_at, rec))
}

// ------------------------------------------------------------------------------- the button map

/// Where one button-map entry lives in the image: the profile's map record, the 22 target masks
/// after the record's flag, and the entry's place among them.
pub fn map_entry_offset(profile: usize, slot: usize) -> Option<usize> {
    if profile >= PROFILES || slot >= MAP_SLOTS {
        return None;
    }
    Some(MAP_KEY + MAP_RECORD_LEN * profile + 4 + 4 * slot)
}

/// The bytes of one button-map entry, with the target replaced: a field write.
///
/// The write is the vendor's own shape: one request-1 chunk carrying the four bytes of the
/// entry at the entry's offset (the third-party layout's writer table: "button map | `0xe0`
/// flag, keys at `0xe4` | profile `0x5c`, entry 4 | 4"), then the commit. Every other byte of
/// the image stays as the pad has it.
pub fn patched_map_entry(
    img: &[u8],
    profile: usize,
    slot: usize,
    target: &str,
) -> Result<(usize, Vec<u8>), String> {
    if img.len() != IMAGE_LEN {
        return Err(format!("the image is {} bytes, not {IMAGE_LEN}", img.len()));
    }
    let at = map_entry_offset(profile, slot)
        .ok_or_else(|| format!("slot {slot} of profile {} is not a map slot", profile + 1))?;
    let mask =
        target_mask(target).ok_or_else(|| format!("{target:?} is not a target this build writes"))?;
    let mut entry = vec![0u8; 4];
    entry.copy_from_slice(&mask.to_le_bytes());
    Ok((at, entry))
}

// ------------------------------------------------------------------------------- the image's CRC

/// The image's own CRC: the value the pad stores and the candidates the research allows.
///
/// The research (5.3) says the value at image offset `0x0C` is a CRC-16/MODBUS "over all 1592
/// bytes" and that "the byte range the pad checks is unknown". The third-party layout says the
/// vendor computes it "on the image as read" and stores it before the full write, so the field
/// holds the CRC of the image *with whatever the field held before*. That leaves four
/// candidates, and this build computes all of them and says which one matches, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageCrc {
    /// The value stored at image offset `0x0C`.
    pub stored: u32,
    /// CRC-16/MODBUS over all 1592 bytes, as the image was read.
    pub whole: u16,
    /// CRC-16/MODBUS over all 1592 bytes with the CRC field itself zeroed.
    pub zeroed: u16,
    /// CRC-16/MODBUS over the image without the CRC field: bytes `0x00..0x0C`, then `0x10..end`.
    pub without_field: u16,
    /// CRC-16/MODBUS over the bytes after the CRC field.
    pub after_field: u16,
}

impl ImageCrc {
    /// True when the stored value can be a CRC at all: the vendor's CRC is a `ushort`, so a
    /// stored value with bits above the low half set is something else.
    pub fn stored_can_be_a_crc(&self) -> bool {
        self.stored <= u32::from(u16::MAX)
    }

    /// The candidate that matches the stored value, when one of them does.
    pub fn matching(&self) -> Option<&'static str> {
        let stored = self.stored as u16;
        if stored == self.whole {
            return Some("all 1592 bytes");
        }
        if stored == self.zeroed {
            return Some("all 1592 bytes with the CRC field zeroed");
        }
        if stored == self.without_field {
            return Some("the image without the CRC field");
        }
        if stored == self.after_field {
            return Some("the bytes after the CRC field");
        }
        None
    }
}

/// The image's CRC candidates and the value the pad stores.
pub fn image_crc(img: &[u8]) -> Result<ImageCrc, String> {
    if img.len() != IMAGE_LEN {
        return Err(format!("the image is {} bytes, not {IMAGE_LEN}", img.len()));
    }
    let mut zeroed = img.to_vec();
    zeroed[0x0C..0x10].copy_from_slice(&[0, 0, 0, 0]);
    let mut without = Vec::with_capacity(img.len() - 4);
    without.extend_from_slice(&img[..0x0C]);
    without.extend_from_slice(&img[0x10..]);
    Ok(ImageCrc {
        stored: u32_at(img, 0x0C),
        whole: crc16(img),
        zeroed: crc16(&zeroed),
        without_field: crc16(&without),
        after_field: crc16(&img[0x10..]),
    })
}

// ----------------------------------------------------------------------------------- the XInput view

/// One XInput state, as the XInput API reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct XState {
    /// The packet counter.
    pub packet: u32,
    /// The button mask.
    pub buttons: u16,
    /// The left trigger.
    pub lt: u8,
    /// The right trigger.
    pub rt: u8,
    /// The left stick, x.
    pub lx: i16,
    /// The left stick, y.
    pub ly: i16,
    /// The right stick, x.
    pub rx: i16,
    /// The right stick, y.
    pub ry: i16,
}

/// The XInput button bits and the names the vendor's own poll log uses. The guide bit (`0x0400`)
/// is not in the documented set and is not named.
pub const XINPUT_BUTTONS: [(&str, u16); 14] = [
    ("dpad-up", 0x0001),
    ("dpad-down", 0x0002),
    ("dpad-left", 0x0004),
    ("dpad-right", 0x0008),
    ("start", 0x0010),
    ("back", 0x0020),
    ("left-stick-click", 0x0040),
    ("right-stick-click", 0x0080),
    ("left-bumper", 0x0100),
    ("right-bumper", 0x0200),
    ("a", 0x1000),
    ("b", 0x2000),
    ("x", 0x4000),
    ("y", 0x8000),
];

/// The four coarse battery levels the XInput API reports.
pub const XINPUT_BATTERY_LEVELS: [&str; 4] = ["empty", "low", "medium", "full"];

/// The name of an XInput battery level (`XINPUT_BATTERY_STATE`).
pub fn battery_level_name(state: u8) -> &'static str {
    XINPUT_BATTERY_LEVELS.get(usize::from(state)).copied().unwrap_or("unknown")
}

/// The name of an XInput battery type (`XINPUT_BATTERY_TYPE`).
pub fn battery_type_name(kind: u8) -> &'static str {
    match kind {
        2 => "standard",
        3 => "alkaline",
        4 => "nimh",
        _ => "unknown",
    }
}

/// The buttons a mask holds, by name.
pub fn xinput_buttons(mask: u16) -> Vec<&'static str> {
    XINPUT_BUTTONS.iter().filter(|(_, bit)| mask & bit != 0).map(|(id, _)| *id).collect()
}

/// True when the packet counter moved and nothing the report carries changed.
///
/// This is the signature of a back paddle or a fast bumper: the pad reports the press over the
/// XUSB link, but its XInput driver drops it from every field and only counts it (research 2.2d:
/// `packet_watch.py` and `xusb_raw.py`). It cannot say press from release, nor which of the four
/// buttons it was, and a face-button tap that starts and ends between two samples looks the
/// same.
///
/// **It is not a paddle on its own.** The pad's own heartbeat moves the counter steadily while
/// nothing is pressed: the 2026-10-27 runs show the same ~31-step rhythm with and without a
/// paddle pressed (research 2.2d's own +100-per-press never showed up on this pad). A step is
/// only worth calling a paddle when it breaks the rhythm measured over the idle window; see
/// [`IdleCadence`].
pub fn extra_button_moved(prev: &XState, now: &XState) -> bool {
    now.packet != prev.packet && !state_changed_but_not_the_counter(prev, now)
}

/// True when two states differ in anything but the packet counter.
pub fn state_changed_but_not_the_counter(prev: &XState, now: &XState) -> bool {
    XState { packet: 0, ..*now } != XState { packet: 0, ..*prev }
}

/// One sample of the XInput state, and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// The state the API reported.
    pub state: XState,
    /// True when the sample came from the idle window: the first seconds of a run, when nothing
    /// is pressed and the pad's own heartbeat is all that moves.
    pub idle: bool,
}

/// The idle heartbeat: how the packet counter moves while nothing is pressed.
///
/// The 2026-10-27 runs (the pad on the desk, nothing pressed) show the counter taking steps of
/// roughly 30 packets every poll, steadily, with the odd shorter or longer one. A press that
/// moves the counter much further than that is what a paddle looks like; the research's own
/// capture of a paddle press on this pad showed a step of +100 (research 2.2d).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IdleCadence {
    /// How many steps were measured.
    pub samples: u32,
    /// The smallest step measured.
    pub min: u32,
    /// The largest step measured.
    pub max: u32,
    /// The middle step measured.
    pub median: u32,
}

impl IdleCadence {
    /// The step size a packet-only move has to exceed to count as a paddle candidate.
    ///
    /// Twice the middle step, and at least twenty over the biggest one: the heartbeat's steps
    /// are noisy, so a candidate has to stand well clear of them. The vendor's own capture of a
    /// paddle press on this pad showed a step of +100 (research 2.2d), far outside the heartbeat.
    pub fn candidate_threshold(&self) -> Option<u32> {
        if self.samples == 0 {
            return None;
        }
        Some((self.median * 2).max(self.max + 20))
    }

    /// True when a step of `delta` breaks the idle rhythm.
    pub fn breaks(&self, delta: u32) -> bool {
        self.candidate_threshold().is_some_and(|threshold| delta > threshold)
    }
}

/// The cadence of a set of steps: how far the counter moved each time.
pub fn idle_cadence(steps: &[u32]) -> IdleCadence {
    let mut sorted: Vec<u32> = steps.iter().copied().filter(|s| *s > 0).collect();
    if sorted.is_empty() {
        return IdleCadence::default();
    }
    sorted.sort_unstable();
    IdleCadence {
        samples: sorted.len() as u32,
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        median: sorted[sorted.len() / 2],
    }
}

/// What a run of XInput samples showed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XInputRun {
    /// The pad's own heartbeat, measured over the idle window.
    pub cadence: IdleCadence,
    /// The buttons the run showed, by name.
    pub buttons: Vec<&'static str>,
    /// The largest left trigger value the run showed.
    pub left_trigger: u8,
    /// The largest right trigger value the run showed.
    pub right_trigger: u8,
    /// How often the packet counter moved and nothing else did.
    pub packet_only_moves: u32,
    /// The packet-only moves that broke the idle rhythm: the only thing that could be a paddle.
    pub candidates: Vec<u32>,
}

/// What a run of XInput samples says.
///
/// The counter-only moves are counted, but they are the pad's heartbeat until they break the
/// rhythm measured over the idle window: only then are they candidates.
pub fn analyse_run(samples: &[Sample]) -> XInputRun {
    let mut run = XInputRun::default();
    let mut buttons: Vec<&'static str> = Vec::new();
    let mut idle_steps: Vec<u32> = Vec::new();
    let mut later_steps: Vec<u32> = Vec::new();
    for sample in samples {
        for name in xinput_buttons(sample.state.buttons) {
            if !buttons.contains(&name) {
                buttons.push(name);
            }
        }
        run.left_trigger = run.left_trigger.max(sample.state.lt);
        run.right_trigger = run.right_trigger.max(sample.state.rt);
    }
    for pair in samples.windows(2) {
        let (prev, now) = (&pair[0], &pair[1]);
        let delta = now.state.packet.wrapping_sub(prev.state.packet);
        if delta == 0 {
            continue;
        }
        if state_changed_but_not_the_counter(&prev.state, &now.state) {
            // Something the pad's driver does report moved: not a paddle signature.
            continue;
        }
        run.packet_only_moves += 1;
        if now.idle {
            idle_steps.push(delta);
        } else {
            later_steps.push(delta);
        }
    }
    run.cadence = idle_cadence(&idle_steps);
    run.buttons = buttons;
    run.candidates = later_steps.into_iter().filter(|d| run.cadence.breaks(*d)).collect();
    run
}

/// The DInput input report, as far as the research names its fields (section 7.1, the SDL driver).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DInputReport {
    /// The hat switch, 0..7 with 8 for centred.
    pub hat: u8,
    /// The buttons of byte 8: A (`0x01`), B (`0x02`), X (`0x08`), Y (`0x10`), L1 (`0x40`), R1
    /// (`0x80`), the right paddle (`0x04`) and the left paddle (`0x20`).
    pub buttons: u8,
    /// The buttons of byte 9: Home (`0x10`), Back (`0x04`), Start (`0x08`), L3 (`0x20`), R3
    /// (`0x40`).
    pub buttons2: u8,
    /// The buttons of byte 10: the fast bumpers L4 (`0x01`) and R4 (`0x02`).
    pub buttons3: u8,
    /// The battery in percent, when the report is long enough to carry it.
    pub battery: Option<u8>,
    /// The pad is charging, when the report is long enough to say so.
    pub charging: bool,
}

impl DInputReport {
    /// The left paddle (PL) is pressed.
    pub fn left_paddle(&self) -> bool {
        self.buttons & 0x20 != 0
    }

    /// The right paddle (PR) is pressed.
    pub fn right_paddle(&self) -> bool {
        self.buttons & 0x04 != 0
    }

    /// The fast bumper L4 is pressed.
    pub fn l4(&self) -> bool {
        self.buttons3 & 0x01 != 0
    }

    /// The fast bumper R4 is pressed.
    pub fn r4(&self) -> bool {
        self.buttons3 & 0x02 != 0
    }
}

/// Decode one DInput input report.
///
/// The report carries the report id in byte 0, the hat in byte 1, the sticks in bytes 2..5, the
/// triggers in bytes 6 and 7, the buttons in bytes 8..10 and the battery in byte 14 (bit 7
/// charging, the low seven bits the percent). A report that is too short for the fields it
/// carries is refused.
pub fn decode_dinput(report: &[u8]) -> Result<DInputReport, String> {
    if report.len() < 11 {
        return Err(format!("the DInput report is {} bytes, not 11", report.len()));
    }
    Ok(DInputReport {
        hat: report[1],
        buttons: report[8],
        buttons2: report[9],
        buttons3: report[10],
        battery: (report.len() > 14).then(|| report[14] & 0x7F),
        charging: report.len() > 14 && report[14] & 0x80 != 0,
    })
}

// ------------------------------------------------------------------------------------- the evidence

/// The bytes the research recorded, kept here so this repository holds one copy of them.
///
/// The vendor app's own write log (`raw/8bitdo/trace/V1.32_1.txt.decoded.txt`) line 4 is the
/// identify request, and the 2026-10-17 capture
/// (`raw/8bitdo/paddle_poll_20261017-154438.log`) holds the replies the vendor app got.
pub mod evidence {
    /// The vendor app's identify request: `81 05 00 21 01`, then zeros.
    pub const IDENTIFY_REQUEST: &[u8] = &[
        0x81, 0x05, 0x00, 0x21, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    /// The reply the vendor app got to its identify request: `02 22 65 00`, then zeros.
    pub const IDENTIFY_REPLY: &[u8] = &[
        0x02, 0x22, 0x65, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    /// The `Reportstate(1)` packet the vendor app sends on connect and on exit:
    /// `81 11 04 07 00 01`, then zeros (the trace's line 6).
    pub const REPORT_STATE_1: &[u8] = &[
        0x81, 0x11, 0x04, 0x07, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    /// The reply to the profile read's first chunk on the test pad, 2026-10-27 (the XInput
    /// personality, `2DC8:310B`), copied out of the hardware log `.build/s2/hw-pad-read-2.txt`.
    ///
    /// It is 63 bytes: the 18-byte header and the 45 bytes of the image's first chunk. The CRC
    /// the pad answers with (`0x4f5b`) is the CRC of the 45 `0xCC` filler bytes our own read
    /// request carried, not a CRC of the reply: the XInput personality does not check the chunk
    /// data (research 5.3 lists only the CRC-capable personalities).
    pub const REPLY_CHUNK_0: &[u8] = &[
        0x02, 0x04, 0x04, 0x00, 0x02, 0x00, 0x2d, 0x00, 0x5b, 0x4f, 0x38, 0x06, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x11, 0x09, 0x20, 0x20, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0x03, 0x00, 0x00, 0x00, 0x00, 0x63, 0x00, 0x68, 0x00, 0x75, 0x00, 0x6d, 0x00, 0x62,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
}

/// A made-up profile image, for the tests: profile 1 and profile 3 marked in use, profile 2 not,
/// the active profile 3. The pad has never been read, so this is the only shape the tests can
/// use; it is built from the record layout, not from a device.
pub fn synthetic_image() -> Vec<u8> {
    let mut img = vec![0u8; IMAGE_LEN];
    for (n, flag) in [ENABLED, 0x2019_0000, ENABLED].into_iter().enumerate() {
        img[FLAGS + 4 * n..FLAGS + 4 * n + 4].copy_from_slice(&flag.to_le_bytes());
    }
    img[MODE] = 3; // XInput
    img[CUR_SLOT] = 2;
    for (n, name) in ["Racing", "Unused", "FPS"].into_iter().enumerate() {
        let at = NAME + NAME_LEN * n;
        img[at..at + name.len()].copy_from_slice(name.as_bytes());
    }
    // The joystick and trigger records: profiles 1 and 3 in use, with distinct bytes.
    for (base, bytes) in [
        (JOYSTICK, [[10u8, 120, 11, 121], [0, 0, 0, 0], [20, 130, 21, 131]]),
        (TRIGGER, [[5, 200, 6, 201], [0, 0, 0, 0], [30, 250, 31, 251]]),
    ] {
        for (n, b) in bytes.iter().enumerate() {
            let at = base + RANGE_RECORD_LEN * n;
            let flag = if n == 1 { 0 } else { ENABLED };
            img[at..at + 4].copy_from_slice(&flag.to_le_bytes());
            img[at + 4..at + 8].copy_from_slice(b);
        }
    }
    // The rumble records: profile 1 in use with both motors at 0.8 and 0.4.
    for (n, (left, right)) in [(0.8f32, 0.4f32), (0.0, 0.0), (1.0, 0.6)].into_iter().enumerate() {
        let at = RUMBLE + RUMBLE_RECORD_LEN * n;
        let flag = if n == 1 { 0 } else { ENABLED };
        img[at..at + 4].copy_from_slice(&flag.to_le_bytes());
        img[at + 4..at + 8].copy_from_slice(&left.to_le_bytes());
        img[at + 8..at + 12].copy_from_slice(&right.to_le_bytes());
    }
    // The button map: profile 1 in use; the four extra slots hold A, none, Start and a mask the
    // research does not name (a stick direction, which belongs to another product).
    let map = MAP_KEY;
    img[map..map + 4].copy_from_slice(&ENABLED.to_le_bytes());
    for (slot, mask) in [(18, 0x2000u32), (19, 0), (20, 0x1), (21, 0x0800_0001)] {
        img[map + 4 + 4 * slot..map + 8 + 4 * slot].copy_from_slice(&mask.to_le_bytes());
    }
    img
}

/// A reply frame for a chunk, the way the pad's vendor channel answers a read request.
pub fn reply_frame(len: usize, crc_ok: bool, offset: u32, data: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; REPORT_LEN];
    f[0] = IN_ID;
    f[1] = 0x04;
    f[2..4].copy_from_slice(&4u16.to_le_bytes());
    f[4..6].copy_from_slice(&REQ_READ.to_le_bytes());
    let crc = if crc_ok { crc16(data) } else { crc16(data) ^ 0x0101 };
    f[6..10].copy_from_slice(&(len as u32 | (u32::from(crc) << 16)).to_le_bytes());
    f[10..14].copy_from_slice(&(IMAGE_LEN as u32).to_le_bytes());
    f[14..18].copy_from_slice(&offset.to_le_bytes());
    f[DATA_AT..DATA_AT + len].copy_from_slice(data);
    f
}

/// The bytes of a frame as a hex string, for the logs.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for b in bytes {
        let _ = write!(out, "{b:02x} ");
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::evidence::*;
    use super::*;

    fn hex_bytes(s: &str) -> Vec<u8> {
        let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        assert!(clean.len().is_multiple_of(2), "odd number of hex digits in {s:?}");
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    // ------------------------------------------------------------------------------- the CRC

    #[test]
    fn the_crc_is_crc16_modbus() {
        // The catalogue check value of CRC-16/MODBUS (init FFFF, poly A001 reflected, no final
        // xor): "123456789" -> 0x4B37.
        assert_eq!(crc16(b"123456789"), 0x4B37);
        assert_eq!(crc16(&[]), 0xFFFF);
    }

    // -------------------------------------------------------------------------- the request layout

    #[test]
    fn the_read_request_has_the_layout_the_research_script_sends() {
        // Hand-assembled from profile_read.py: 81 04 | u16 2 | u16 0 | u32 (len | crc << 16) |
        // u32 0x638 | u32 offset | 0xCC filler.
        let r = read_request(0).unwrap();
        assert_eq!(r.len(), REPORT_LEN);
        let crc = crc16(&[READ_FILL; CHUNK]);
        let mut want = vec![0x81, 0x04, 0x02, 0x00, 0x00, 0x00];
        want.extend_from_slice(&(CHUNK as u32 | (u32::from(crc) << 16)).to_le_bytes());
        want.extend_from_slice(&(IMAGE_LEN as u32).to_le_bytes());
        want.extend_from_slice(&0u32.to_le_bytes());
        want.extend(std::iter::repeat_n(READ_FILL, CHUNK));
        want.resize(REPORT_LEN, 0);
        assert_eq!(r, want);
        // The declared command in the vendor's own device model is the head of every read request.
        assert_eq!(&r[1..6], hex_bytes("0402000000"));
    }

    #[test]
    fn thirty_six_requests_cover_the_image() {
        // 35 chunks of 45 bytes and one of 17.
        let mut at = 0;
        let mut n = 0;
        while at < IMAGE_LEN {
            let req = read_request(at).unwrap();
            let len = (u32::from_le_bytes(req[6..10].try_into().unwrap()) & 0xFFFF) as usize;
            assert_eq!(u32::from_le_bytes(req[14..18].try_into().unwrap()) as usize, at);
            assert_eq!(req.len(), REPORT_LEN);
            at += len;
            n += 1;
        }
        assert_eq!((n, at), (36, IMAGE_LEN));
        assert_eq!(IMAGE_LEN, 1592);
        assert!(read_request(IMAGE_LEN).is_err());
    }

    #[test]
    fn the_identify_request_is_the_one_the_vendor_app_sent() {
        assert_eq!(identify_request(), IDENTIFY_REQUEST);
        assert_eq!(parse_identify_reply(IDENTIFY_REPLY), Ok(0x65));
        // The connect-time Reportstate is the trace's line 6 (length form).
        assert_eq!(report_state_connect_request(1), REPORT_STATE_1);
    }

    #[test]
    fn the_commit_is_the_no_length_form_of_research_5_2() {
        // `81 04 06 00 23 01` + zeros: no length byte, request word 6, argument 0x0123.
        let c = commit_request();
        assert_eq!(&c[..6], &hex_bytes("810406002301")[..]);
        assert!(c[6..].iter().all(|b| *b == 0));
        assert_eq!(c.len(), REPORT_LEN);
        // The old length-byte form is only the connect-time shape.
        assert_ne!(c[1], 0x11);
    }

    #[test]
    fn reportstate_in_a_session_is_the_no_length_form() {
        let on = report_state_request(1);
        assert_eq!(&on[..6], &hex_bytes("810407000100")[..]);
        let off = report_state_request(0);
        assert_eq!(&off[..6], &hex_bytes("810407000000")[..]);
        assert!(on[6..].iter().all(|b| *b == 0) && on.len() == REPORT_LEN);
    }

    #[test]
    fn an_acknowledgement_is_accepted_only_for_the_request_it_answers() {
        assert_eq!(parse_ack_reply(&ack_frame(1), 1), Ok(()));
        assert_eq!(parse_ack_reply(&ack_frame(6), 6), Ok(()));
        assert!(parse_ack_reply(&ack_frame(2), 1).unwrap_err().contains("request word 2"));
        assert!(parse_ack_reply(&ack_frame(1), 6).is_err());
        let mut bad = ack_frame(1);
        bad[0] = 0x03;
        assert!(parse_ack_reply(&bad, 1).is_err());
        let mut bad = ack_frame(1);
        bad[1] = 0x22;
        assert!(parse_ack_reply(&bad, 1).is_err());
        let mut bad = ack_frame(1);
        bad[2] = 0;
        assert!(parse_ack_reply(&bad, 1).is_err());
        assert!(parse_ack_reply(&[0x02, 0x04], 1).is_err());
    }

    // -------------------------------------------------------------------------- the reply layout

    #[test]
    fn the_real_chunk_zero_reply_is_accepted_because_it_echoes_our_own_crc() {
        // The test pad in the XInput personality answers the read with the CRC our request
        // carried: the CRC of the 45 bytes of 0xCC filler, not a CRC of the reply.
        assert_eq!(crc16(&[READ_FILL; 45]), 0x4f5b, "the CRC the pad echoed");
        let reply = evidence::REPLY_CHUNK_0;
        assert_eq!(reply.len(), 64, "the header, the first 45 bytes of the image and a filler byte");
        let data = parse_read_reply(reply, 45, 0).expect("the real reply is accepted");
        assert_eq!(data.len(), 45);

        // The image's first 45 bytes: the profile flags, the CRC field, the mode, the active
        // profile and the start of the profile name.
        assert_eq!(&data[0..4], &ENABLED.to_le_bytes(), "profile 1 is in use");
        assert_eq!(&data[4..12], &[0xff; 8], "the two other profiles are erased flash");
        assert_eq!(&data[12..16], &[0xff, 0xff, 0xff, 0xff], "the image carries no CRC");
        assert_eq!(u16::from_le_bytes([data[16], data[17]]), 3, "the pad is in XInput mode");
        assert_eq!(u16::from_le_bytes([data[18], data[19]]), 0, "profile 1 is the active one");
        // The profile name field starts at 0x14 and holds "chumb" as UTF-16 **big endian**: the
        // bytes are 00 63 00 68 00 75 00 6d 00 62 00. The codec's name reader only reads UTF-8,
        // so it reports no name; the names are not shown in the product anyway (the property
        // model has no text kind), and the record is here so the finding is not lost.
        let name: Vec<u8> = "chumb".encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
        assert_eq!(&data[20..30], &name[..], "the profile name is \"chumb\"");
    }

    #[test]
    fn a_reply_whose_crc_is_neither_the_data_nor_the_echo_is_refused() {
        let mut reply = evidence::REPLY_CHUNK_0.to_vec();
        reply[8] ^= 0x01;
        let err = parse_read_reply(&reply, 45, 0).unwrap_err();
        assert!(err.contains("does not match the data"), "{err}");
    }

    #[test]
    fn the_crc_capable_personalities_still_check_the_chunk_data() {
        // The other personalities answer with a CRC of their own data (research 5.3).
        let data: Vec<u8> = (0..45u8).collect();
        let reply = reply_frame(45, true, 0, &data);
        assert_eq!(parse_read_reply(&reply, 45, 0).unwrap(), &data[..]);
        let broken = reply_frame(45, false, 0, &data);
        assert!(parse_read_reply(&broken, 45, 0).is_err());
    }

    #[test]
    fn a_reply_is_accepted_only_when_every_field_agrees() {
        let data: Vec<u8> = (0..45u8).collect();
        let good = |f: &dyn Fn(&mut Vec<u8>)| {
            let mut r = reply_frame(45, true, 0x2D, &data);
            f(&mut r);
            r
        };
        assert_eq!(parse_read_reply(&good(&|_| {}), 45, 0x2D).unwrap(), &data[..]);
        for (what, f) in [
            ("report id", &(|r: &mut Vec<u8>| r[0] = 1) as &dyn Fn(&mut Vec<u8>)),
            ("class byte", &|r| r[1] = 5),
            ("header copy", &|r| r[2] = 5),
            ("request word", &|r| r[4] = 6),
            ("length", &|r| r[6] = 44),
            ("crc", &|r| r[8] ^= 1),
            ("total", &|r| r[10] = 0x37),
            ("offset", &|r| r[15] ^= 1),
            ("data", &|r| r[30] ^= 1),
            ("truncated", &|r| r.truncate(40)),
        ] {
            assert!(parse_read_reply(&good(f), 45, 0x2D).is_err(), "{what}");
        }
    }

    #[test]
    fn a_reply_naming_another_offset_is_refused_and_the_right_one_accepted() {
        let data: Vec<u8> = (0..45u8).collect();
        let asked_at = 0x2D;
        let wrong = reply_frame(45, true, 0, &data);
        assert!(parse_read_reply(&wrong, 45, asked_at).is_err());
        let right = reply_frame(45, true, asked_at as u32, &data);
        assert_eq!(parse_read_reply(&right, 45, asked_at).unwrap(), &data[..]);
        let mut wrong_total = reply_frame(45, true, asked_at as u32, &data);
        wrong_total[10] = 0x37;
        assert!(parse_read_reply(&wrong_total, 45, asked_at).is_err());    }

    // ---------------------------------------------------------------------------- the image layout

    #[test]
    fn the_record_offsets_are_the_cumulative_sizes_and_the_image_is_0x638() {
        assert_eq!(LAYOUT.iter().map(|(_, s)| s).sum::<usize>(), IMAGE_LEN);
        for (name, constant) in [
            ("flag[3]", FLAGS),
            ("gamepad_mode", MODE),
            ("cur_slot", CUR_SLOT),
            ("file_name[3]", NAME),
            ("joystick[3]", JOYSTICK),
            ("trigger[3]", TRIGGER),
            ("map_key[3]", MAP_KEY),
            ("hotwheel[3]", HOTWHEEL),
        ] {
            assert_eq!(offset_of(name), Some(constant), "{name}");
        }
        // The research table (5.4) agrees on every offset it lists.
        for (name, want) in [
            ("crc_value", 0x0C),
            ("rumble[3]", 0x74),
            ("special_feature[3]", 0xC8),
            ("record_macro_fun[3]", 0x1F4),
            ("x_rumble[3]", 0x47C),
            ("sixaxis[3]", 0x494),
            ("dynamic[3]", 0x4B8),
            ("single[3]", 0x50C),
        ] {
            assert_eq!(offset_of(name), Some(want), "{name}");
        }
    }

    #[test]
    fn the_image_decodes_to_the_values_written_into_it() {
        let img = synthetic_image();
        assert_eq!(active_profile(&img), Some(2));
        assert_eq!(stored_mode(&img), Some("XInput"));
        let p1 = decode_profile(&img, 0).unwrap();
        assert!(p1.in_use);
        assert_eq!(p1.name.as_deref(), Some("Racing"));
        assert_eq!(p1.stick, [10, 120, 11, 121]);
        assert_eq!(p1.trigger, [5, 200, 6, 201]);
        assert_eq!(p1.vibration, [0.8, 0.4]);
        assert!(p1.stick_in_use && p1.trigger_in_use && p1.rumble_in_use && p1.map_in_use);
        assert_eq!(button_target(p1.map[EXTRA_BUTTON_FIRST]), "a");
        assert_eq!(button_target(p1.map[EXTRA_BUTTON_FIRST + 1]), "none");
        assert_eq!(button_target(p1.map[EXTRA_BUTTON_FIRST + 2]), "start");
        assert_eq!(button_target(p1.map[EXTRA_BUTTON_FIRST + 3]), OTHER_TARGET);
        let p2 = decode_profile(&img, 1).unwrap();
        assert!(!p2.in_use);
        assert!(!p2.stick_in_use && !p2.trigger_in_use && !p2.rumble_in_use && !p2.map_in_use);
        let p3 = decode_profile(&img, 2).unwrap();
        assert!(p3.in_use);
        assert_eq!(p3.name.as_deref(), Some("FPS"));
        assert_eq!(p3.stick, [20, 130, 21, 131]);
        // A wrong size is an error, not a guess.
        assert!(decode_profile(&img[..IMAGE_LEN - 1], 0).is_err());
        assert!(decode_profile(&img, 3).is_err());
        // The names are only taken from plain UTF-8.
        let mut odd = img.clone();
        odd[NAME] = 0xFF;
        assert_eq!(decode_profile(&odd, 0).unwrap().name, None);
    }

    #[test]
    fn the_percentages_round_trip_through_the_stored_byte() {
        // A hundred percent is a hundred steps over 256 bytes, so a byte does not always come
        // back to itself: the percent it decodes to is the one the editor shows, and the write
        // path keeps the byte when the value written is the one it read.
        for byte in [0u8, 1, 17, 128, 200, 254, 255] {
            let percent = byte_to_percent(byte);
            assert!((0.0..=100.0).contains(&percent), "{byte} is {percent}%");
            let back = percent_to_byte(percent);
            assert!(
                (i32::from(back) - i32::from(byte)).abs() <= 1,
                "{byte} came back as {back}"
            );
        }
        // The bytes that stand for a whole percent come back exactly.
        for byte in [0u8, 51, 102, 128, 153, 204, 255] {
            assert_eq!(percent_to_byte(byte_to_percent(byte)), byte, "{byte}");
        }
        assert_eq!(byte_to_percent(255), 100.0);
        assert_eq!(byte_to_percent(0), 0.0);
        assert_eq!(byte_to_percent(128), 50.0);
    }

    #[test]
    fn the_unchanged_value_write_returns_the_record_exactly_as_read() {
        let img = synthetic_image();
        for (suffix, _, _) in RANGE_ROWS {
            for profile in [0usize, 2] {
                let key = range_key(profile, suffix);
                let f = field_of(&key).unwrap();
                let current = img[f.record_at + f.byte_in_record];
                let percent = byte_to_percent(current);
                let (at, rec) = patched_record(&img, &key, percent).unwrap();
                assert_eq!(&img[at..at + RANGE_RECORD_LEN], &rec[..], "{key}: same bytes back");
                // The request that carries it: offset, length, CRC and data, and it is a request-1
                // write.
                let req = write_request(at, &rec).unwrap();
                assert_eq!(&req[1..4], hex_bytes("040100"));
                assert_eq!(u32::from_le_bytes(req[14..18].try_into().unwrap()) as usize, at);
                assert_eq!(u32::from_le_bytes(req[10..14].try_into().unwrap()), IMAGE_LEN as u32);
                let word = u32::from_le_bytes(req[6..10].try_into().unwrap());
                assert_eq!((word & 0xFFFF) as usize, RANGE_RECORD_LEN);
                assert_eq!((word >> 16) as u16, crc16(&rec));
                assert_eq!(&req[DATA_AT..DATA_AT + RANGE_RECORD_LEN], &rec[..]);
                assert!(req[DATA_AT + RANGE_RECORD_LEN..].iter().all(|b| *b == 0));
            }
        }
        // Out of range and unwritable rows are refused; a record not marked in use is never written.
        assert!(patched_record(&img, &range_key(0, "stick-left-start"), 101.0).is_err());
        assert!(patched_record(&img, &range_key(0, "stick-left-start"), -1.0).is_err());
        assert!(patched_record(&img, &range_key(0, "ring-colour"), 1.0).is_err());
        assert!(patched_record(&img, &range_key(1, "stick-left-start"), 1.0).is_err(), "profile 2 is not in use");
    }

    #[test]
    fn a_changed_value_moves_exactly_one_byte() {
        let img = synthetic_image();
        let key = range_key(0, "stick-left-start");
        let (at, rec) = patched_record(&img, &key, 50.0).unwrap();
        let before = &img[at..at + RANGE_RECORD_LEN];
        assert_eq!(rec.iter().zip(before).filter(|(a, b)| a != b).count(), 1);
        assert_eq!(rec[4], percent_to_byte(50.0));
    }

    #[test]
    fn the_button_masks_decode_to_the_named_targets() {
        for (mask, id) in [
            (0u32, "none"),
            (0x2000, "a"),
            (0x1000, "b"),
            (0x10, "x"),
            (0x20, "y"),
            (0x400, "left-bumper"),
            (0x800, "right-bumper"),
            (0x4000, "left-trigger"),
            (0x8000, "right-trigger"),
            (0x10000, "menu"),
            (0x20000, "home"),
            (0x2, "left-stick-click"),
            (0x4, "right-stick-click"),
            (0x8, "select"),
            (0x1, "start"),
            (0x200, "dpad-up"),
            (0x100, "dpad-down"),
            (0x80, "dpad-left"),
            (0x40, "dpad-right"),
            (0x200000, "paddle-3"),
            (0x400000, "screenshot"),
            (0x800000, "turbo-single"),
            (0x1000000, "turbo-auto"),
            (0x2000000, "paddle-1"),
            (0x4000000, "paddle-2"),
            (0x40000000, "paddle-4"),
            (0x8000000, "swap"),
        ] {
            assert_eq!(button_target(mask), id, "{mask:#x}");
            assert_eq!(target_mask(id), Some(mask), "{id}");
        }
        // The masks that belong to other products, and the erased flash, are not named: the
        // stick directions and `Record` are HitBox-only and store zero on the Ultimate 2, and
        // `BT_CON` is named by neither source in words this build can use.
        for mask in [
            0x40000u32,
            0x80000,
            0x100000,
            0x8000010,
            0x8000020,
            0x8000040,
            0x8000080,
            0x10000000,
            0x20000000,
            0x8000001,
            0x8000002,
            0x8000004,
            0x8000008,
            0xffff_ffff,
        ] {
            assert_eq!(button_target(mask), OTHER_TARGET, "{mask:#x}");
        }
        assert_eq!(target_mask(OTHER_TARGET), None, "\"other\" is not a target");
        assert_eq!(target_options().len(), TARGETS.len() + 1);
    }

    #[test]
    fn every_code_of_the_captured_map_is_a_named_target() {
        // The captured profile 1 map, as the hardware read decoded it on 2026-10-27: the identity map
        // the vendor app writes for the XInput personality, with R2, Menu and Home holding the
        // masks the research's table names.
        let mut image = synthetic_image();
        let map = MAP_KEY;
        for (slot, mask) in [
            (0usize, 0x2000u32),
            (1, 0x1000),
            (2, 0x10),
            (3, 0x20),
            (4, 0x400),
            (5, 0x800),
            (6, 0x4000),
            (7, 0x8000),
            (8, 0x2),
            (9, 0x4),
            (10, 0x8),
            (11, 0x1),
            (12, 0x10000),
            (13, 0x20000),
            (14, 0x200),
            (15, 0x100),
            (16, 0x80),
            (17, 0x40),
            (18, 0),
            (19, 0),
            (20, 0),
            (21, 0),
        ] {
            let at = map + 4 + 4 * slot;
            image[at..at + 4].copy_from_slice(&mask.to_le_bytes());
        }
        let profile = decode_profile(&image, 0).unwrap();
        for (slot, mask) in profile.map.iter().enumerate() {
            assert_ne!(
                button_target(*mask),
                OTHER_TARGET,
                "slot {slot} holds {mask:#x}: every code of the captured map is a named target"
            );
        }
        // The four paddles of that profile are unmapped.
        for slot in EXTRA_BUTTON_FIRST..MAP_SLOTS {
            assert_eq!(button_target(profile.map[slot]), "none", "slot {slot}");
        }
    }

    #[test]
    fn a_map_entry_is_patched_in_place_and_nothing_else_moves() {
        let img = synthetic_image();
        let (at, entry) = patched_map_entry(&img, 2, 18, "a").unwrap();
        assert_eq!(at, map_entry_offset(2, 18).unwrap());
        assert_eq!(entry, vec![0x00, 0x20, 0x00, 0x00], "the mask of A, little endian");
        assert_eq!(entry.len(), 4);
        // Every other byte of the image stays as it was.
        let mut patched = img.clone();
        patched[at..at + 4].copy_from_slice(&entry);
        let moved: Vec<usize> = (0..img.len()).filter(|i| img[*i] != patched[*i]).collect();
        assert!(
            moved.iter().all(|i| (at..at + 4).contains(i)),
            "only the entry moved: {moved:?}"
        );
        assert_eq!(&patched[at..at + 4], &entry[..]);
        // "none" is the mask zero.
        let (_, none) = patched_map_entry(&img, 2, 18, "none").unwrap();
        assert_eq!(none, vec![0, 0, 0, 0]);
        // A target this build does not name is refused.
        assert!(patched_map_entry(&img, 2, 18, "other").is_err());
        assert!(patched_map_entry(&img, 3, 18, "a").is_err());
        assert!(patched_map_entry(&img, 2, 22, "a").is_err());
        assert!(patched_map_entry(&img[..10], 2, 18, "a").is_err());
    }

    // ------------------------------------------------------------------------------ the XInput view

    #[test]
    fn an_extra_button_shows_only_as_the_packet_counter_moving() {
        let idle = XState { packet: 9010, ..XState::default() };
        assert!(extra_button_moved(&idle, &XState { packet: 9110, ..idle }));
        assert!(extra_button_moved(&idle, &XState { packet: idle.packet.wrapping_add(1), ..idle }));
        assert!(!extra_button_moved(&idle, &idle));
        for changed in [
            XState { buttons: 0x1000, ..idle },
            XState { lt: 1, ..idle },
            XState { rt: 1, ..idle },
            XState { lx: 1, ..idle },
            XState { ly: -1, ..idle },
            XState { rx: 1, ..idle },
            XState { ry: 1, ..idle },
        ] {
            assert!(!extra_button_moved(&idle, &XState { packet: 9110, ..changed }), "{changed:?}");
        }
        let top = XState { packet: u32::MAX, ..XState::default() };
        assert!(extra_button_moved(&top, &XState { packet: 99, ..top }));
    }

    #[test]
    fn the_xinput_buttons_are_named_by_their_bits() {
        assert_eq!(xinput_buttons(0x1000), vec!["a"]);
        assert_eq!(xinput_buttons(0x1021), vec!["dpad-up", "back", "a"]);
        assert!(xinput_buttons(0).is_empty());
    }

    // --------------------------------------------------------------------------- the DInput report

    #[test]
    fn the_dinput_report_carries_the_paddles_the_bumpers_and_the_battery() {
        // The layout the SDL driver reads (research 7.1).
        let mut report = vec![0u8; 34];
        report[0] = 0x04;
        report[1] = 0; // hat centred
        report[8] = 0x20; // PL
        report[10] = 0x03; // L4 and R4
        report[14] = 0x80 | 55; // charging, 55 %
        let d = decode_dinput(&report).unwrap();
        assert!(d.left_paddle());
        assert!(!d.right_paddle());
        assert!(d.l4() && d.r4());
        assert_eq!(d.battery, Some(55));
        assert!(d.charging);
        // A short report carries no battery.
        let mut short = report.clone();
        short.truncate(12);
        let d = decode_dinput(&short).unwrap();
        assert_eq!(d.battery, None);
        assert!(!d.charging);
        // Too short for the fields it names is an error.
        assert!(decode_dinput(&[0x04, 0, 0]).is_err());
    }

    // ---------------------------------------------------------------------------- the image CRC

    #[test]
    fn the_image_crc_candidates_are_computed_over_every_range_the_research_allows() {
        let img = synthetic_image();
        let crc = image_crc(&img).unwrap();
        assert_eq!(crc.stored, u32::from_le_bytes(img[0x0C..0x10].try_into().unwrap()));
        assert_eq!(crc.whole, crc16(&img));
        let mut zeroed = img.clone();
        zeroed[0x0C..0x10].copy_from_slice(&[0, 0, 0, 0]);
        assert_eq!(crc.zeroed, crc16(&zeroed));
        let mut without = img[..0x0C].to_vec();
        without.extend_from_slice(&img[0x10..]);
        assert_eq!(crc.without_field, crc16(&without));
        assert_eq!(crc.after_field, crc16(&img[0x10..]));
        // The synthetic image stores no CRC of its own, so nothing matches.
        assert!(crc.matching().is_none());
        assert!(image_crc(&img[..10]).is_err(), "a short image is not an image");
    }

    #[test]
    fn the_real_image_carries_no_crc_of_its_own() {
        // The captured image, chunk 0: the value at offset 0x0C is the erased-flash pattern, the same
        // value the two unused profile flags hold. It is not a CRC: the vendor's CRC is a
        // `ushort`, so a stored value with bits above the low half set is something else. The
        // image was never CRC'd by the vendor app, so no range can match it.
        let data = &evidence::REPLY_CHUNK_0[DATA_AT..];
        let stored = u32::from_le_bytes(data[12..16].try_into().unwrap());
        assert_eq!(stored, 0xffff_ffff);
        assert_eq!(&data[4..12], &[0xff; 8], "the same value the unused profile flags hold");
        assert!(stored > u32::from(u16::MAX), "0xffffffff cannot be a CRC-16");
    }

    // ------------------------------------------------------------------------- the idle cadence

    /// The test pad on the desk, 2026-10-27, the first paddle run (`.build/s2/hw-pad-paddles-2.txt`):
    /// the packet counter, the button mask and the two triggers of every sample the probe
    /// printed. Nothing that mattered was pressed; the log holds 67 "packet moved and nothing
    /// else" lines, which are the pad's own heartbeat.
    const RUN_2: [(u32, u16, u8, u8); 90] = [
        (115, 0x1000, 0, 0),
        (146, 0x1000, 0, 0),
        (177, 0x1000, 0, 0),
        (200, 0x1000, 0, 0),
        (204, 0x0000, 0, 0),
        (236, 0x0000, 0, 0),
        (267, 0x0000, 0, 0),
        (298, 0x0000, 0, 0),
        (300, 0x0000, 0, 0),
        (322, 0x0000, 0, 0),
        (352, 0x0000, 0, 0),
        (382, 0x0000, 0, 0),
        (400, 0x0000, 0, 0),
        (430, 0x0000, 0, 0),
        (461, 0x0000, 0, 0),
        (492, 0x0000, 0, 0),
        (500, 0x0000, 0, 0),
        (503, 0x0000, 10, 0),
        (525, 0x0000, 183, 0),
        (557, 0x0000, 255, 0),
        (588, 0x0000, 255, 0),
        (619, 0x0000, 255, 0),
        (630, 0x0000, 255, 0),
        (642, 0x0000, 168, 0),
        (672, 0x0000, 0, 0),
        (704, 0x0000, 0, 0),
        (735, 0x0000, 0, 0),
        (765, 0x0000, 0, 0),
        (784, 0x0000, 0, 255),
        (816, 0x0000, 0, 255),
        (847, 0x0000, 0, 255),
        (878, 0x0000, 0, 255),
        (910, 0x0000, 0, 0),
        (941, 0x0000, 0, 0),
        (972, 0x0000, 0, 0),
        (1003, 0x0000, 0, 0),
        (1008, 0x0100, 0, 0),
        (1039, 0x0100, 0, 0),
        (1070, 0x0100, 0, 0),
        (1102, 0x0100, 0, 0),
        (1103, 0x0100, 0, 0),
        (1114, 0x0000, 0, 0),
        (1145, 0x0000, 0, 0),
        (1176, 0x0000, 0, 0),
        (1203, 0x0000, 0, 0),
        (1219, 0x0200, 0, 0),
        (1250, 0x0200, 0, 0),
        (1280, 0x0200, 0, 0),
        (1303, 0x0200, 0, 0),
        (1309, 0x0000, 0, 0),
        (1339, 0x0000, 0, 0),
        (1370, 0x0000, 0, 0),
        (1401, 0x0000, 0, 0),
        (1403, 0x0000, 0, 0),
        (1405, 0x0000, 3, 0),
        (1436, 0x0000, 255, 0),
        (1466, 0x0000, 255, 0),
        (1498, 0x0000, 255, 0),
        (1526, 0x0000, 255, 0),
        (1550, 0x0000, 66, 0),
        (1581, 0x0000, 0, 0),
        (1613, 0x0000, 0, 0),
        (1644, 0x0000, 0, 0),
        (1654, 0x0000, 0, 0),
        (1679, 0x0000, 0, 255),
        (1710, 0x0000, 0, 255),
        (1741, 0x0000, 0, 255),
        (1768, 0x0000, 0, 255),
        (1794, 0x0000, 0, 0),
        (1826, 0x0000, 0, 0),
        (1856, 0x0000, 0, 0),
        (1888, 0x0000, 0, 0),
        (1890, 0x0000, 0, 0),
        (1910, 0x0100, 0, 0),
        (1941, 0x0100, 0, 0),
        (1972, 0x0100, 0, 0),
        (1990, 0x0100, 0, 0),
        (1997, 0x0000, 0, 0),
        (2027, 0x0000, 0, 0),
        (2058, 0x0000, 0, 0),
        (2089, 0x0000, 0, 0),
        (2090, 0x0000, 0, 0),
        (2113, 0x0200, 0, 0),
        (2144, 0x0200, 0, 0),
        (2176, 0x0200, 0, 0),
        (2190, 0x0200, 0, 0),
        (2205, 0x0000, 0, 0),
        (2236, 0x0000, 0, 0),
        (2267, 0x0000, 0, 0),
        (2290, 0x0000, 0, 0),
    ];

    /// The second paddle run of the same day (`.build/s2/hw-pad-paddles-3.txt`): the tester pressed
    /// both back paddles and L4 and R4, and **nothing at all changed anywhere**. The log holds 39
    /// "packet moved and nothing else" lines: the same heartbeat as the first run.
    const RUN_3: [(u32, u16, u8, u8); 41] = [
        (2302, 0x1000, 0, 0),
        (2334, 0x1000, 0, 0),
        (2365, 0x1000, 0, 0),
        (2390, 0x1000, 0, 0),
        (2401, 0x0000, 0, 0),
        (2432, 0x0000, 0, 0),
        (2464, 0x0000, 0, 0),
        (2490, 0x0000, 0, 0),
        (2512, 0x0000, 0, 0),
        (2544, 0x0000, 0, 0),
        (2575, 0x0000, 0, 0),
        (2590, 0x0000, 0, 0),
        (2605, 0x0000, 0, 0),
        (2636, 0x0000, 0, 0),
        (2666, 0x0000, 0, 0),
        (2690, 0x0000, 0, 0),
        (2695, 0x0000, 0, 0),
        (2726, 0x0000, 0, 0),
        (2757, 0x0000, 0, 0),
        (2788, 0x0000, 0, 0),
        (2790, 0x0000, 0, 0),
        (2803, 0x0000, 0, 0),
        (2834, 0x0000, 0, 0),
        (2865, 0x0000, 0, 0),
        (2890, 0x0000, 0, 0),
        (2915, 0x0000, 0, 0),
        (2946, 0x0000, 0, 0),
        (2977, 0x0000, 0, 0),
        (2990, 0x0000, 0, 0),
        (2997, 0x0000, 0, 0),
        (3028, 0x0000, 0, 0),
        (3059, 0x0000, 0, 0),
        (3090, 0x0000, 0, 0),
        (3111, 0x0000, 0, 0),
        (3142, 0x0000, 0, 0),
        (3173, 0x0000, 0, 0),
        (3190, 0x0000, 0, 0),
        (3203, 0x0000, 0, 0),
        (3233, 0x0000, 0, 0),
        (3264, 0x0000, 0, 0),
        (3290, 0x0000, 0, 0),
    ];

    /// The samples of a run, with the idle window in front of them.
    ///
    /// The pad reports about once every 0.7 s, so the three idle seconds are the first five
    /// samples.
    fn samples_of(run: &[(u32, u16, u8, u8)]) -> Vec<Sample> {
        run.iter()
            .enumerate()
            .map(|(i, (packet, buttons, lt, rt))| Sample {
                state: XState {
                    packet: *packet,
                    buttons: *buttons,
                    lt: *lt,
                    rt: *rt,
                    ..XState::default()
                },
                idle: i < 5,
            })
            .collect()
    }

    #[test]
    fn the_first_real_run_yields_no_paddle_candidate() {
        let run = analyse_run(&samples_of(&RUN_2));
        // The 67 "packet moved and nothing else" lines of the log are the pad's heartbeat.
        assert_eq!(run.packet_only_moves, 67);
        assert!(run.candidates.is_empty(), "the heartbeat is not a paddle: {:?}", run.candidates);
        // The heartbeat: steps of about 31 packets.
        assert_eq!(run.cadence.samples, 3, "three idle steps in the first three seconds");
        assert_eq!(run.cadence.median, 31);
        assert_eq!(run.cadence.min, 23);
        assert_eq!(run.cadence.max, 31);
        let threshold = run.cadence.candidate_threshold().expect("a cadence");
        assert!(threshold > 36, "the threshold clears the biggest step of the run ({threshold})");
        // What the pad did report: A, the bumpers and the triggers.
        assert_eq!(run.buttons, vec!["a", "left-bumper", "right-bumper"]);
        assert_eq!(run.left_trigger, 255);
        assert_eq!(run.right_trigger, 255);
    }

    #[test]
    fn the_second_real_run_yields_no_paddle_candidate_either() {
        // The tester pressed both back paddles and L4 and R4: the rhythm is the same, so nothing
        // breaks it and there is no candidate.
        let run = analyse_run(&samples_of(&RUN_3));
        assert_eq!(run.packet_only_moves, 39);
        assert!(run.candidates.is_empty(), "{:?}", run.candidates);
        assert_eq!(run.cadence.median, 31);
        assert_eq!(run.buttons, vec!["a"]);
        assert_eq!(run.left_trigger, 0);
        assert_eq!(run.right_trigger, 0);
    }

    #[test]
    fn a_step_that_breaks_the_heartbeat_is_a_candidate() {
        // Research 2.2d: a paddle press on this pad moved the counter by +100.
        let mut run: Vec<(u32, u16, u8, u8)> = RUN_2[..6].to_vec();
        run.push((run[5].0 + 100, 0, 0, 0));
        let samples: Vec<Sample> = run
            .iter()
            .enumerate()
            .map(|(i, (packet, buttons, lt, rt))| Sample {
                state: XState {
                    packet: *packet,
                    buttons: *buttons,
                    lt: *lt,
                    rt: *rt,
                    ..XState::default()
                },
                idle: i < 5,
            })
            .collect();
        let analysis = analyse_run(&samples);
        assert_eq!(analysis.candidates, vec![100]);
    }

    #[test]
    fn the_cadence_of_nothing_cannot_be_broken() {
        let cadence = IdleCadence::default();
        assert!(cadence.candidate_threshold().is_none());
        assert!(!cadence.breaks(1000));
    }

    // --------------------------------------------------------------------------------- a fuzz pass

    /// SplitMix64: the generator peripheral-hub's tests use (a fixed algorithm, no platform
    /// dependence).
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

    #[test]
    fn fuzzed_replies_images_and_reports_never_panic() {
        let mut rng = SplitMix64(0x008B_17D0_2026);
        for _ in 0..100_000 {
            let n = (rng.next() % 80) as usize;
            let mut frame: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            // Half of the frames start like a real reply so the deeper checks run.
            if rng.next().is_multiple_of(2) && frame.len() > 10 {
                frame[0] = 2;
                frame[1] = if rng.next().is_multiple_of(2) { 4 } else { 0x22 };
                frame[2] = 4;
                frame[4] = 2;
            }
            let _ = parse_read_reply(&frame, (rng.next() % 60) as usize, (rng.next() % 0x638) as usize);
            let _ = parse_identify_reply(&frame);
            let _ = decode_dinput(&frame);
        }
        for _ in 0..300 {
            let img: Vec<u8> = (0..IMAGE_LEN).map(|_| rng.next() as u8).collect();
            for p in 0..PROFILES {
                let _ = decode_profile(&img, p);
            }
        }
        let mut prev = XState::default();
        for _ in 0..100_000 {
            let r = rng.next();
            let s = XState {
                packet: r as u32,
                buttons: (r >> 32) as u16,
                lt: (r >> 48) as u8,
                rt: (r >> 56) as u8,
                lx: rng.next() as i16,
                ly: rng.next() as i16,
                rx: rng.next() as i16,
                ry: rng.next() as i16,
            };
            let _ = extra_button_moved(&prev, &s);
            prev = s;
        }
    }
}
