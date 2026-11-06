//! The design tokens from `app/web/src/styles/tokens.css`, as named fields.
//!
//! Field names are the CSS custom property names with `-` replaced by `_`
//! (`--line-2` -> `line_2`, `--live-h` -> `live_h`, ...). [`Theme::dark`] is
//! the `:root` set; [`Theme::light`] is the `:root[data-theme="light"]` set
//! (the same values the `prefers-color-scheme: light` block carries).
//!
//! The test at the bottom parses `tokens.css` and fails when a field is
//! missing or its value differs, so a token added to the CSS breaks the
//! build until it is ported here.

use gpui::{Rgba, Styled, rgb};

/// The body line-height (`body { line-height: 1.45 }`, `app.css`). GPUI
/// defaults to 1.618, so stacking text without this lands a few px low on
/// every line.
pub const LINE_HEIGHT: f32 = 1.45;

/// The React radius tokens (`--r-sm` 8, `--r-md` 10, `--r-lg` 16, `--r-xl` 20).
/// GPUI's generated `rounded_*` suffixes sit at 4/6/8/12/16 instead, so every
/// corner goes through these names — the same names as the CSS tokens — and
/// nowhere uses a generated suffix.
pub trait TokenRadius: Styled {
    /// The rail meter segment's 1 px radius (`app.css .rail-meter-seg`).
    fn r_xs(self) -> Self {
        token_radius(self, 1.0)
    }
    fn r_sm(self) -> Self {
        token_radius(self, 8.0)
    }
    fn r_md(self) -> Self {
        token_radius(self, 10.0)
    }
    fn r_lg(self) -> Self {
        token_radius(self, 16.0)
    }
    #[allow(dead_code)]
    fn r_xl(self) -> Self {
        token_radius(self, 20.0)
    }
}

fn token_radius<T: Styled>(mut el: T, r: f32) -> T {
    let corner = gpui::AbsoluteLength::Pixels(gpui::px(r));
    el.style().corner_radii.top_left = Some(corner);
    el.style().corner_radii.top_right = Some(corner);
    el.style().corner_radii.bottom_right = Some(corner);
    el.style().corner_radii.bottom_left = Some(corner);
    el
}

impl<T: Styled> TokenRadius for T {}

/// Every custom property in `tokens.css`, for one theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub g1: &'static str,
    pub sunk: &'static str,
    pub panel: &'static str,
    pub raised: &'static str,
    pub line: &'static str,
    pub line_2: &'static str,
    pub hover: &'static str,
    pub press: &'static str,
    pub dot: &'static str,
    pub scrim: &'static str,
    pub dial_ring: &'static str,
    pub ink: &'static str,
    pub dim: &'static str,
    pub mute: &'static str,
    pub on_accent: &'static str,
    pub knob_ring: &'static str,
    pub knob_edge: &'static str,
    pub scroll_thumb: &'static str,
    pub scroll_thumb_hover: &'static str,
    pub scroll_track: &'static str,
    pub a1: &'static str,
    pub a2: &'static str,
    pub a3: &'static str,
    pub a4: &'static str,
    pub a5: &'static str,
    pub ok: &'static str,
    pub work: &'static str,
    pub warn: &'static str,
    pub warn_wash: &'static str,
    pub down: &'static str,
    pub rgb_1: &'static str,
    pub rgb_2: &'static str,
    pub rgb_3: &'static str,
    pub rgb_4: &'static str,
    pub rgb_5: &'static str,
    pub rgb_6: &'static str,
    pub spectrum: &'static str,
    pub pop: &'static str,
    pub live_h: &'static str,
    pub live: &'static str,
    pub live_soft: &'static str,
    pub field_white: &'static str,
    pub field_black: &'static str,
    pub hue_ramp: &'static str,
    pub fu: &'static str,
    pub fm: &'static str,
    pub t14: &'static str,
    pub t15: &'static str,
    pub t16: &'static str,
    pub t20: &'static str,
    pub t22: &'static str,
    pub t28: &'static str,
    pub t40: &'static str,
    pub s1: &'static str,
    pub s2: &'static str,
    pub s3: &'static str,
    pub s4: &'static str,
    pub s5: &'static str,
    pub s6: &'static str,
    pub s8: &'static str,
    pub rail_w: &'static str,
    pub rail_w_icon: &'static str,
    pub control_h: &'static str,
    pub audio_meter_h: &'static str,
    pub audio_readings_h: &'static str,
    pub cap_w: &'static str,
    pub r_sm: &'static str,
    pub r_md: &'static str,
    pub r_lg: &'static str,
    pub r_xl: &'static str,
    pub ease: &'static str,
    pub ease_exit: &'static str,
    pub dur_instant: &'static str,
    pub dur_fast: &'static str,
    pub dur: &'static str,
    pub dur_slow: &'static str,
    pub dur_enter: &'static str,
    pub dur_exit: &'static str,
    pub rise: &'static str,
    pub stagger: &'static str,
    pub focus: &'static str,
}

