//! CSS color parsing for the Canvas 2D fill/stroke styles.
//!
//! Accepts the forms a SignalRGB effect realistically uses: `#rgb`, `#rgba`, `#rrggbb`,
//! `#rrggbbaa`, `rgb()`, `rgba()`, `hsl()`, `hsla()` and the named basics. Anything else is
//! a parse failure, which the context treats as "keep the previous style" (the browser
//! does the same: an invalid assignment to `fillStyle` leaves the old value in place).

/// A straight-alpha sRGB color, 0..1 per channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CssColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl CssColor {
    pub const BLACK: CssColor = CssColor { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    /// `rgba(255, 0, 0, 0.5)`-style from bytes and a 0..1 alpha.
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
    }
}

/// The CSS named colors a 2D context is expected to know. The full CSS list has 148
/// entries; these are the ones effects actually use. An unknown name is a parse error.
fn named(name: &str) -> Option<CssColor> {
    let rgb = match name {
        "black" => (0, 0, 0),
        "silver" => (192, 192, 192),
        "gray" | "grey" => (128, 128, 128),
        "white" => (255, 255, 255),
        "maroon" => (128, 0, 0),
        "red" => (255, 0, 0),
        "purple" => (128, 0, 128),
        "fuchsia" | "magenta" => (255, 0, 255),
        "green" => (0, 128, 0),
        "lime" => (0, 255, 0),
        "olive" => (128, 128, 0),
        "yellow" => (255, 255, 0),
        "navy" => (0, 0, 128),
        "blue" => (0, 0, 255),
        "teal" => (0, 128, 128),
        "aqua" | "cyan" => (0, 255, 255),
        "orange" => (255, 165, 0),
        "pink" => (255, 192, 203),
        "brown" => (165, 42, 42),
        "gold" => (255, 215, 0),
        "indigo" => (75, 0, 130),
        "violet" => (238, 130, 238),
        "darkblue" => (0, 0, 139),
        "deeppink" => (255, 20, 147),
        "hotpink" => (255, 105, 180),
        "crimson" => (220, 20, 60),
        "coral" => (255, 127, 80),
        "darkgreen" => (0, 100, 0),
        "darkred" => (139, 0, 0),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "salmon" => (250, 128, 114),
        "tan" => (210, 180, 140),
        "turquoise" => (64, 224, 208),
        "beige" => (245, 245, 220),
        "ivory" => (255, 255, 240),
        "azure" => (240, 255, 255),
        _ => return None,
    };
    Some(CssColor::from_rgba8(rgb.0, rgb.1, rgb.2, 1.0))
}

