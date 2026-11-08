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
    "SkipBack",
    "SkipForward",
    "Sun",
    "CaretDown",
    "EaselDark",
    "EaselLight",
    "EaselLegs",
    "EaselCanvas",
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
    // The Settings page (#205).
    "CompassTool",
    "DownloadSimple",
    "UploadSimple",
    "FolderOpen",
    "SpeakerHigh",
    // The Devices page (#209).
    "Cpu",
    "Memory",
    "Keyboard",
    "Mouse",
    "Microphone",
    "Lightbulb",
    "GraphicsCard",
    "Plugs",
    "ArrowLeft",
    "ArrowRight",
    // The Peripherals page (#208).
    "BatteryFull",
    "BatteryCharging",
    "ArrowsClockwise",
    "ArrowCounterClockwise",
    "CaretRight",
    // The Effects page (#210).
    "CaretLeft",
    "Code",
    "DotsThree",
    "FileHtml",
    "FilePlus",
    "MonitorPlay",
    "PencilSimple",
    "Star",
    "Waveform",
    "Check",
    // The Layout page (#213).
    "ArrowUUpLeft",
    "ArrowUUpRight",
    "ArrowsOut",
    "Headphones",
    "Minus",
    "SlidersHorizontal",
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
        "SkipBack" => include_str!("../assets/icons/SkipBack.svg"),
        "SkipForward" => include_str!("../assets/icons/SkipForward.svg"),
        "Sun" => include_str!("../assets/icons/Sun.svg"),
        "CaretDown" => include_str!("../assets/icons/CaretDown.svg"),
        "EaselDark" => include_str!("../assets/icons/EaselDark.svg"),
        "EaselLight" => include_str!("../assets/icons/EaselLight.svg"),
        "EaselLegs" => include_str!("../assets/icons/EaselLegs.svg"),
        "EaselCanvas" => include_str!("../assets/icons/EaselCanvas.svg"),
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
        "CompassTool" => include_str!("../assets/icons/CompassTool.svg"),
        "DownloadSimple" => include_str!("../assets/icons/DownloadSimple.svg"),
        "UploadSimple" => include_str!("../assets/icons/UploadSimple.svg"),
        "FolderOpen" => include_str!("../assets/icons/FolderOpen.svg"),
        "SpeakerHigh" => include_str!("../assets/icons/SpeakerHigh.svg"),
        "Cpu" => include_str!("../assets/icons/Cpu.svg"),
        "Memory" => include_str!("../assets/icons/Memory.svg"),
        "Keyboard" => include_str!("../assets/icons/Keyboard.svg"),
        "Mouse" => include_str!("../assets/icons/Mouse.svg"),
        "Microphone" => include_str!("../assets/icons/Microphone.svg"),
        "Lightbulb" => include_str!("../assets/icons/Lightbulb.svg"),
        "GraphicsCard" => include_str!("../assets/icons/GraphicsCard.svg"),
        "Plugs" => include_str!("../assets/icons/Plugs.svg"),
        "ArrowLeft" => include_str!("../assets/icons/ArrowLeft.svg"),
        "ArrowRight" => include_str!("../assets/icons/ArrowRight.svg"),
        "BatteryFull" => include_str!("../assets/icons/BatteryFull.svg"),
        "BatteryCharging" => include_str!("../assets/icons/BatteryCharging.svg"),
        "ArrowsClockwise" => include_str!("../assets/icons/ArrowsClockwise.svg"),
        "ArrowCounterClockwise" => include_str!("../assets/icons/ArrowCounterClockwise.svg"),
        "CaretRight" => include_str!("../assets/icons/CaretRight.svg"),
        "CaretLeft" => include_str!("../assets/icons/CaretLeft.svg"),
        "Code" => include_str!("../assets/icons/Code.svg"),
        "DotsThree" => include_str!("../assets/icons/DotsThree.svg"),
        "FileHtml" => include_str!("../assets/icons/FileHtml.svg"),
        "FilePlus" => include_str!("../assets/icons/FilePlus.svg"),
        "MonitorPlay" => include_str!("../assets/icons/MonitorPlay.svg"),
        "PencilSimple" => include_str!("../assets/icons/PencilSimple.svg"),
        "Star" => include_str!("../assets/icons/Star.svg"),
        "Waveform" => include_str!("../assets/icons/Waveform.svg"),
        "Check" => include_str!("../assets/icons/Check.svg"),
        "ArrowUUpLeft" => include_str!("../assets/icons/ArrowUUpLeft.svg"),
        "ArrowUUpRight" => include_str!("../assets/icons/ArrowUUpRight.svg"),
        "ArrowsOut" => include_str!("../assets/icons/ArrowsOut.svg"),
        "Headphones" => include_str!("../assets/icons/Headphones.svg"),
        "Minus" => include_str!("../assets/icons/Minus.svg"),
        "SlidersHorizontal" => include_str!("../assets/icons/SlidersHorizontal.svg"),
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
                        "CaretDown" => include_str!("../assets/icons/CaretDown-fill.svg"),
                        "CaretLeft" => include_str!("../assets/icons/CaretLeft-fill.svg"),
                        "Code" => include_str!("../assets/icons/Code-fill.svg"),
                        "DotsThree" => include_str!("../assets/icons/DotsThree-fill.svg"),
                        "FileHtml" => include_str!("../assets/icons/FileHtml-fill.svg"),
                        "FilePlus" => include_str!("../assets/icons/FilePlus-fill.svg"),
                        "MonitorPlay" => include_str!("../assets/icons/MonitorPlay-fill.svg"),
                        "PencilSimple" => include_str!("../assets/icons/PencilSimple-fill.svg"),
                        "Star" => include_str!("../assets/icons/Star-fill.svg"),
                        "Waveform" => include_str!("../assets/icons/Waveform-fill.svg"),
                        "Check" => include_str!("../assets/icons/Check-fill.svg"),
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
                        // #204 G8-r3: CaretDown was missing here, so the
                        // source trigger drew the regular caret instead of
                        // the bold one React uses (`Select.tsx:219`).
                        "CaretDown" => include_str!("../assets/icons/CaretDown-bold.svg"),
                        "CaretLeft" => include_str!("../assets/icons/CaretLeft-bold.svg"),
                        "Code" => include_str!("../assets/icons/Code-bold.svg"),
                        "DotsThree" => include_str!("../assets/icons/DotsThree-bold.svg"),
                        "FileHtml" => include_str!("../assets/icons/FileHtml-bold.svg"),
                        "FilePlus" => include_str!("../assets/icons/FilePlus-bold.svg"),
                        "MonitorPlay" => include_str!("../assets/icons/MonitorPlay-bold.svg"),
                        "PencilSimple" => include_str!("../assets/icons/PencilSimple-bold.svg"),
                        "Star" => include_str!("../assets/icons/Star-bold.svg"),
                        "Waveform" => include_str!("../assets/icons/Waveform-bold.svg"),
                        "Check" => include_str!("../assets/icons/Check-bold.svg"),
                        // The Layout page's "place it" key (#213).
                        "Plus" => include_str!("../assets/icons/Plus-bold.svg"),
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
            assert!(svg.contains("<path") || svg.contains("<rect"), "{name} has no path");
            if !name.starts_with("Easel") {
                assert!(svg.contains("currentColor"), "{name} is not currentColor");
                assert!(svg.contains("0 0 256 256"), "{name} is not the 256 box");
            }
        }
    }
}