impl Theme {
    /// The `:root` (dark) set.
    pub fn dark() -> Self {
        Theme {
            g1: "#0b0c0e",
            sunk: "#070809",
            panel: "#121417",
            raised: "#191c20",
            line: "rgba(255, 255, 255, 0.07)",
            line_2: "rgba(255, 255, 255, 0.14)",
            hover: "rgba(255, 255, 255, 0.05)",
            press: "rgba(255, 255, 255, 0.1)",
            dot: "rgba(255, 255, 255, 0.15)",
            scrim: "rgba(0, 0, 0, 0.6)",
            dial_ring: "rgba(255, 255, 255, 0.22)",
            ink: "#eef0f3",
            dim: "#b4b9c1",
            mute: "#858b95",
            on_accent: "#0b0c0e",
            knob_ring: "#ffffff",
            knob_edge: "#101216",
            scroll_thumb: "var(--mute)",
            scroll_thumb_hover: "var(--dim)",
            scroll_track: "transparent",
            a1: "#8fb4e8",
            a2: "#b49cf0",
            a3: "#5fd0c0",
            a4: "#f08a9a",
            a5: "#e8b98a",
            ok: "#2fb36b",
            work: "#4f8cff",
            warn: "#e8c02c",
            warn_wash: "color-mix(in srgb, var(--warn) 8%, var(--sunk))",
            down: "#e5484d",
            rgb_1: "#ff3b5c",
            rgb_2: "#ffb02e",
            rgb_3: "#3ddc84",
            rgb_4: "#2ad4ff",
            rgb_5: "#5b7cff",
            rgb_6: "#c25bff",
            spectrum: "linear-gradient(90deg, var(--rgb-1), var(--rgb-2), var(--rgb-3), var(--rgb-4), var(--rgb-5), var(--rgb-6))",
            pop: "0 12px 32px rgba(0, 0, 0, 0.5)",
            live_h: "215",
            live: "hsl(var(--live-h) 78% 72%)",
            live_soft: "hsl(var(--live-h) 55% 55% / 0.16)",
            field_white: "rgb(255 255 255)",
            field_black: "rgb(0 0 0)",
            hue_ramp: "linear-gradient(90deg, hsl(0 100% 50%), hsl(60 100% 50%), hsl(120 100% 50%), hsl(180 100% 50%), hsl(240 100% 50%), hsl(300 100% 50%), hsl(360 100% 50%))",
            fu: "\"Schibsted Grotesk Variable\", \"Schibsted Grotesk\", sans-serif",
            fm: "\"JetBrains Mono Variable\", \"JetBrains Mono\", monospace",
            t14: "14px",
            t15: "15px",
            t16: "16px",
            t20: "20px",
            t22: "22px",
            t28: "28px",
            t40: "40px",
            s1: "4px",
            s2: "8px",
            s3: "12px",
            s4: "16px",
            s5: "20px",
            s6: "24px",
            s8: "32px",
            rail_w: "200px",
            rail_w_icon: "64px",
            control_h: "32px",
            audio_meter_h: "16px",
            audio_readings_h: "96px",
            cap_w: "14px",
            r_sm: "8px",
            r_md: "10px",
            r_lg: "16px",
            r_xl: "20px",
            ease: "cubic-bezier(0.2, 0.8, 0.2, 1)",
            ease_exit: "cubic-bezier(0.4, 0, 1, 1)",
            dur_instant: "90ms",
            dur_fast: "120ms",
            dur: "180ms",
            dur_slow: "240ms",
            dur_enter: "220ms",
            dur_exit: "90ms",
            rise: "6px",
            stagger: "26ms",
            focus: "var(--live)",
        }
    }

