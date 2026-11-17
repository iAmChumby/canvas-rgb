//! OpenRGB SDK wire format, shared by the client (`openrgb.rs`) and the fake server (`fake.rs`).
//!
//! Layouts follow OpenRGB's SDK as implemented by Hypercolor's `hypercolor-openrgb-sdk`
//! (Apache-2.0; see `NOTICE`). Every size field counts the bytes after the header, and the
//! controller-data size prefix counts itself, as OpenRGB does.

use canvas_proto::{AdapterError, Rgb8};

pub const HEADER_LEN: usize = 16;
pub const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
/// Highest protocol version this client asks for. OpenRGB release_1.1 speaks 6
/// (`NetworkProtocol.h:28`) and answers the lower of this and its own; an older OpenRGB answers 4 or 5
/// and the client keeps to the layouts of that version.
pub const CLIENT_MAX_PROTOCOL_VERSION: u32 = 6;
/// The first protocol version with detection events, ACKs and controller ids (#290).
pub const PROTOCOL_WITH_EVENTS: u32 = 6;
/// Oldest protocol version whose controller-data layout this codec decodes.
pub const MIN_PROTOCOL_VERSION: u32 = 4;

pub const REQUEST_CONTROLLER_COUNT: u32 = 0;
pub const REQUEST_CONTROLLER_DATA: u32 = 1;
/// Protocol 6: the server acknowledges every packet it takes from a client (`NetworkServer.cpp:3562-3580`).
pub const ACK: u32 = 10;
pub const REQUEST_PROTOCOL_VERSION: u32 = 40;
pub const SET_CLIENT_NAME: u32 = 50;
/// Protocol 6: the server names itself after the version reply (`NetworkServer.cpp:3777-3790`).
pub const SET_SERVER_NAME: u32 = 51;
pub const DEVICE_LIST_UPDATED: u32 = 100;
/// OpenRGB's detection broadcasts. Protocol 6 and later only (`NetworkServer.cpp:4003-4015`,
/// `:3963-3999`, `:3949-3961`): the server sends these to every client of that version, with no
/// request id in them (#290). 101 comes when detection starts, 102 as its progress changes, 103 when
/// it is over.
pub const DETECTION_STARTED: u32 = 101;
pub const DETECTION_PROGRESS_CHANGED: u32 = 102;
pub const DETECTION_COMPLETE: u32 = 103;
/// Client-only, protocol 5 and later, zero payload: asks OpenRGB to look at the hardware again. The
/// server answers nothing directly; it broadcasts the detection packets above (#290). Protocol 6
/// acknowledges it, and the acknowledgement says "received", never "started": 101 says started.
pub const REQUEST_RESCAN_DEVICES: u32 = 140;
pub const RESIZE_ZONE: u32 = 1000;
pub const UPDATE_LEDS: u32 = 1050;
pub const SET_CUSTOM_MODE: u32 = 1100;
/// Protocol 6: the server tells every client of that version that a controller changed, the sender
/// included. After an `UPDATE_LEDS` the payload is the controller's colours (`NetworkServer.cpp:4175-4330`).
pub const SIGNAL_UPDATE: u32 = 1150;
/// `RGBCONTROLLER_UPDATE_REASON_UPDATELEDS` (`RGBControllerInterface.h:43`).
pub const UPDATE_REASON_UPDATE_LEDS: u32 = 0;
/// Matrix map entry for a grid cell with no LED (OpenRGB's "not used" value).
pub const MATRIX_GAP: u32 = u32::MAX;

