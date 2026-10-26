//! OpenRGB SDK wire format, shared by the client (`openrgb.rs`) and the fake server (`fake.rs`).
//!
//! Layouts follow OpenRGB's SDK as implemented by Hypercolor's `hypercolor-openrgb-sdk`
//! (Apache-2.0; see `NOTICE`). Every size field counts the bytes after the header, and the
//! controller-data size prefix counts itself, as OpenRGB does.

use canvas_proto::{AdapterError, Rgb8};

pub const HEADER_LEN: usize = 16;
pub const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
/// Highest protocol version this client asks for.
pub const CLIENT_MAX_PROTOCOL_VERSION: u32 = 5;
/// Oldest protocol version whose controller-data layout this codec decodes.
pub const MIN_PROTOCOL_VERSION: u32 = 4;

pub const REQUEST_CONTROLLER_COUNT: u32 = 0;
pub const REQUEST_CONTROLLER_DATA: u32 = 1;
pub const REQUEST_PROTOCOL_VERSION: u32 = 40;
pub const SET_CLIENT_NAME: u32 = 50;
pub const DEVICE_LIST_UPDATED: u32 = 100;
pub const RESIZE_ZONE: u32 = 1000;
pub const UPDATE_LEDS: u32 = 1050;
pub const SET_CUSTOM_MODE: u32 = 1100;
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
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireSegment {
    pub name: String,
    pub segment_type: i32,
    pub start_index: u32,
    pub leds_count: u32,
}

impl WireController {
    /// Decodes a REQUEST_CONTROLLER_DATA payload for protocol `version` (4 or 5).
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
            controller.modes.push(decode_mode(&mut cursor)?);
        }
        let zone_count = cursor.u16()?;
        for _ in 0..zone_count {
            controller.zones.push(decode_zone(&mut cursor, version)?);
        }
        let led_count = cursor.u16()?;
        for _ in 0..led_count {
            let name = cursor.string()?;
            controller.leds.push((name, cursor.u32()?));
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
        if cursor.remaining() != 0 {
            return Err(AdapterError::Protocol(format!("{} trailing bytes in controller data", cursor.remaining())));
        }
        Ok(controller)
    }

    /// Encodes the v5 layout, as a server sends it.
    pub fn encode(&self) -> Result<Vec<u8>, AdapterError> {
        let mut body = Vec::new();
        body.extend_from_slice(&self.device_type.to_le_bytes());
        for text in [&self.name, &self.vendor, &self.description, &self.version, &self.serial, &self.location] {
            push_c_string(&mut body, text)?;
        }
        push_count(&mut body, self.modes.len())?;
        body.extend_from_slice(&self.active_mode.to_le_bytes());
        for mode in &self.modes {
            push_c_string(&mut body, &mode.name)?;
            body.extend_from_slice(&mode.value.to_le_bytes());
            for word in [mode.flags, mode.speed_min, mode.speed_max, mode.brightness_min, mode.brightness_max,
                mode.colors_min, mode.colors_max, mode.speed, mode.brightness, mode.direction, mode.color_mode] {
                body.extend_from_slice(&word.to_le_bytes());
            }
            push_colors(&mut body, &mode.colors)?;
        }
        push_count(&mut body, self.zones.len())?;
        for zone in &self.zones {
            push_c_string(&mut body, &zone.name)?;
            body.extend_from_slice(&zone.zone_type.to_le_bytes());
            for word in [zone.leds_min, zone.leds_max, zone.leds_count] {
                body.extend_from_slice(&word.to_le_bytes());
            }
            match &zone.matrix {
                None => body.extend_from_slice(&0_u16.to_le_bytes()),
                Some((height, width, values)) => {
                    let len = u16::try_from(8 + values.len() * 4)
                        .map_err(|_| AdapterError::Protocol("matrix map too large".into()))?;
                    body.extend_from_slice(&len.to_le_bytes());
                    body.extend_from_slice(&height.to_le_bytes());
                    body.extend_from_slice(&width.to_le_bytes());
                    for value in values {
                        body.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            push_count(&mut body, zone.segments.len())?;
            for segment in &zone.segments {
                push_c_string(&mut body, &segment.name)?;
                body.extend_from_slice(&segment.segment_type.to_le_bytes());
                body.extend_from_slice(&segment.start_index.to_le_bytes());
                body.extend_from_slice(&segment.leds_count.to_le_bytes());
            }
            body.extend_from_slice(&zone.flags.to_le_bytes());
        }
        push_count(&mut body, self.leds.len())?;
        for (name, value) in &self.leds {
            push_c_string(&mut body, name)?;
            body.extend_from_slice(&value.to_le_bytes());
        }
        push_colors(&mut body, &self.colors)?;
        push_count(&mut body, self.led_alt_names.len())?;
        for name in &self.led_alt_names {
            push_c_string(&mut body, name)?;
        }
        body.extend_from_slice(&self.flags.to_le_bytes());
        let total = u32::try_from(body.len() + 4).map_err(|_| AdapterError::Protocol("controller data too large".into()))?;
        let mut payload = Vec::with_capacity(body.len() + 4);
        payload.extend_from_slice(&total.to_le_bytes());
        payload.extend_from_slice(&body);
        Ok(payload)
    }
}

fn decode_mode(cursor: &mut Cursor<'_>) -> Result<WireMode, AdapterError> {
    let mut mode = WireMode { name: cursor.string()?, value: cursor.i32()?, ..Default::default() };
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
    zone.leds_min = cursor.u32()?;
    zone.leds_max = cursor.u32()?;
    zone.leds_count = cursor.u32()?;
    let matrix_len = usize::from(cursor.u16()?);
    if matrix_len != 0 {
        if matrix_len < 8 || (matrix_len - 8) % 4 != 0 {
            return Err(AdapterError::Protocol(format!("invalid matrix length {matrix_len}")));
        }
        let height = cursor.u32()?;
        let width = cursor.u32()?;
        let mut values = Vec::with_capacity((matrix_len - 8) / 4);
        for _ in 0..(matrix_len - 8) / 4 {
            values.push(cursor.u32()?);
        }
        zone.matrix = Some((height, width, values));
    }
    if version >= 4 {
        let segment_count = cursor.u16()?;
        for _ in 0..segment_count {
            zone.segments.push(WireSegment {
                name: cursor.string()?,
                segment_type: cursor.i32()?,
                start_index: cursor.u32()?,
                leds_count: cursor.u32()?,
            });
        }
    }
    if version >= 5 {
        zone.flags = cursor.u32()?;
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
                    matrix: Some((2, 2, vec![0, MATRIX_GAP, 1, 2])), segments: vec![], flags: 0 },
                WireZone { name: "JARGB1".into(), zone_type: 1, leds_min: 0, leds_max: 300, leds_count: 5,
                    matrix: None, segments: vec![WireSegment { name: "seg".into(), segment_type: 1, start_index: 0, leds_count: 5 }],
                    flags: 7 },
            ],
            leds: vec![("A".into(), 0), ("B".into(), 1)],
            colors: vec![Rgb8 { r: 1, g: 2, b: 3 }, Rgb8 { r: 4, g: 5, b: 6 }],
            led_alt_names: vec!["alt".into()],
            flags: 9,
        }
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