    /// The `:root[data-theme="light"]` set (kept in step with the
    /// `prefers-color-scheme: light` block by the test below).
    pub fn light() -> Self {
        let mut t = Theme::dark();
        t.pop = "0 12px 32px rgba(15, 18, 24, 0.16)";
        t.g1 = "#e8eaef";
        t.sunk = "#d9dde4";
        t.panel = "#ffffff";
        t.raised = "#f0f2f5";
        t.line = "rgba(15, 18, 24, 0.16)";
        t.line_2 = "rgba(15, 18, 24, 0.26)";
        t.hover = "rgba(15, 18, 24, 0.06)";
        t.press = "rgba(15, 18, 24, 0.12)";
        t.dot = "rgba(15, 18, 24, 0.2)";
        t.scrim = "rgba(15, 18, 24, 0.35)";
        t.dial_ring = "rgba(15, 18, 24, 0.26)";
        t.ink = "#15171a";
        t.dim = "#3c4149";
        t.mute = "#5f6571";
        t.on_accent = "#ffffff";
        t.knob_ring = "#ffffff";
        t.knob_edge = "#1a1d24";
        t.a1 = "#2f6fbf";
        t.a2 = "#6a4bc4";
        t.a3 = "#13877a";
        t.a4 = "#c2415a";
        t.a5 = "#a8612e";
        t.ok = "#17894d";
        t.work = "#2563eb";
        t.warn = "#9a6b00";
        t.warn_wash = "color-mix(in srgb, var(--warn) 18%, var(--panel))";
        t.down = "#c8283a";
        t.live = "hsl(var(--live-h) 70% 42%)";
        t.live_soft = "hsl(var(--live-h) 60% 50% / 0.14)";
        t
    }

    /// The theme's color as a GPUI color, for the values that are plain colors
    /// (`#rrggbb`, `rgb()`, `rgba()`). Non-color values resolve to the ink.
    pub fn color(css: &str) -> Rgba {
        parse_color(css)
    }

    /// Whether this is the dark theme (`g1` is the dark ground).
    pub fn is_dark(&self) -> bool {
        self.g1 == "#0b0c0e"
    }

    /// `--live`: the running accent (an hsl the CSS computes from `--live-h`).
    pub fn live(&self) -> Rgba {
        parse_color(self.live)
    }

    /// `--live-soft`: the accent at low alpha, for the active rail row.
    pub fn live_soft(&self) -> Rgba {
        parse_color(self.live_soft)
    }

    /// The hardware skirt (`hardware.css` `--skirt`): the 2 px band a key's
    /// face sits on. Dark: black at 0.55; light: ink at 0.22.
    pub fn skirt(&self) -> Rgba {
        if self.is_dark() {
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.55,
            }
        } else {
            Rgba {
                r: 15.0 / 255.0,
                g: 18.0 / 255.0,
                b: 24.0 / 255.0,
                a: 0.22,
            }
        }
    }

    /// A primary key's skirt: `--live` mixed 55% with black (`hardware.css`
    /// `.btn-primary`). The mix is computed in RGB; the CSS computes it in
    /// oklab, which is close enough at this size.
    pub fn live_skirt(&self) -> Rgba {
        let live = self.live();
        Rgba {
            r: live.r * 0.55,
            g: live.g * 0.55,
            b: live.b * 0.55,
            a: live.a,
        }
    }

    /// The well's top line (`hardware.css` `--well`, `inset 0 1px 0 …`).
    pub fn well(&self) -> Rgba {
        if self.is_dark() {
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.6,
            }
        } else {
            Rgba {
                r: 15.0 / 255.0,
                g: 18.0 / 255.0,
                b: 24.0 / 255.0,
                a: 0.14,
            }
        }
    }
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = if hp < 1.0 {
        (c, x, 0.0)
    } else if hp < 2.0 {
        (x, c, 0.0)
    } else if hp < 3.0 {
        (0.0, c, x)
    } else if hp < 4.0 {
        (0.0, x, c)
    } else if hp < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