/// Frames a packet: `ORGB`, device index, packet id, payload size (all u32 LE), payload.
pub fn encode_packet(device_index: u32, packet_id: u32, payload: &[u8]) -> Result<Vec<u8>, AdapterError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(AdapterError::Protocol(format!("payload of {} bytes exceeds {MAX_PAYLOAD}", payload.len())));
    }
    let size = u32::try_from(payload.len()).map_err(|_| AdapterError::Protocol("payload size".into()))?;
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(b"ORGB");
    bytes.extend_from_slice(&device_index.to_le_bytes());
    bytes.extend_from_slice(&packet_id.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Parses a 16-byte header into `(device_index, packet_id, payload_size)`.
pub fn decode_header(header: &[u8; HEADER_LEN]) -> Result<(u32, u32, usize), AdapterError> {
    if &header[..4] != b"ORGB" {
        return Err(AdapterError::Protocol("invalid OpenRGB packet magic".into()));
    }
    let word = |at: usize| u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let size = word(12) as usize;
    if size > MAX_PAYLOAD {
        return Err(AdapterError::Protocol("OpenRGB payload too large".into()));
    }
    Ok((word(4), word(8), size))
}

/// Appends an OpenRGB string: u16 length including the trailing NUL, bytes, NUL.
pub fn push_c_string(out: &mut Vec<u8>, value: &str) -> Result<(), AdapterError> {
    let len = u16::try_from(value.len() + 1)
        .map_err(|_| AdapterError::Protocol("OpenRGB string longer than 65534 bytes".into()))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    out.push(0);
    Ok(())
}

pub fn client_name_payload(name: &str) -> Vec<u8> {
    let mut payload = name.as_bytes().to_vec();
    payload.push(0);
    payload
}

/// UPDATELEDS payload: size (including itself), u16 LED count, 4 bytes per LED (r, g, b, 0).
pub fn update_leds_payload(colors: &[Rgb8]) -> Result<Vec<u8>, AdapterError> {
    let count = u16::try_from(colors.len())
        .map_err(|_| AdapterError::Protocol(format!("{} LEDs exceeds the u16 OpenRGB limit", colors.len())))?;
    let size = 4 + 2 + colors.len() * 4;
    let mut payload = Vec::with_capacity(size);
    payload.extend_from_slice(&(size as u32).to_le_bytes());
    payload.extend_from_slice(&count.to_le_bytes());
    for color in colors {
        payload.extend_from_slice(&[color.r, color.g, color.b, 0]);
    }
    Ok(payload)
}

pub fn resize_zone_payload(zone: u32, leds: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8);
    payload.extend_from_slice(&zone.to_le_bytes());
    payload.extend_from_slice(&leds.to_le_bytes());
    payload
}

