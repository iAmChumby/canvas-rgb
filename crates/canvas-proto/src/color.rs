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

// ---------------------------------------------------------------- colour temperature

/// The warmest colour temperature [`kelvin_to_rgb`] draws, in kelvin.
pub const KELVIN_MIN: f32 = 1800.0;
/// The coolest colour temperature [`kelvin_to_rgb`] draws, in kelvin. The global warmth
/// stage at this value changes nothing ([`warmth_gains`]).
pub const KELVIN_MAX: f32 = 6500.0;
/// The least blue a white ever gets, 0..1, before [`kelvin_to_led`] decodes it.
///
/// A blackbody below about 2100 K has almost no blue, and an RGB LED with its blue off
/// reads as orange, not as a warm white, the more so when it is dimmed. Holding blue at
/// this floor keeps the warm end a white.
pub const KELVIN_BLUE_FLOOR: f32 = 0.12;

/// A white of colour temperature `kelvin`, as display (sRGB-encoded) RGB in 0..1: the
/// colour a screen shows for it.
///
/// The curve is Tanner Helland's fit to the blackbody colours, the one most LED tools use.
/// `kelvin` is clamped to [`KELVIN_MIN`]..=[`KELVIN_MAX`]; a non-finite value draws the
/// warmest white. Over that range red stays full and green and blue rise with the
/// temperature, so 6500 K is close to a neutral white.
pub fn kelvin_to_rgb(kelvin: f32) -> [f32; 3] {
    let k = if kelvin.is_finite() { kelvin.clamp(KELVIN_MIN, KELVIN_MAX) } else { KELVIN_MIN };
    // The fit works in hundreds of kelvin and gives 0..255 per channel.
    let t = (k / 100.0) as f64;
    let red = if t <= 66.0 { 255.0 } else { 329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) };
    let green = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let blue = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    let unit = |v: f64| (v / 255.0).clamp(0.0, 1.0) as f32;
    [unit(red), unit(green), unit(blue).max(KELVIN_BLUE_FLOOR)]
}

/// One sRGB-encoded channel, 0..1, to linear light, 0..1 (IEC 61966-2-1).
pub fn srgb_to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The white of colour temperature `kelvin` as LED drive levels, 0..1.
///
/// An LED's PWM duty is linear light, while [`kelvin_to_rgb`] gives display values, which a
/// screen darkens on the way out. Sent unchanged, a warm white's green and blue come out
/// too bright and the white reads yellow instead of amber (#50: 1800 K looked far less warm
/// than the Govee lamps). Decoding to linear light first gives the LED the light the colour
/// stands for: 1800 K is `#ff3603` instead of `#ff7e1f`.
pub fn kelvin_to_led(kelvin: f32) -> [f32; 3] {
    kelvin_to_rgb(kelvin).map(srgb_to_linear)
}

/// The per-channel gains of the global warmth stage at `kelvin`, 0..1.
///
/// [`KELVIN_MAX`] and above (and a non-finite value) is the identity, `[1, 1, 1]`: the stage
/// is off. Below it, each channel is scaled by the LED white of `kelvin` relative to the LED
/// white of [`KELVIN_MAX`]. The ratio, not [`kelvin_to_led`] itself, is what keeps 6500 K
/// exactly neutral; it also means a white canvas under the stage is a little bluer than the
/// White effect at the same kelvin (1800 K: green about 0.212 against 0.210, blue
/// about 0.014 against 0.013 of full), because the LED white of 6500 K is itself slightly warm.
pub fn warmth_gains(kelvin: f32) -> [f32; 3] {
    if !kelvin.is_finite() || kelvin >= KELVIN_MAX {
        return [1.0; 3];
    }
    let warm = kelvin_to_led(kelvin);
    let cool = kelvin_to_led(KELVIN_MAX);
    [0, 1, 2].map(|i| (warm[i] / cool[i]).clamp(0.0, 1.0))
}

/// A device's white balance (#50): one gain per LED channel, 0..1, multiplied into every
/// colour the device is sent. The default changes nothing. Gains only ever cut a channel,
/// so a balanced colour never clips.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WhiteBalance {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl Default for WhiteBalance {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

impl WhiteBalance {
    pub const NEUTRAL: WhiteBalance = WhiteBalance { red: 1.0, green: 1.0, blue: 1.0 };

    /// Every gain is finite and in 0..=1.
    pub fn is_valid(&self) -> bool {
        [self.red, self.green, self.blue].iter().all(|g| g.is_finite() && (0.0..=1.0).contains(g))
    }

    pub fn gains(&self) -> [f32; 3] {
        [self.red, self.green, self.blue]
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

    #[test]
    fn the_led_white_decodes_the_display_white() {
        // 1800 K: display (255, 126, 31) -> LED (255, 54, 3).
        let led = kelvin_to_led(1800.0);
        let to8 = |v: f32| (v * 255.0).round() as u8;
        assert_eq!(led.map(to8), [255, 54, 3], "{led:?}");
        let led = kelvin_to_led(2700.0);
        assert_eq!(led.map(to8), [255, 98, 25], "{led:?}");
        // Linear light is never brighter than the display value it came from.
        for k in [1800.0, 2700.0, 4000.0, 6500.0] {
            let (d, l) = (kelvin_to_rgb(k), kelvin_to_led(k));
            assert!((0..3).all(|i| l[i] <= d[i] + 1e-6), "{k} K: {l:?} above {d:?}");
        }
    }

    #[test]
    fn warmth_is_off_at_the_cool_end_and_warms_below_it() {
        assert_eq!(warmth_gains(KELVIN_MAX), [1.0; 3]);
        assert_eq!(warmth_gains(9000.0), [1.0; 3]);
        assert_eq!(warmth_gains(f32::NAN), [1.0; 3]);
        let mut last = warmth_gains(KELVIN_MAX - 100.0);
        let mut k = KELVIN_MAX - 200.0;
        while k >= KELVIN_MIN {
            let g = warmth_gains(k);
            assert!(g.iter().all(|v| (0.0..=1.0).contains(v)), "{k} K: {g:?}");
            assert!(g[2] <= last[2] && g[1] <= last[1], "{k} K is no warmer than {} K", k + 100.0);
            last = g;
            k -= 100.0;
        }
        assert!(warmth_gains(KELVIN_MIN)[2] < 0.05, "1800 K kept its blue");
    }

    #[test]
    fn white_balance_validates_its_gains() {
        assert!(WhiteBalance::NEUTRAL.is_valid());
        assert!(WhiteBalance { red: 0.0, green: 0.5, blue: 1.0 }.is_valid());
        assert!(!WhiteBalance { red: 1.1, green: 1.0, blue: 1.0 }.is_valid());
        assert!(!WhiteBalance { red: -0.1, green: 1.0, blue: 1.0 }.is_valid());
        assert!(!WhiteBalance { red: f32::NAN, green: 1.0, blue: 1.0 }.is_valid());
    }
}