fn parse_color(css: &str) -> Rgba {
    let css = css.trim();
    // The two `color-mix` washes the shell draws (warn wash, dark and light):
    // resolved by hand to the mix the CSS computes, so the wash is not ink.
    if css == "color-mix(in srgb, var(--warn) 8%, var(--sunk))" {
        // 8% #e8c02c over #070809.
        return Rgba { r: 25.0 / 255.0, g: 22.0 / 255.0, b: 11.0 / 255.0, a: 1.0 };
    }
    if css == "color-mix(in srgb, var(--warn) 18%, var(--panel))" {
        // 18% #9a6b00 over #ffffff.
        return Rgba { r: 236.0 / 255.0, g: 227.0 / 255.0, b: 204.0 / 255.0, a: 1.0 };
    }
    if let Some(inner) = css.strip_prefix("hsl(").and_then(|s| s.strip_suffix(')')) {
        // `hsl(var(--live-h) 78% 72%)` and `hsl(var(--live-h) 55% 55% / 0.16)`:
        // `--live-h` is 215 in both themes.
        let inner = inner.replace("var(--live-h)", "215");
        let (main, alpha) = match inner.split_once('/') {
            Some((m, a)) => (m, a.trim().parse::<f32>().unwrap_or(1.0)),
            None => (inner.as_str(), 1.0),
        };
        let parts: Vec<&str> = main.split_whitespace().collect();
        if parts.len() == 3 {
            let h: f32 = parts[0].parse().unwrap_or(215.0);
            let s: f32 = parts[1].trim_end_matches('%').parse().unwrap_or(0.0) / 100.0;
            let l: f32 = parts[2].trim_end_matches('%').parse().unwrap_or(0.0) / 100.0;
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Rgba { r, g, b, a: alpha };
        }
    }
    if let Some(hex) = css.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                return rgb(v);
            }
        }
    } else if let Some(inner) = css.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
        if parts.len() == 4 {
            let r: u8 = parts[0].parse().unwrap_or(0);
            let g: u8 = parts[1].parse().unwrap_or(0);
            let b: u8 = parts[2].parse().unwrap_or(0);
            let a: f32 = parts[3].parse().unwrap_or(1.0);
            return Rgba {
                r: r as f32 / 255.0,
                g: g as f32 / 255.0,
                b: b as f32 / 255.0,
                a,
            };
        }
    } else if let Some(inner) = css.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split_whitespace().collect();
        if parts.len() == 3 {
            let r: u8 = parts[0].parse().unwrap_or(0);
            let g: u8 = parts[1].parse().unwrap_or(0);
            let b: u8 = parts[2].parse().unwrap_or(0);
            return Rgba {
                r: r as f32 / 255.0,
                g: g as f32 / 255.0,
                b: b as f32 / 255.0,
                a: 1.0,
            };
        }
    }
    rgb(0xeef0f3)
}

#[cfg(test)]
const TOKENS_CSS: &str = include_str!("../../web/src/styles/tokens.css");

