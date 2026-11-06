//! Fonts, embedded (#G10): the Schibsted Grotesk and JetBrains Mono variable
//! files from `@fontsource-variable/*` (the same files the React editor loads),
//! with their OFL licence files, copied into `assets/fonts/`.
//!
//! Registration is [`register`] on the app's text system. [`font_ui`] and
//! [`font_mono`] are the families `tokens.css` names (`--fu`, `--fm`).

use std::borrow::Cow;

use gpui::{App, Font, font};

/// The UI font (`--fu`). The family name inside the font file: the "Variable" suffix in
/// `tokens.css` is the name fontsource gives its `@font-face`, not part of the file.
pub const UI_FAMILY: &str = "Schibsted Grotesk";
/// The figures font (`--fm`), by the family name inside the font file.
pub const MONO_FAMILY: &str = "JetBrains Mono";

static SCHIBSTED: &[u8] = include_bytes!("../assets/fonts/schibsted-grotesk-variable.ttf");
static JETBRAINS: &[u8] = include_bytes!("../assets/fonts/jetbrains-mono-variable.ttf");

#[cfg(test)]
pub const SCHIBSTED_OFL: &str = include_str!("../assets/fonts/SchibstedGrotesk-OFL.txt");
#[cfg(test)]
pub const JETBRAINS_OFL: &str = include_str!("../assets/fonts/JetBrainsMono-OFL.txt");
// The woff2 files are what the React editor ships (the latin `wght` subsets); gpui's text
// system (DirectWrite) rejects woff2, so the TTF files above are lossless decompressions of
// them, every table identical except `head` (checksums): fontTools `TTFont(woff2)`,
// `flavor = None`, `save(ttf)`. The hash test checks the woff2 copies against node_modules.
#[cfg(test)]
static SCHIBSTED_WOFF2: &[u8] = include_bytes!("../assets/fonts/schibsted-grotesk-variable.woff2");
#[cfg(test)]
static JETBRAINS_WOFF2: &[u8] = include_bytes!("../assets/fonts/jetbrains-mono-variable.woff2");

/// Register both variable fonts with the app's text system.
pub fn register(cx: &App) {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(SCHIBSTED),
            Cow::Borrowed(JETBRAINS),
        ])
        .expect("embedded variable fonts");
}

/// The UI font (`--fu`).
pub fn font_ui() -> Font {
    font(UI_FAMILY)
}

/// The figures font (`--fm`).
pub fn font_mono() -> Font {
    font(MONO_FAMILY)
}

/// FNV-1a (64-bit), for the hash test below — no extra crate.
#[cfg(test)]
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded bytes hash the same as the copies in `node_modules`
    /// (`@fontsource-variable/schibsted-grotesk` and
    /// `@fontsource-variable/jetbrains-mono`), so the GPUI editor draws with
    /// the very files the React editor ships. Skipped with a clear message
    /// when `node_modules` is absent (a fresh checkout without `npm ci`).
    #[test]
    fn embedded_fonts_match_node_modules() {
        let origin = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/node_modules/@fontsource-variable"
        );
        let schibsted = std::path::Path::new(origin)
            .join("schibsted-grotesk/files/schibsted-grotesk-latin-wght-normal.woff2");
        let jetbrains = std::path::Path::new(origin)
            .join("jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2");
        if !schibsted.exists() || !jetbrains.exists() {
            eprintln!("node_modules absent; skipping embedded-font hash check");
            return;
        }
        let schibsted_disk = std::fs::read(&schibsted).expect("read schibsted woff2");
        let jetbrains_disk = std::fs::read(&jetbrains).expect("read jetbrains woff2");
        assert_eq!(
            fnv1a(SCHIBSTED_WOFF2),
            fnv1a(&schibsted_disk),
            "embedded schibsted-grotesk does not match node_modules"
        );
        assert_eq!(
            fnv1a(JETBRAINS_WOFF2),
            fnv1a(&jetbrains_disk),
            "embedded jetbrains-mono does not match node_modules"
        );
    }

    #[test]
    fn both_ofl_licences_are_embedded() {
        assert!(SCHIBSTED_OFL.contains("SIL Open Font License"));
        assert!(JETBRAINS_OFL.contains("SIL Open Font License"));
    }
}
