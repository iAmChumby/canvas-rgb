//! 8-bit RGB as sent to devices. Serializes as a `#rrggbb` string.

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, TS)]
#[ts(export, type = "string")]
#[repr(C)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub const BLACK: Rgb8 = Rgb8::new(0, 0, 0);
    pub const WHITE: Rgb8 = Rgb8::new(255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Linear 0..1 floats (as stored in the canvas) to 8-bit, clamped.
    pub fn from_f32(c: [f32; 3]) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        Self::new(q(c[0]), q(c[1]), q(c[2]))
    }

    pub fn to_f32(self) -> [f32; 3] {
        [self.r as f32 / 255.0, self.g as f32 / 255.0, self.b as f32 / 255.0]
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid color {0:?}: expected #rrggbb")]
pub struct ParseColorError(pub String);

impl FromStr for Rgb8 {
    type Err = ParseColorError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseColorError(s.to_string());
        let hex = s.strip_prefix('#').ok_or_else(err)?;
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(err());
        }
        let p = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err());
        Ok(Self::new(p(0)?, p(2)?, p(4)?))
    }
}

impl fmt::Display for Rgb8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Rgb8 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb8 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c: Rgb8 = "#6000fc".parse().unwrap();
        assert_eq!(c, Rgb8::new(0x60, 0x00, 0xfc));
        assert_eq!(c.to_hex(), "#6000fc");
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"#6000fc\"");
        assert!("6000fc".parse::<Rgb8>().is_err());
        assert!("#60z0fc".parse::<Rgb8>().is_err());
    }
}