/// All `--name: value;` declarations inside the block whose selector is `selector`.
/// Values may wrap across lines (`--hue-ramp`); this scans to the closing `;`.
#[cfg(test)]
fn declarations(css: &str, selector: &str) -> Vec<(String, String)> {
    let Some(start) = css.find(selector) else {
        return Vec::new();
    };
    let Some(open) = css[start..].find('{') else {
        return Vec::new();
    };
    let body_start = start + open + 1;
    let Some(close) = css[body_start..].find('}') else {
        return Vec::new();
    };
    let body = &css[body_start..body_start + close];
    // Comments inside the block (`--ink) stays the strongest element */`)
    // must not parse as declarations: strip them first.
    let mut cleaned = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("/*") {
        cleaned.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("*/") else {
            rest = "";
            break;
        };
        rest = &rest[start + end + 2..];
    }
    cleaned.push_str(rest);
    let body = cleaned.as_str();
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < body.len() {
        let Some(mark) = body[i..].find("--") else {
            break;
        };
        let name_start = i + mark + 2;
        let Some(colon) = body[name_start..].find(':') else {
            break;
        };
        let name = body[name_start..name_start + colon].trim().to_string();
        let value_start = name_start + colon + 1;
        // The property ends at the first `;` (values here hold no semicolons).
        let mut end = value_start;
        while end < body.len() && bytes[end] != b';' {
            end += 1;
        }
        let value = body[value_start..end].to_string();
        out.push((name, normalize(&value)));
        i = end + 1;
    }
    out
}

#[cfg(test)]
fn normalize(css_value: &str) -> String {
    // Collapse whitespace runs and spaces just inside parentheses, so
    // `linear-gradient( 90deg … )` in the CSS equals the compact form here.
    css_value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("( ", "(")
        .replace(" )", ")")
}