/// Parses a CSS color string. Leading/trailing whitespace is ignored.
pub fn parse(input: &str) -> Option<CssColor> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    // A function call: name(args).
    if s.contains('(') {
        let open = s.find('(')?;
        if !s.ends_with(')') {
            return None;
        }
        let name = s[..open].trim().to_ascii_lowercase();
        let args = &s[open + 1..s.len() - 1];
        return match name.as_str() {
            "rgb" | "rgba" => parse_rgb(args),
            "hsl" | "hsla" => parse_hsl(args),
            _ => None,
        };
    }
    // A named color.
    named(&s.to_ascii_lowercase())
}

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`.
fn parse_hex(hex: &str) -> Option<CssColor> {
    // The digits are sliced by byte index below, which is only safe for ASCII.
    if !hex.is_ascii() {
        return None;
    }
    let nibble = |c: u8| -> Option<f32> {
        let v = (c as char).to_digit(16)?;
        Some(v as f32 * 17.0 / 255.0)
    };
    let byte = |s: &str| -> Option<f32> { u8::from_str_radix(s, 16).ok().map(|v| v as f32 / 255.0) };
    match hex.len() {
        3 => {
            // #rgb expands each nibble: f -> ff.
            let mut it = hex.bytes();
            let r = nibble(it.next()?)?;
            let g = nibble(it.next()?)?;
            let b = nibble(it.next()?)?;
            Some(CssColor { r, g, b, a: 1.0 })
        }
        4 => {
            let mut it = hex.bytes();
            let r = nibble(it.next()?)?;
            let g = nibble(it.next()?)?;
            let b = nibble(it.next()?)?;
            let a = nibble(it.next()?)?;
            Some(CssColor { r, g, b, a })
        }
        6 => Some(CssColor {
            r: byte(&hex[0..2])?,
            g: byte(&hex[2..4])?,
            b: byte(&hex[4..6])?,
            a: 1.0,
        }),
        8 => Some(CssColor {
            r: byte(&hex[0..2])?,
            g: byte(&hex[2..4])?,
            b: byte(&hex[4..6])?,
            a: byte(&hex[6..8])?,
        }),
        _ => None,
    }
}

/// Splits `rgb()`/`rgba()` arguments on commas or whitespace, honoring percentages.
fn split_args(args: &str) -> Vec<&str> {
    args.split(|c: char| c == ',' || c.is_whitespace() || c == '/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// A channel value: either a 0..255 number or a percentage.
fn channel(token: &str) -> Option<f32> {
    if let Some(pct) = token.strip_suffix('%') {
        let v: f32 = pct.trim().parse().ok()?;
        Some((v / 100.0).clamp(0.0, 1.0))
    } else {
        let v: f32 = token.parse().ok()?;
        Some((v / 255.0).clamp(0.0, 1.0))
    }
}

/// Alpha: a number 0..1, or a percentage.
fn alpha(token: &str) -> Option<f32> {
    if let Some(pct) = token.strip_suffix('%') {
        let v: f32 = pct.trim().parse().ok()?;
        Some((v / 100.0).clamp(0.0, 1.0))
    } else {
        let v: f32 = token.parse().ok()?;
        Some(v.clamp(0.0, 1.0))
    }
}

fn parse_rgb(args: &str) -> Option<CssColor> {
    let parts = split_args(args);
    if parts.len() < 3 {
        return None;
    }
    let r = channel(parts[0])?;
    let g = channel(parts[1])?;
    let b = channel(parts[2])?;
    let a = match parts.get(3) {
        Some(t) => alpha(t)?,
        None => 1.0,
    };
    Some(CssColor { r, g, b, a })
}

fn parse_hsl(args: &str) -> Option<CssColor> {
    let parts = split_args(args);
    if parts.len() < 3 {
        return None;
    }
    let h: f32 = parts[0].parse().ok()?;
    let s: f32 = parts[1].strip_suffix('%')?.trim().parse().ok()?;
    let l: f32 = parts[2].strip_suffix('%')?.trim().parse().ok()?;
    let a = match parts.get(3) {
        Some(t) => alpha(t)?,
        None => 1.0,
    };
    let (r, g, b) = hsl_to_rgb(h, s / 100.0, l / 100.0);
    Some(CssColor { r, g, b, a })
}

/// `hsl()` to RGB, per the CSS spec's algorithm.
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = ((h % 360.0) + 360.0) % 360.0;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn hex_forms() {
        let c = parse("#ff0000").unwrap();
        assert!(close(c.r, 1.0) && close(c.g, 0.0) && close(c.a, 1.0));
        let c = parse("#f00").unwrap();
        assert!(close(c.r, 1.0) && close(c.g, 0.0));
        let c = parse("#ff000080").unwrap();
        assert!(close(c.a, 128.0 / 255.0));
        let c = parse("#f008").unwrap();
        assert!(close(c.a, 136.0 / 255.0));
        // A five-digit hex is not a CSS color.
        assert!(parse("#ff000").is_none());
        assert!(parse("#gg0000").is_none());
        // Non-ASCII input is rejected without slicing inside a character.
        assert!(parse("#ééé").is_none());
    }

    #[test]
    fn rgb_and_rgba() {
        let c = parse("rgb(255, 128, 0)").unwrap();
        assert!(close(c.r, 1.0) && close(c.g, 128.0 / 255.0) && close(c.b, 0.0));
        let c = parse("rgba(255, 0, 0, 0.5)").unwrap();
        assert!(close(c.a, 0.5));
        let c = parse("rgb(100% 0% 0%)").unwrap();
        assert!(close(c.r, 1.0));
    }

    #[test]
    fn hsl_and_hsla() {
        let c = parse("hsl(0, 100%, 50%)").unwrap();
        assert!(close(c.r, 1.0) && close(c.g, 0.0) && close(c.b, 0.0));
        let c = parse("hsla(120, 100%, 50%, 0.25)").unwrap();
        assert!(close(c.g, 1.0) && close(c.a, 0.25));
    }

    #[test]
    fn named_colors() {
        assert!(parse("black").unwrap() == CssColor::BLACK);
        assert!(parse("red").unwrap() == CssColor::from_rgba8(255, 0, 0, 1.0));
        assert!(parse("notacolor").is_none());
    }

    #[test]
    fn an_eight_digit_hex_carries_its_alpha() {
        // The SignalRGB format writes a colour plus its alpha as one hex string:
        // ctx.fillStyle = background + "22" -> #rrggbbaa.
        let c = parse("#0b1b3a14").unwrap();
        assert!(close(c.r, 0x0b as f32 / 255.0));
        assert!(close(c.g, 0x1b as f32 / 255.0));
        assert!(close(c.b, 0x3a as f32 / 255.0));
        assert!(close(c.a, 0x14 as f32 / 255.0));
    }
}
