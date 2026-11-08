//! Fonts, embedded (#G10): Schibsted Grotesk and JetBrains Mono, made from the
//! `@fontsource-variable/*` files the React editor loads, with their OFL licence
//! files, in `assets/fonts/`.
//!
//! Registration is [`register`] on the app's text system. [`UI_FAMILY`] and
//! [`MONO_FAMILY`] are the families `tokens.css` names (`--fu`, `--fm`). Set them with
//! `.font_family(..)`, never `.font(gpui::font(..))`: gpui's `Styled::font` also writes the
//! font's weight (normal) and so undoes any `.font_weight(..)` set before it (#204 review).

use std::borrow::Cow;

use gpui::App;

/// The UI font (`--fu`). The family name inside the font file: the "Variable" suffix in
/// `tokens.css` is the name fontsource gives its `@font-face`, not part of the file.
pub const UI_FAMILY: &str = "Schibsted Grotesk";
/// The figures font (`--fm`), by the family name inside the font file.
pub const MONO_FAMILY: &str = "JetBrains Mono";

/// Static weight instances of the two variable fonts, one file per weight `tokens.css` and
/// the React styles use (400, 500, 600, 650) plus 700. DirectWrite draws an embedded
/// variable font at its default instance only (every weight came out 400, #204 review), so
/// each weight is its own face in the same family and DirectWrite matches it by weight.
static FACES: [&[u8]; 10] = [
    include_bytes!("../assets/fonts/schibsted-grotesk-400.ttf"),
    include_bytes!("../assets/fonts/schibsted-grotesk-500.ttf"),
    include_bytes!("../assets/fonts/schibsted-grotesk-600.ttf"),
    include_bytes!("../assets/fonts/schibsted-grotesk-650.ttf"),
    include_bytes!("../assets/fonts/schibsted-grotesk-700.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-400.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-500.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-600.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-650.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-700.ttf"),
];

#[cfg(test)]
pub const SCHIBSTED_OFL: &str = include_str!("../assets/fonts/SchibstedGrotesk-OFL.txt");
#[cfg(test)]
pub const JETBRAINS_OFL: &str = include_str!("../assets/fonts/JetBrainsMono-OFL.txt");
// The woff2 files are what the React editor ships (the latin `wght` subsets). DirectWrite
// rejects woff2, so the static TTF faces above are made from them with fontTools:
// `instantiateVariableFont(TTFont(woff2), {"wght": w})`, `OS/2.usWeightClass = w`, name IDs
// 1/2/4/6/16/17 set to the family and weight (16 = "Schibsted Grotesk" / "JetBrains Mono").
// The hash test checks the woff2 copies against node_modules.
#[cfg(test)]
static SCHIBSTED_WOFF2: &[u8] = include_bytes!("../assets/fonts/schibsted-grotesk-variable.woff2");
#[cfg(test)]
static JETBRAINS_WOFF2: &[u8] = include_bytes!("../assets/fonts/jetbrains-mono-variable.woff2");

/// Register every embedded face with the app's text system.
pub fn register(cx: &App) {
    cx.text_system()
        .add_fonts(FACES.iter().map(|f| Cow::Borrowed(*f)).collect())
        .expect("embedded fonts");
}

/// The OpenType features every window's text draws with: `kern` on. gpui's
/// DirectWrite layout applies no pair kerning until a typography names it
/// (measured: "STATUS" and "Go to Effects" came out 3 px and 1 px wider than
/// Chrome's, which kerns; the font's GPOS pairs T-A and A-T are -1.57 px
/// each at 15 px).
pub fn kerning() -> gpui::FontFeatures {
    gpui::FontFeatures(std::sync::Arc::new(vec![("kern".to_string(), 1)]))
}

/// Turn kerning on for `element` and every text under it (the shell's root
/// element): gpui inherits the feature list down the element tree the way CSS
/// inherits `font-kerning`.
pub fn with_kerning<E: gpui::Styled>(mut element: E) -> E {
    element
        .text_style()
        .get_or_insert_with(Default::default)
        .font_features = Some(kerning());
    element
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
    use gpui::{Styled, div};

    #[test]
    fn the_shell_root_turns_kerning_on_for_every_text_under_it() {
        let mut root = with_kerning(div());
        let features = root
            .text_style()
            .as_ref()
            .and_then(|s| s.font_features.clone())
            .expect("kerning is set on the root text style");
        assert_eq!(features.tag_value_list(), &[("kern".to_string(), 1)]);
    }

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

    /// The table with `tag` in an sfnt file, from its table directory.
    fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
        let be32 = |o: usize| u32::from_be_bytes(font[o..o + 4].try_into().unwrap()) as usize;
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        (0..count).map(|i| 12 + i * 16).find(|&r| &font[r..r + 4] == tag).map(|r| {
            let (off, len) = (be32(r + 8), be32(r + 12));
            &font[off..off + len]
        })
    }

    /// Every embedded face is a static instance (no `fvar`) with the weight class its slot
    /// promises, so DirectWrite has a real face for each weight the views ask for.
    #[test]
    fn every_face_is_static_with_its_weight() {
        let weights = [400u16, 500, 600, 650, 700];
        for (i, face) in FACES.iter().enumerate() {
            assert!(table(face, b"fvar").is_none(), "face {i} is still a variable font");
            let os2 = table(face, b"OS/2").expect("OS/2 table");
            let class = u16::from_be_bytes([os2[4], os2[5]]);
            assert_eq!(class, weights[i % 5], "face {i} weight class");
        }
    }

    /// No view sets its font with `.font(..)`: gpui's `Styled::font` writes the weight too
    /// and resets an earlier `.font_weight(..)` to normal, which drew every bold label in the
    /// editor at 400. Families go through `.font_family(..)`.
    #[test]
    fn no_view_resets_the_font_weight() {
        // Built at run time so this test's own source does not match.
        let needle = [".", "font("].concat();
        fn walk(dir: &std::path::Path, needle: &str, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read src") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, needle, hits);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).expect("read source");
                    for (n, line) in text.lines().enumerate() {
                        if line.contains(needle) && !line.trim_start().starts_with("//") {
                            hits.push(format!("{}:{}", path.display(), n + 1));
                        }
                    }
                }
            }
        }
        let mut hits = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &needle, &mut hits);
        assert!(hits.is_empty(), "set the family with font_family, not {needle}..): {hits:?}");
    }

    #[test]
    fn both_ofl_licences_are_embedded() {
        assert!(SCHIBSTED_OFL.contains("SIL Open Font License"));
        assert!(JETBRAINS_OFL.contains("SIL Open Font License"));
    }
}