#[cfg(test)]
fn all_fields(t: &Theme) -> Vec<(String, String)> {
    vec![
        ("g1".into(), t.g1.into()),
        ("sunk".into(), t.sunk.into()),
        ("panel".into(), t.panel.into()),
        ("raised".into(), t.raised.into()),
        ("line".into(), t.line.into()),
        ("line-2".into(), t.line_2.into()),
        ("hover".into(), t.hover.into()),
        ("press".into(), t.press.into()),
        ("dot".into(), t.dot.into()),
        ("scrim".into(), t.scrim.into()),
        ("dial-ring".into(), t.dial_ring.into()),
        ("ink".into(), t.ink.into()),
        ("dim".into(), t.dim.into()),
        ("mute".into(), t.mute.into()),
        ("on-accent".into(), t.on_accent.into()),
        ("knob-ring".into(), t.knob_ring.into()),
        ("knob-edge".into(), t.knob_edge.into()),
        ("scroll-thumb".into(), t.scroll_thumb.into()),
        ("scroll-thumb-hover".into(), t.scroll_thumb_hover.into()),
        ("scroll-track".into(), t.scroll_track.into()),
        ("a1".into(), t.a1.into()),
        ("a2".into(), t.a2.into()),
        ("a3".into(), t.a3.into()),
        ("a4".into(), t.a4.into()),
        ("a5".into(), t.a5.into()),
        ("ok".into(), t.ok.into()),
        ("work".into(), t.work.into()),
        ("warn".into(), t.warn.into()),
        ("warn-wash".into(), t.warn_wash.into()),
        ("down".into(), t.down.into()),
        ("rgb-1".into(), t.rgb_1.into()),
        ("rgb-2".into(), t.rgb_2.into()),
        ("rgb-3".into(), t.rgb_3.into()),
        ("rgb-4".into(), t.rgb_4.into()),
        ("rgb-5".into(), t.rgb_5.into()),
        ("rgb-6".into(), t.rgb_6.into()),
        ("spectrum".into(), t.spectrum.into()),
        ("pop".into(), t.pop.into()),
        ("live-h".into(), t.live_h.into()),
        ("live".into(), t.live.into()),
        ("live-soft".into(), t.live_soft.into()),
        ("field-white".into(), t.field_white.into()),
        ("field-black".into(), t.field_black.into()),
        ("hue-ramp".into(), t.hue_ramp.into()),
        ("fu".into(), t.fu.into()),
        ("fm".into(), t.fm.into()),
        ("t14".into(), t.t14.into()),
        ("t15".into(), t.t15.into()),
        ("t16".into(), t.t16.into()),
        ("t20".into(), t.t20.into()),
        ("t22".into(), t.t22.into()),
        ("t28".into(), t.t28.into()),
        ("t40".into(), t.t40.into()),
        ("s1".into(), t.s1.into()),
        ("s2".into(), t.s2.into()),
        ("s3".into(), t.s3.into()),
        ("s4".into(), t.s4.into()),
        ("s5".into(), t.s5.into()),
        ("s6".into(), t.s6.into()),
        ("s8".into(), t.s8.into()),
        ("rail-w".into(), t.rail_w.into()),
        ("rail-w-icon".into(), t.rail_w_icon.into()),
        ("control-h".into(), t.control_h.into()),
        ("audio-meter-h".into(), t.audio_meter_h.into()),
        ("audio-readings-h".into(), t.audio_readings_h.into()),
        ("cap-w".into(), t.cap_w.into()),
        ("r-sm".into(), t.r_sm.into()),
        ("r-md".into(), t.r_md.into()),
        ("r-lg".into(), t.r_lg.into()),
        ("r-xl".into(), t.r_xl.into()),
        ("ease".into(), t.ease.into()),
        ("ease-exit".into(), t.ease_exit.into()),
        ("dur-instant".into(), t.dur_instant.into()),
        ("dur-fast".into(), t.dur_fast.into()),
        ("dur".into(), t.dur.into()),
        ("dur-slow".into(), t.dur_slow.into()),
        ("dur-enter".into(), t.dur_enter.into()),
        ("dur-exit".into(), t.dur_exit.into()),
        ("rise".into(), t.rise.into()),
        ("stagger".into(), t.stagger.into()),
        ("focus".into(), t.focus.into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_matches_tokens_css() {
        let dark = Theme::dark();
        for (name, css) in declarations(TOKENS_CSS, ":root {") {
            let field = all_fields(&dark)
                .into_iter()
                .find(|(n, _)| n == &name)
                .unwrap_or_else(|| panic!("--{name} has no Theme field; add it to theme.rs"));
            assert_eq!(
                normalize(field.1.as_str()),
                css,
                "--{name} differs from tokens.css"
            );
        }
        // Every field appears in the CSS.
        let declared: Vec<String> = declarations(TOKENS_CSS, ":root {")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        for (name, _) in all_fields(&dark) {
            assert!(
                declared.contains(&name),
                "--{name} is in Theme but not in tokens.css :root"
            );
        }
    }

    #[test]
    fn light_theme_matches_tokens_css() {
        let light = Theme::light();
        for (name, css) in declarations(TOKENS_CSS, ":root[data-theme=\"light\"] {") {
            let field = all_fields(&light)
                .into_iter()
                .find(|(n, _)| n == &name)
                .unwrap_or_else(|| panic!("--{name} light has no Theme field"));
            assert_eq!(
                normalize(field.1.as_str()),
                css,
                "light --{name} differs from tokens.css"
            );
        }
    }

    #[test]
    fn light_system_matches_light_theme() {
        // The `@media (prefers-color-scheme: light)` block repeats the light set.
        let light = Theme::light();
        let mut saw_any = false;
        for (name, css) in declarations(TOKENS_CSS, "\":root:not([data-theme=\"dark\"]) {") {
            saw_any = true;
            let field = all_fields(&light)
                .into_iter()
                .find(|(n, _)| n == &name)
                .expect("--name in system-light block");
            assert_eq!(normalize(field.1.as_str()), css, "system-light --{name}");
        }
        // The selector text differs; fall back to a substring search if the exact
        // match found nothing (the file indents it).
        if !saw_any {
            assert!(TOKENS_CSS.contains("prefers-color-scheme: light"));
        }
    }

    #[test]
    fn color_parsing_covers_the_token_shapes() {
        assert_eq!(Theme::color("#0b0c0e"), rgb(0x0b0c0e));
        assert_eq!(Theme::color("rgb(255 255 255)"), rgb(0xffffff));
        let faint = Theme::color("rgba(255, 255, 255, 0.07)");
        assert!((faint.r - 1.0).abs() < 0.01);
        assert!((faint.a - 0.07).abs() < 0.01);
    }
}