// ------------------------------------------------------------------ controller data

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireController {
    pub device_type: i32,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub version: String,
    pub serial: String,
    pub location: String,
    pub active_mode: i32,
    pub modes: Vec<WireMode>,
    pub zones: Vec<WireZone>,
    pub leds: Vec<(String, u32)>,
    pub colors: Vec<Rgb8>,
    pub led_alt_names: Vec<String>,
    pub flags: u32,
    /// Protocol 6 only: the name the user gave the device (empty before 6).
    pub display_name: String,
    /// Protocol 6 only: the device-specific configuration, a JSON text (empty before 6).
    pub configuration: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireMode {
    pub name: String,
    pub value: i32,
    pub flags: u32,
    pub speed_min: u32,
    pub speed_max: u32,
    pub brightness_min: u32,
    pub brightness_max: u32,
    pub colors_min: u32,
    pub colors_max: u32,
    pub speed: u32,
    pub brightness: u32,
    pub direction: u32,
    pub color_mode: u32,
    pub colors: Vec<Rgb8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireZone {
    pub name: String,
    pub zone_type: i32,
    pub leds_min: u32,
    pub leds_max: u32,
    pub leds_count: u32,
    /// `(height, width, values)`; `values[row * width + col]` is the LED index or a gap marker.
    pub matrix: Option<(u32, u32, Vec<u32>)>,
    pub segments: Vec<WireSegment>,
    pub flags: u32,
    /// Protocol 6 only: the zone's own active mode and modes (zero and none before 6).
    pub active_mode: i32,
    pub modes: Vec<WireMode>,
    /// Protocol 6 only: the zone's display name (empty before 6).
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireSegment {
    pub name: String,
    pub segment_type: i32,
    pub start_index: u32,
    pub leds_count: u32,
    /// Protocol 6 only: the segment's matrix map and flags (none and zero before 6).
    pub matrix: Option<(u32, u32, Vec<u32>)>,
    pub flags: u32,
}

/// OpenRGB's zone types (`RGBControllerInterface.h:153-159`). Protocol 6 adds three of them; a server
/// shows protocol 5 and older the nearest old one (`RGBController.cpp:3186-3203`), and so does
/// [`WireController::decode`] at protocol 6, so the rest of the client sees the five-type world it
/// was written for.
fn legacy_zone_type(raw: i32) -> i32 {
    match raw {
        3 | 6 => 1,
        4 | 5 => 2,
        other => other,
    }
}

/// The reply to `REQUEST_CONTROLLER_COUNT`: the count, and at protocol 6 the id of every controller
/// (`NetworkServer.cpp:3585-3640`). Before 6 a controller is named by its position, so there is no list.
pub fn decode_controller_count(payload: &[u8], version: u32) -> Result<(u32, Vec<u32>), AdapterError> {
    let mut cursor = Cursor::new(payload);
    let count = cursor.u32()?;
    let mut ids = Vec::new();
    if version >= PROTOCOL_WITH_EVENTS {
        // The count is checked against the payload before anything is sized from it.
        if (count as usize).checked_mul(4).is_none_or(|bytes| bytes > cursor.remaining()) {
            return Err(AdapterError::Protocol(format!("controller count {count} does not fit the id list")));
        }
        ids.reserve(count as usize);
        for _ in 0..count {
            ids.push(cursor.u32()?);
        }
    }
    Ok((count, ids))
}

/// The reply to `REQUEST_CONTROLLER_COUNT`, as a server sends it.
pub fn controller_count_payload(ids: &[u32], version: u32) -> Vec<u8> {
    let mut payload = (ids.len() as u32).to_le_bytes().to_vec();
    if version >= PROTOCOL_WITH_EVENTS {
        for id in ids {
            payload.extend_from_slice(&id.to_le_bytes());
        }
    }
    payload
}

/// The payload of an ACK: the id of the packet it acknowledges and a status, 0 for OK
/// (`NetworkProtocol.h:74-76`).
pub fn ack_payload(acked_packet_id: u32, status: u32) -> Vec<u8> {
    let mut payload = acked_packet_id.to_le_bytes().to_vec();
    payload.extend_from_slice(&status.to_le_bytes());
    payload
}

impl WireController {
    /// Decodes a REQUEST_CONTROLLER_DATA payload for protocol `version` (4, 5 or 6).
    ///
    /// The layouts are OpenRGB release_1.1's serializer (`RGBController.cpp:2519-2707` for the
    /// controller, `:2944-3078` modes, `:3163-3337` zones, `:3080-3131` segments, `:2802-2825` LEDs).
    /// Protocol 6 drops the mode value and the LED value, and adds the display name and the
    /// device-specific configuration to the controller, the matrix map and flags to a segment, and
    /// the modes and display name to a zone.
    pub fn decode(payload: &[u8], version: u32) -> Result<Self, AdapterError> {
        if !(MIN_PROTOCOL_VERSION..=CLIENT_MAX_PROTOCOL_VERSION).contains(&version) {
            return Err(AdapterError::Protocol(format!("unsupported protocol version {version}")));
        }
        let mut cursor = Cursor::new(payload);
        let advertised = cursor.u32()? as usize;
        if advertised != payload.len() {
            return Err(AdapterError::Protocol(format!(
                "controller data size {advertised} does not match payload {}", payload.len())));
        }
        let mut controller = WireController {
            device_type: cursor.i32()?,
            name: cursor.string()?,
            vendor: cursor.string()?,
            description: cursor.string()?,
            version: cursor.string()?,
            serial: cursor.string()?,
            location: cursor.string()?,
            ..Default::default()
        };
        let mode_count = cursor.u16()?;
        controller.active_mode = cursor.i32()?;
        for _ in 0..mode_count {
            controller.modes.push(decode_mode(&mut cursor, version)?);
        }
        let zone_count = cursor.u16()?;
        for _ in 0..zone_count {
            controller.zones.push(decode_zone(&mut cursor, version)?);
        }
        let led_count = cursor.u16()?;
        for position in 0..led_count {
            let name = cursor.string()?;
            // Protocol 6 does not send the LED's value (`RGBController.cpp:2818`): its position is
            // the value every earlier protocol sends for the devices canvas-rgb knows.
            let value = if version >= PROTOCOL_WITH_EVENTS { u32::from(position) } else { cursor.u32()? };
            controller.leds.push((name, value));
        }
        let color_count = cursor.u16()?;
        for _ in 0..color_count {
            controller.colors.push(cursor.rgb()?);
        }
        if version >= 5 {
            let alt_count = cursor.u16()?;
            for _ in 0..alt_count {
                controller.led_alt_names.push(cursor.string()?);
            }
            controller.flags = cursor.u32()?;
        }
        if version >= PROTOCOL_WITH_EVENTS {
            controller.display_name = cursor.string()?;
            controller.configuration = cursor.long_string()?;
        }
        if cursor.remaining() != 0 {
            return Err(AdapterError::Protocol(format!("{} trailing bytes in controller data", cursor.remaining())));
        }
        Ok(controller)
    }

    /// Encodes the v5 layout, as a protocol 5 server sends it.
    #[cfg(test)]
    pub fn encode(&self) -> Result<Vec<u8>, AdapterError> {
        self.encode_for(5)
    }

    /// Encodes the layout of protocol `version` (4, 5 or 6), as a server of that version sends it.
    pub fn encode_for(&self, version: u32) -> Result<Vec<u8>, AdapterError> {
        let mut body = Vec::new();
        body.extend_from_slice(&self.device_type.to_le_bytes());
        for text in [&self.name, &self.vendor, &self.description, &self.version, &self.serial, &self.location] {
            push_c_string(&mut body, text)?;
        }
        push_count(&mut body, self.modes.len())?;
        body.extend_from_slice(&self.active_mode.to_le_bytes());
        for mode in &self.modes {
            push_mode(&mut body, mode, version)?;
        }
        push_count(&mut body, self.zones.len())?;
        for zone in &self.zones {
            push_c_string(&mut body, &zone.name)?;
            body.extend_from_slice(&zone.zone_type.to_le_bytes());
            for word in [zone.leds_min, zone.leds_max, zone.leds_count] {
                body.extend_from_slice(&word.to_le_bytes());
            }
            push_matrix(&mut body, zone.matrix.as_ref())?;
            if version >= 4 {
                push_count(&mut body, zone.segments.len())?;
                for segment in &zone.segments {
                    push_c_string(&mut body, &segment.name)?;
                    body.extend_from_slice(&segment.segment_type.to_le_bytes());
                    body.extend_from_slice(&segment.start_index.to_le_bytes());
                    body.extend_from_slice(&segment.leds_count.to_le_bytes());
                    if version >= PROTOCOL_WITH_EVENTS {
                        push_matrix(&mut body, segment.matrix.as_ref())?;
                        body.extend_from_slice(&segment.flags.to_le_bytes());
                    }
                }
            }
            if version >= 5 {
                body.extend_from_slice(&zone.flags.to_le_bytes());
            }
            if version >= PROTOCOL_WITH_EVENTS {
                body.extend_from_slice(&zone.active_mode.to_le_bytes());
                push_count(&mut body, zone.modes.len())?;
                for mode in &zone.modes {
                    push_mode(&mut body, mode, version)?;
                }
                push_c_string(&mut body, &zone.display_name)?;
            }
        }
        push_count(&mut body, self.leds.len())?;
        for (name, value) in &self.leds {
            push_c_string(&mut body, name)?;
            if version < PROTOCOL_WITH_EVENTS {
                body.extend_from_slice(&value.to_le_bytes());
            }
        }
        push_colors(&mut body, &self.colors)?;
        if version >= 5 {
            push_count(&mut body, self.led_alt_names.len())?;
            for name in &self.led_alt_names {
                push_c_string(&mut body, name)?;
            }
            body.extend_from_slice(&self.flags.to_le_bytes());
        }
        if version >= PROTOCOL_WITH_EVENTS {
            push_c_string(&mut body, &self.display_name)?;
            let length = u32::try_from(self.configuration.len() + 1)
                .map_err(|_| AdapterError::Protocol("OpenRGB configuration too long".into()))?;
            body.extend_from_slice(&length.to_le_bytes());
            body.extend_from_slice(self.configuration.as_bytes());
            body.push(0);
        }
        let total = u32::try_from(body.len() + 4).map_err(|_| AdapterError::Protocol("controller data too large".into()))?;
        let mut payload = Vec::with_capacity(body.len() + 4);
        payload.extend_from_slice(&total.to_le_bytes());
        payload.extend_from_slice(&body);
        Ok(payload)
    }
}

/// Appends a matrix map as the server does: the size as u16 (zero for none), then height, width and
/// the cells (`RGBController.cpp:2850-2897`).
fn push_matrix(out: &mut Vec<u8>, matrix: Option<&(u32, u32, Vec<u32>)>) -> Result<(), AdapterError> {
    match matrix {
        None => out.extend_from_slice(&0_u16.to_le_bytes()),
        Some((height, width, values)) => {
            let len = u16::try_from(8 + values.len() * 4)
                .map_err(|_| AdapterError::Protocol("matrix map too large".into()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&height.to_le_bytes());
            out.extend_from_slice(&width.to_le_bytes());
            for value in values {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    Ok(())
}

/// Reads a matrix map the way [`push_matrix`] writes it.
fn read_matrix(cursor: &mut Cursor<'_>) -> Result<Option<(u32, u32, Vec<u32>)>, AdapterError> {
    let matrix_len = usize::from(cursor.u16()?);
    if matrix_len == 0 {
        return Ok(None);
    }
    if matrix_len < 8 || (matrix_len - 8) % 4 != 0 {
        return Err(AdapterError::Protocol(format!("invalid matrix length {matrix_len}")));
    }
    let height = cursor.u32()?;
    let width = cursor.u32()?;
    let mut values = Vec::with_capacity((matrix_len - 8) / 4);
    for _ in 0..(matrix_len - 8) / 4 {
        values.push(cursor.u32()?);
    }
    Ok(Some((height, width, values)))
}

fn push_mode(out: &mut Vec<u8>, mode: &WireMode, version: u32) -> Result<(), AdapterError> {
    push_c_string(out, &mode.name)?;
    if version < PROTOCOL_WITH_EVENTS {
        out.extend_from_slice(&mode.value.to_le_bytes());
    }
    for word in [mode.flags, mode.speed_min, mode.speed_max, mode.brightness_min, mode.brightness_max,
        mode.colors_min, mode.colors_max, mode.speed, mode.brightness, mode.direction, mode.color_mode] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    push_colors(out, &mode.colors)
}

fn decode_mode(cursor: &mut Cursor<'_>, version: u32) -> Result<WireMode, AdapterError> {
    let name = cursor.string()?;
    // Protocol 6 does not send the mode's value (`RGBController.cpp:2970`).
    let value = if version >= PROTOCOL_WITH_EVENTS { 0 } else { cursor.i32()? };
    let mut mode = WireMode { name, value, ..Default::default() };
    mode.flags = cursor.u32()?;
    mode.speed_min = cursor.u32()?;
    mode.speed_max = cursor.u32()?;
    mode.brightness_min = cursor.u32()?;
    mode.brightness_max = cursor.u32()?;
    mode.colors_min = cursor.u32()?;
    mode.colors_max = cursor.u32()?;
    mode.speed = cursor.u32()?;
    mode.brightness = cursor.u32()?;
    mode.direction = cursor.u32()?;
    mode.color_mode = cursor.u32()?;
    mode.colors = cursor.colors()?;
    Ok(mode)
}

fn decode_zone(cursor: &mut Cursor<'_>, version: u32) -> Result<WireZone, AdapterError> {
    let mut zone = WireZone { name: cursor.string()?, zone_type: cursor.i32()?, ..Default::default() };
    if version >= PROTOCOL_WITH_EVENTS {
        zone.zone_type = legacy_zone_type(zone.zone_type);
    }
    zone.leds_min = cursor.u32()?;
    zone.leds_max = cursor.u32()?;
    zone.leds_count = cursor.u32()?;
    zone.matrix = read_matrix(cursor)?;
    if version >= 4 {
        let segment_count = cursor.u16()?;
        for _ in 0..segment_count {
            let mut segment = WireSegment {
                name: cursor.string()?,
                segment_type: cursor.i32()?,
                start_index: cursor.u32()?,
                leds_count: cursor.u32()?,
                ..Default::default()
            };
            if version >= PROTOCOL_WITH_EVENTS {
                segment.matrix = read_matrix(cursor)?;
                segment.flags = cursor.u32()?;
            }
            zone.segments.push(segment);
        }
    }
    if version >= 5 {
        zone.flags = cursor.u32()?;
    }
    if version >= PROTOCOL_WITH_EVENTS {
        zone.active_mode = cursor.i32()?;
        let mode_count = cursor.u16()?;
        for _ in 0..mode_count {
            zone.modes.push(decode_mode(cursor, version)?);
        }
        zone.display_name = cursor.string()?;
    }
    Ok(zone)
}

fn push_count(out: &mut Vec<u8>, count: usize) -> Result<(), AdapterError> {
    let count = u16::try_from(count).map_err(|_| AdapterError::Protocol("OpenRGB count exceeds u16".into()))?;
    out.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn push_colors(out: &mut Vec<u8>, colors: &[Rgb8]) -> Result<(), AdapterError> {
    push_count(out, colors.len())?;
    for color in colors {
        out.extend_from_slice(&[color.r, color.g, color.b, 0]);
    }
    Ok(())
}

/// Bounds-checked little-endian reader. Every read fails with a protocol error, never panics.
pub struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], AdapterError> {
        let end = self.pos.checked_add(len).filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| AdapterError::Protocol("truncated OpenRGB data".into()))?;
        let value = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(value)
    }

    pub fn u16(&mut self) -> Result<u16, AdapterError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, AdapterError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, AdapterError> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn rgb(&mut self) -> Result<Rgb8, AdapterError> {
        let b = self.take(4)?;
        Ok(Rgb8 { r: b[0], g: b[1], b: b[2] })
    }

    fn colors(&mut self) -> Result<Vec<Rgb8>, AdapterError> {
        let count = self.u16()?;
        (0..count).map(|_| self.rgb()).collect()
    }

    pub fn string(&mut self) -> Result<String, AdapterError> {
        let len = usize::from(self.u16()?);
        self.nul_terminated(len)
    }

    /// A string whose length (NUL included) is a u32: the device-specific configuration is one
    /// (`RGBController.cpp:2691-2704`).
    pub fn long_string(&mut self) -> Result<String, AdapterError> {
        let len = self.u32()? as usize;
        self.nul_terminated(len)
    }

    fn nul_terminated(&mut self, len: usize) -> Result<String, AdapterError> {
        let bytes = self.take(len)?;
        let Some((0, content)) = bytes.split_last() else {
            return Err(AdapterError::Protocol("OpenRGB string missing NUL terminator".into()));
        };
        String::from_utf8(content.to_vec()).map_err(|_| AdapterError::Protocol("OpenRGB string is not UTF-8".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WireController {
        WireController {
            device_type: 5,
            name: "Keyboard".into(),
            vendor: "Epomaker".into(),
            description: "d".into(),
            version: "1".into(),
            serial: "kb".into(),
            location: "HID".into(),
            active_mode: 0,
            modes: vec![WireMode { name: "Direct".into(), value: 0, color_mode: 1, ..Default::default() }],
            zones: vec![
                WireZone { name: "Keys".into(), zone_type: 2, leds_min: 2, leds_max: 2, leds_count: 2,
                    matrix: Some((2, 2, vec![0, MATRIX_GAP, 1, 2])), segments: vec![], flags: 0, ..Default::default() },
                WireZone { name: "JARGB1".into(), zone_type: 1, leds_min: 0, leds_max: 300, leds_count: 5,
                    matrix: None, segments: vec![WireSegment { name: "seg".into(), segment_type: 1, start_index: 0, leds_count: 5, ..Default::default() }],
                    flags: 7, ..Default::default() },
            ],
            leds: vec![("A".into(), 0), ("B".into(), 1)],
            colors: vec![Rgb8 { r: 1, g: 2, b: 3 }, Rgb8 { r: 4, g: 5, b: 6 }],
            led_alt_names: vec!["alt".into()],
            flags: 9,
            ..Default::default()
        }
    }

    // ---- protocol 6: golden bytes written by hand from OpenRGB release_1.1's serializer, field by
    // field, so neither the encoder nor the decoder is checked against itself.

    fn u16le(value: u16) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }

    fn u32le(value: u32) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }

    /// A string as `strcpy` writes it after its u16 length (the length counts the NUL).
    fn text(value: &str) -> Vec<u8> {
        let mut bytes = u16le(value.len() as u16 + 1);
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
        bytes
    }

    /// One controller as `GetDeviceDescriptionData(.., 6)` writes it (`RGBController.cpp:2519-2707`),
    /// with `zone_type` as the zone's type word.
    fn golden_v6(zone_type: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(u32le(1)); // the device type: DRAM
        for field in ["N", "V", "D", "1", "S", "L"] {
            body.extend(text(field)); // name, vendor, description, version, serial, location (:2519-2600)
        }
        body.extend(u16le(1)); // number of modes
        body.extend(u32le(0)); // active mode
        // One mode, `GetModeDescriptionData` (:2944-3078): the name, no value at protocol 6 (:2970), the
        // flags, speed min and max, brightness min and max, colours min and max, speed, brightness,
        // direction, colour mode and the colour count.
        body.extend(text("Direct"));
        body.extend(u32le(0x20));
        for word in [0, 0, 0, 0, 0, 0, 0, 0, 0] {
            body.extend(u32le(word));
        }
        body.extend(u32le(1));
        body.extend(u16le(0));
        body.extend(u16le(1)); // number of zones
        // One zone, `GetZoneDescriptionData` (:3163-3337): name, type (:3186), min, max and count, no
        // matrix map (:3264-3269), one segment (:3274-3286) ...
        body.extend(text("Z"));
        body.extend(u32le(zone_type));
        for word in [2, 2, 2] {
            body.extend(u32le(word));
        }
        body.extend(u16le(0));
        body.extend(u16le(1));
        // ... with its name, type, start and count, then at protocol 6 the matrix map size, the map
        // (height, width, cells) and the flags (`GetSegmentDescriptionData`, :3080-3131, :3117-3128) ...
        body.extend(text("g"));
        body.extend(u32le(0));
        body.extend(u32le(0));
        body.extend(u32le(2));
        body.extend(u16le(16));
        body.extend(u32le(1));
        body.extend(u32le(2));
        body.extend(u32le(0));
        body.extend(u32le(1));
        body.extend(u32le(0));
        // ... the zone flags (:3293), then at protocol 6 the zone's active mode, its modes and its
        // display name (:3302-3336).
        body.extend(u32le(0));
        body.extend(u32le(0));
        body.extend(u16le(0));
        body.extend(text("ZD"));
        body.extend(u16le(2)); // number of LEDs; each is a name and, before protocol 6, a value (:2802-2825)
        body.extend(text("A"));
        body.extend(text("B"));
        body.extend(u16le(2)); // colours (:2477-2500): a count, then 4 bytes each
        body.extend([1, 2, 3, 0, 4, 5, 6, 0]);
        body.extend(u16le(0)); // LED alternate names (protocol 5, :2656)
        body.extend(u32le(9)); // controller flags (protocol 5, :2682)
        body.extend(text("DN")); // display name (protocol 6, :2691-2704)
        body.extend(u32le(3)); // configuration: a u32 length, NUL included
        body.extend(b"{}\0");
        let mut payload = u32le(body.len() as u32 + 4); // the size counts itself (`NetworkServer.cpp:3665-3677`)
        payload.extend(body);
        payload
    }

    #[test]
    fn a_protocol_6_controller_decodes_from_the_servers_bytes() {
        let wire = WireController::decode(&golden_v6(1), 6).expect("decode");
        assert_eq!((wire.device_type, wire.name.as_str(), wire.serial.as_str(), wire.location.as_str()), (1, "N", "S", "L"));
        assert_eq!(wire.modes.len(), 1);
        assert_eq!((wire.modes[0].name.as_str(), wire.modes[0].flags, wire.modes[0].color_mode), ("Direct", 0x20, 1));
        let zone = &wire.zones[0];
        assert_eq!((zone.name.as_str(), zone.zone_type, zone.leds_count, zone.flags), ("Z", 1, 2, 0));
        assert_eq!(zone.display_name, "ZD");
        let segment = &zone.segments[0];
        assert_eq!((segment.name.as_str(), segment.leds_count, segment.matrix.clone()), ("g", 2, Some((1, 2, vec![0, 1]))));
        assert_eq!(wire.leds, vec![("A".to_string(), 0), ("B".to_string(), 1)]);
        assert_eq!(wire.colors, vec![Rgb8 { r: 1, g: 2, b: 3 }, Rgb8 { r: 4, g: 5, b: 6 }]);
        assert_eq!((wire.flags, wire.display_name.as_str(), wire.configuration.as_str()), (9, "DN", "{}"));
    }

    #[test]
    fn a_protocol_6_controller_encodes_to_the_servers_bytes() {
        let wire = WireController::decode(&golden_v6(1), 6).expect("decode");
        assert_eq!(wire.encode_for(6).expect("encode"), golden_v6(1));
    }

    #[test]
    fn a_protocol_6_payload_is_not_a_protocol_5_payload() {
        assert!(WireController::decode(&golden_v6(1), 5).is_err());
        assert!(WireController::decode(&golden_v6(1), 7).is_err(), "versions above 6 are refused");
    }

    /// The zone types protocol 6 adds are shown as the nearest old one, as the server does for a
    /// protocol 5 client (`RGBController.cpp:3186-3203`): a client written for five types keeps working.
    #[test]
    fn the_zone_types_protocol_6_adds_read_as_the_nearest_old_type() {
        for (raw, seen) in [(0, 0), (1, 1), (2, 2), (3, 1), (4, 2), (5, 2), (6, 1)] {
            let wire = WireController::decode(&golden_v6(raw), 6).expect("decode");
            assert_eq!(wire.zones[0].zone_type, seen, "zone type {raw}");
        }
    }

    #[test]
    fn the_controller_count_reply_carries_the_ids_at_protocol_6_only() {
        assert_eq!(decode_controller_count(&controller_count_payload(&[7, 9, 12], 6), 6).expect("count"), (3, vec![7, 9, 12]));
        assert_eq!(decode_controller_count(&controller_count_payload(&[7, 9, 12], 5), 5).expect("count"), (3, vec![]));
        // A count that promises more ids than the payload holds is an error, never an allocation.
        let mut lying = u32::MAX.to_le_bytes().to_vec();
        lying.extend_from_slice(&1_u32.to_le_bytes());
        assert!(decode_controller_count(&lying, 6).is_err());
    }

    #[test]
    fn controller_data_round_trips_at_v5() {
        let wire = sample();
        let payload = wire.encode().expect("encode");
        assert_eq!(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize, payload.len(),
            "size prefix counts itself");
        assert_eq!(WireController::decode(&payload, 5).expect("decode"), wire);
    }

    #[test]
    fn v5_payload_does_not_decode_as_v4() {
        // v4 has no alt-name list, controller flags or zone flags, so the v5 tail is left over.
        let payload = sample().encode().expect("encode");
        assert!(WireController::decode(&payload, 4).is_err());
    }

    #[test]
    fn truncated_or_mis_sized_payloads_are_errors() {
        let payload = sample().encode().expect("encode");
        assert!(WireController::decode(&payload[..payload.len() - 1], 5).is_err());
        let mut wrong_prefix = payload.clone();
        wrong_prefix[..4].copy_from_slice(&7_u32.to_le_bytes());
        assert!(WireController::decode(&wrong_prefix, 5).is_err());
        assert!(WireController::decode(&payload, 3).is_err(), "versions below 4 are refused");
    }

    #[test]
    fn packets_frame_header_and_payload() {
        let bytes = encode_packet(3, UPDATE_LEDS, &[9, 8]).expect("encode");
        let mut header = [0; HEADER_LEN];
        header.copy_from_slice(&bytes[..HEADER_LEN]);
        assert_eq!(decode_header(&header).expect("header"), (3, UPDATE_LEDS, 2));
        assert_eq!(&bytes[HEADER_LEN..], &[9, 8]);
        let mut bad = header;
        bad[0] = b'X';
        assert!(decode_header(&bad).is_err());
    }

    #[test]
    fn update_leds_payload_counts_itself_and_the_count_field() {
        let colors = vec![Rgb8 { r: 255, g: 0, b: 0 }; 3];
        let payload = update_leds_payload(&colors).expect("payload");
        assert_eq!(payload.len(), 4 + 2 + 3 * 4);
        assert_eq!(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize, payload.len());
        assert_eq!(&payload[6..10], &[255, 0, 0, 0]);
    }
}
