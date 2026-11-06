//! Icon assets: Phosphor SVGs from `@phosphor-icons/react` (the same set the
//! React editor renders), embedded as files under `assets/icons/`.
//!
//! The SVG files carry the path data the React package renders (one `<path>`
//! per icon, `fill="currentColor"`, viewBox `0 0 256 256`), so the GPUI icons
//! are drawn from the same shapes. [`IconAssets`] is the `AssetSource` GPUI
//! loads them through.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

/// The icon names the shell and Displays need, matching the React imports.
pub const ICONS: &[&str] = &[
    "SquaresFour",
    "Sparkle",
    "Circuitry",
    "Monitor",
    "GameController",
    "Fan",
    "Gear",
    "FloppyDisk",
    "WarningOctagon",
    "ThermometerSimple",
    "ArrowClockwise",
    "CheckCircle",
    "CircleDashed",
    "MagnifyingGlass",
    "Pause",
    "Play",
    "Warning",
    // The Fans page (#204).
    "DotsThreeOutline",
    "CopySimple",
    "Trash",
    "Eye",
    "EyeSlash",
    "ListChecks",
    "X",
    "Plus",
    "Check-bold",
    "Minus-bold",
];

fn icon_svg(name: &str) -> Option<&'static str> {
    Some(match name {
        "SquaresFour" => include_str!("../assets/icons/SquaresFour.svg"),
        "Sparkle" => include_str!("../assets/icons/Sparkle.svg"),
        "Circuitry" => include_str!("../assets/icons/Circuitry.svg"),
        "Monitor" => include_str!("../assets/icons/Monitor.svg"),
        "GameController" => include_str!("../assets/icons/GameController.svg"),
        "Fan" => include_str!("../assets/icons/Fan.svg"),
        "Gear" => include_str!("../assets/icons/Gear.svg"),
        "FloppyDisk" => include_str!("../assets/icons/FloppyDisk.svg"),
        "WarningOctagon" => include_str!("../assets/icons/WarningOctagon.svg"),
        "ThermometerSimple" => include_str!("../assets/icons/ThermometerSimple.svg"),
        "ArrowClockwise" => include_str!("../assets/icons/ArrowClockwise.svg"),
        "CheckCircle" => include_str!("../assets/icons/CheckCircle.svg"),
        "CircleDashed" => include_str!("../assets/icons/CircleDashed.svg"),
        "MagnifyingGlass" => include_str!("../assets/icons/MagnifyingGlass.svg"),
        "Pause" => include_str!("../assets/icons/Pause.svg"),
        "Play" => include_str!("../assets/icons/Play.svg"),
        "Warning" => include_str!("../assets/icons/Warning.svg"),
        "DotsThreeOutline" => include_str!("../assets/icons/DotsThreeOutline.svg"),
        "CopySimple" => include_str!("../assets/icons/CopySimple.svg"),
        "Trash" => include_str!("../assets/icons/Trash.svg"),
        "Eye" => include_str!("../assets/icons/Eye.svg"),
        "EyeSlash" => include_str!("../assets/icons/EyeSlash.svg"),
        "ListChecks" => include_str!("../assets/icons/ListChecks.svg"),
        "X" => include_str!("../assets/icons/X.svg"),
        "Plus" => include_str!("../assets/icons/Plus.svg"),
        "Check-bold" => include_str!("../assets/icons/Check-bold.svg"),
        "Minus-bold" => include_str!("../assets/icons/Minus-bold.svg"),
        _ => return None,
    })
}

pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"));
        let Some(name) = name else {
            return Ok(None);
        };
        // Weights: the base file is `regular`; `-fill`/`-bold` carry the fill
        // and bold path data the React code asks for with `weight=`.
        if let Some(base) = name.strip_suffix("-fill") {
            return Ok(match icon_svg(base) {
                Some(_) => {
                    let svg = match base {
                        "SquaresFour" => include_str!("../assets/icons/SquaresFour-fill.svg"),
                        "Sparkle" => include_str!("../assets/icons/Sparkle-fill.svg"),
                        "Circuitry" => include_str!("../assets/icons/Circuitry-fill.svg"),
                        "Monitor" => include_str!("../assets/icons/Monitor-fill.svg"),
                        "GameController" => include_str!("../assets/icons/GameController-fill.svg"),
                        "Fan" => include_str!("../assets/icons/Fan-fill.svg"),
                        "Gear" => include_str!("../assets/icons/Gear-fill.svg"),
                        "FloppyDisk" => include_str!("../assets/icons/FloppyDisk-fill.svg"),
                        "WarningOctagon" => {
                            include_str!("../assets/icons/WarningOctagon-fill.svg")
                        }
                        "ThermometerSimple" => {
                            include_str!("../assets/icons/ThermometerSimple-fill.svg")
                        }
                        "ArrowClockwise" => include_str!("../assets/icons/ArrowClockwise-fill.svg"),
                        "CheckCircle" => include_str!("../assets/icons/CheckCircle-fill.svg"),
                        "CircleDashed" => include_str!("../assets/icons/CircleDashed-fill.svg"),
                        "MagnifyingGlass" => {
                            include_str!("../assets/icons/MagnifyingGlass-fill.svg")
                        }
                        "Pause" => include_str!("../assets/icons/Pause-fill.svg"),
                        "Play" => include_str!("../assets/icons/Play-fill.svg"),
                        "Warning" => include_str!("../assets/icons/Warning-fill.svg"),
                        _ => icon_svg(base).unwrap_or(""),
                    };
                    Some(Cow::Borrowed(svg.as_bytes()))
                }
                None => None,
            });
        }
        if let Some(base) = name.strip_suffix("-bold") {
            return Ok(match icon_svg(base) {
                Some(_) => {
                    let svg = match base {
                        "SquaresFour" => include_str!("../assets/icons/SquaresFour-bold.svg"),
                        "Sparkle" => include_str!("../assets/icons/Sparkle-bold.svg"),
                        "Circuitry" => include_str!("../assets/icons/Circuitry-bold.svg"),
                        "Monitor" => include_str!("../assets/icons/Monitor-bold.svg"),
                        "GameController" => include_str!("../assets/icons/GameController-bold.svg"),
                        "Fan" => include_str!("../assets/icons/Fan-bold.svg"),
                        "Gear" => include_str!("../assets/icons/Gear-bold.svg"),
                        "FloppyDisk" => include_str!("../assets/icons/FloppyDisk-bold.svg"),
                        "WarningOctagon" => {
                            include_str!("../assets/icons/WarningOctagon-bold.svg")
                        }
                        "ThermometerSimple" => {
                            include_str!("../assets/icons/ThermometerSimple-bold.svg")
                        }
                        "ArrowClockwise" => include_str!("../assets/icons/ArrowClockwise-bold.svg"),
                        "CheckCircle" => include_str!("../assets/icons/CheckCircle-bold.svg"),
                        "CircleDashed" => include_str!("../assets/icons/CircleDashed-bold.svg"),
                        "MagnifyingGlass" => {
                            include_str!("../assets/icons/MagnifyingGlass-bold.svg")
                        }
                        "Pause" => include_str!("../assets/icons/Pause-bold.svg"),
                        "Play" => include_str!("../assets/icons/Play-bold.svg"),
                        "Warning" => include_str!("../assets/icons/Warning-bold.svg"),
                        _ => icon_svg(base).unwrap_or(""),
                    };
                    Some(Cow::Borrowed(svg.as_bytes()))
                }
                None => None,
            });
        }
        Ok(icon_svg(name).map(|svg| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .map(|name| SharedString::from(format!("icons/{name}.svg")))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_embeds_an_svg_with_phosphor_path_data() {
        for name in ICONS {
            let svg = icon_svg(name).unwrap_or_else(|| panic!("icon {name} missing"));
            assert!(svg.contains("<path"), "{name} has no path");
            assert!(svg.contains("currentColor"), "{name} is not currentColor");
            assert!(svg.contains("0 0 256 256"), "{name} is not the 256 box");
        }
    }
}
