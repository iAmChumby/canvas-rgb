//! The text-advance probe (#240): the width GPUI's own text layout gives a
//! word, next to the width the browser gives the same word.
//!
//! The GPUI editor embeds static instances of the two variable fonts (see
//! `fonts.rs`), while Chromium draws the variable font and interpolates the
//! `wght` axis. The two shapers can disagree by a fraction of a pixel per
//! word, which is what this probe measures: it writes one line per word per
//! (size, weight, family) so the numbers can be diffed against the browser's
//! `measureText`.
//!
//! Debug only: it runs when `CANVAS_RGB_TEXT_PROBE` names a file, and does
//! nothing at all otherwise. The file is written once, when the window opens.

use std::io::Write;
use std::path::PathBuf;

use gpui::Window;

use crate::fonts::{MONO_FAMILY, UI_FAMILY};

/// The words the phone chrome draws: the dock's page names, the strip's
/// title and the now-playing name.
const WORDS: &[&str] = &[
    "Nothing",
    "is",
    "running",
    "Nothing is running",
    "Layout",
    "Effects",
    "Devices",
    "Displays",
    "Fans",
    "Settings",
    "Updates",
    "Peripherals",
    "All connected",
    "Pause lights",
];

/// Write the probe's measurements, if `CANVAS_RGB_TEXT_PROBE` names a file.
pub fn maybe_write(window: &Window) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(path) = std::env::var("CANVAS_RGB_TEXT_PROBE") else {
        return;
    };
    if path.trim().is_empty() {
        return;
    }
    write_to(window, &PathBuf::from(path));
}

/// The measurement itself: every word at every (size, weight, family) the
/// phone chrome uses, one line each.
fn write_to(window: &Window, path: &PathBuf) {
    let mut out = String::from("# GPUI text advance probe (#240)\n");
    out.push_str("# style\tsize\tweight\ttext\twidth_px\n");
    let styles: &[(f32, gpui::FontWeight, &str, &str)] = &[
        (14.0, gpui::FontWeight::MEDIUM, UI_FAMILY, "ui"),
        (14.0, gpui::FontWeight::SEMIBOLD, UI_FAMILY, "ui"),
        (20.0, gpui::FontWeight::SEMIBOLD, UI_FAMILY, "ui"),
        (14.0, gpui::FontWeight::NORMAL, MONO_FAMILY, "mono"),
        (15.0, gpui::FontWeight::NORMAL, UI_FAMILY, "ui"),
    ];
    for (size, weight, family, kind) in styles {
        for word in WORDS {
            let w = crate::shell::text_width(window, word, *size, *weight, family);
            out.push_str(&format!(
                "{kind}\t{size}\t{}\t{word}\t{w:.4}\n",
                weight_num(*weight)
            ));
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::File::create(path) {
        let _ = file.write_all(out.as_bytes());
    }
}

/// The weight as a number, so the file reads like the CSS it is compared with.
fn weight_num(weight: gpui::FontWeight) -> u16 {
    match weight {
        gpui::FontWeight::NORMAL => 400,
        gpui::FontWeight::MEDIUM => 500,
        gpui::FontWeight::SEMIBOLD => 600,
        gpui::FontWeight::BOLD => 700,
    }
}
