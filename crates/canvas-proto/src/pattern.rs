//! Patterns (0.16.0): a colour per LED, built on one kind of device, played identically by every
//! device of that kind in a group.
//!
//! A pattern is an effect whose id is [`PATTERN_ID_PREFIX`] + its name, so a group plays it
//! through the existing `Plays::Effect` and no new kind of `Plays` exists. The file
//! (`patterns.json`, [`PatternsFile`]) already has room for frames and motion: 0.16.0 writes one
//! frame and `{"type":"none"}`, and keeps every field it does not know, so a later version may add
//! fields and never has to change the meaning of one written here.
//!
//! The design is `docs/superpowers/specs/2026-11-12-patterns-design.md`.

use crate::color::Rgb8;
use crate::scene::{Arrangement, LightId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The id prefix of a pattern, as `stack:` is for a stack.
pub const PATTERN_ID_PREFIX: &str = "pattern:";
/// The patterns file, beside the other config files.
pub const PATTERNS_FILE: &str = "patterns.json";
/// The `schema_version` of `patterns.json`. Fields are only ever added, so it stays 1.
pub const PATTERNS_SCHEMA: u32 = 1;
/// The longest a pattern's name may be, in characters, before the prefix.
pub const PATTERN_NAME_MAX: usize = 60;
/// The most LEDs a pattern may be built on (the largest zone the engine maps).
pub const PATTERN_LEDS_MAX: u32 = 4096;
/// The most frames a file may hold for one pattern. 0.16.0 writes one.
pub const PATTERN_FRAMES_MAX: usize = 256;
/// The lit group of the editor's takeover.
pub const PATTERN_EDIT_GROUP: &str = "pattern-edit";
/// How bright the other lights are while the editor is open, in percent.
pub const PATTERN_EDIT_DIM_PERCENT: u8 = 20;

/// The cells of a grid pattern, for a later two-dimensional fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GridSize {
    pub width: u32,
    pub height: u32,
}

/// The kind of device a pattern was built on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BuiltOn {
    pub led_count: u32,
    pub arrangement: Arrangement,
    /// The grid's size, for a grid pattern. `None` for every other arrangement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub grid: Option<GridSize>,
}

/// One frame: a colour per LED, in position order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Frame {
    pub colors: Vec<Rgb8>,
    /// Milliseconds this frame holds. Unused in 0.16.0, which writes 0.
    #[serde(default)]
    pub hold: u32,
    /// Fields a later version wrote. Kept as they are.
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// How the frames move. `{"type":"none"}` is the only value 0.16.0 writes; any other `type`, with
/// its fields, is kept and plays as none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Motion {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    #[ts(skip)]
    pub params: serde_json::Map<String, serde_json::Value>,
}

impl Motion {
    /// No motion: the first frame stays.
    pub fn none() -> Self {
        Self { kind: "none".into(), params: serde_json::Map::new() }
    }

    pub fn is_none(&self) -> bool {
        self.kind == "none"
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::none()
    }
}

/// One pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Pattern {
    /// [`PATTERN_ID_PREFIX`] + the name. A save never renames.
    pub id: String,
    pub built_on: BuiltOn,
    pub frames: Vec<Frame>,
    #[serde(default = "Motion::none")]
    pub motion: Motion,
    /// Fields a later version wrote. Kept as they are.
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Pattern {
    /// The id without its prefix.
    pub fn name(&self) -> &str {
        self.id.strip_prefix(PATTERN_ID_PREFIX).unwrap_or(&self.id)
    }

    /// The frame 0.16.0 plays. [`validate_pattern`] guarantees there is one.
    ///
    /// # Panics
    /// On a pattern with no frames, which validation refuses.
    pub fn first_frame(&self) -> &Frame {
        &self.frames[0]
    }
}

/// `patterns.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PatternsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub patterns: Vec<Pattern>,
    /// Fields a later version wrote. Kept as they are.
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for PatternsFile {
    fn default() -> Self {
        Self { schema_version: PATTERNS_SCHEMA, patterns: Vec::new(), extra: serde_json::Map::new() }
    }
}

/// Why a pattern is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// The id does not start with [`PATTERN_ID_PREFIX`].
    BadId,
    NameEmpty,
    NameTooLong,
    NoFrames,
    TooManyFrames { count: usize },
    /// `built_on.led_count` is 0 or above [`PATTERN_LEDS_MAX`].
    LedCount { count: u32 },
    /// A frame's colours do not match `built_on.led_count`.
    FrameLength { frame: usize, found: usize, expected: usize },
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatternError::BadId => write!(f, "a pattern's id starts with \"{PATTERN_ID_PREFIX}\""),
            PatternError::NameEmpty => write!(f, "a pattern needs a name"),
            PatternError::NameTooLong => {
                write!(f, "a pattern's name is at most {PATTERN_NAME_MAX} characters")
            }
            PatternError::NoFrames => write!(f, "a pattern needs a frame"),
            PatternError::TooManyFrames { count } => {
                write!(f, "a pattern holds at most {PATTERN_FRAMES_MAX} frames, not {count}")
            }
            PatternError::LedCount { count } => {
                write!(f, "a pattern is built on 1 to {PATTERN_LEDS_MAX} LEDs, not {count}")
            }
            PatternError::FrameLength { frame, found, expected } => write!(
                f,
                "frame {} has {found} colours; the device it was built on has {expected} LEDs",
                frame + 1
            ),
        }
    }
}

impl std::error::Error for PatternError {}

/// Checks the rules every saved pattern keeps: the prefix and a name of 1 to
/// [`PATTERN_NAME_MAX`] characters, 1 to [`PATTERN_LEDS_MAX`] LEDs, 1 to [`PATTERN_FRAMES_MAX`]
/// frames, and exactly `built_on.led_count` colours in every frame.
pub fn validate_pattern(p: &Pattern) -> Result<(), PatternError> {
    let Some(name) = p.id.strip_prefix(PATTERN_ID_PREFIX) else {
        return Err(PatternError::BadId);
    };
    if name.trim().is_empty() {
        return Err(PatternError::NameEmpty);
    }
    if name.chars().count() > PATTERN_NAME_MAX {
        return Err(PatternError::NameTooLong);
    }
    let count = p.built_on.led_count;
    if count == 0 || count > PATTERN_LEDS_MAX {
        return Err(PatternError::LedCount { count });
    }
    if p.frames.is_empty() {
        return Err(PatternError::NoFrames);
    }
    if p.frames.len() > PATTERN_FRAMES_MAX {
        return Err(PatternError::TooManyFrames { count: p.frames.len() });
    }
    for (i, frame) in p.frames.iter().enumerate() {
        if frame.colors.len() != count as usize {
            return Err(PatternError::FrameLength {
                frame: i,
                found: frame.colors.len(),
                expected: count as usize,
            });
        }
    }
    Ok(())
}

/// How a pattern lands on one light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FitKind {
    /// The same count and arrangement: a copy.
    Exact,
    /// The same arrangement, another count: stretched along it.
    Stretched,
    /// Another arrangement: laid along the LED order.
    OtherShape,
    /// A light of one LED: it takes the pattern's first colour that is on.
    OneLed,
}

/// Rules in this order: the same count and arrangement is [`FitKind::Exact`]; a target of one LED
/// is [`FitKind::OneLed`]; the same arrangement is [`FitKind::Stretched`]; anything else is
/// [`FitKind::OtherShape`].
pub fn fit_kind(built_on: &BuiltOn, target_leds: u32, target: Arrangement) -> FitKind {
    if built_on.led_count == target_leds && built_on.arrangement == target {
        FitKind::Exact
    } else if target_leds == 1 {
        FitKind::OneLed
    } else if built_on.arrangement == target {
        FitKind::Stretched
    } else {
        FitKind::OtherShape
    }
}

/// How the pattern lands on one light of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LightFit {
    pub light: LightId,
    pub fit: FitKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(leds: u32, frames: usize) -> Pattern {
        Pattern {
            id: "pattern:Sunrise".into(),
            built_on: BuiltOn { led_count: leds, arrangement: Arrangement::Ring, grid: None },
            frames: (0..frames)
                .map(|_| Frame {
                    colors: vec![Rgb8 { r: 255, g: 0, b: 0 }; leds as usize],
                    hold: 0,
                    extra: serde_json::Map::new(),
                })
                .collect(),
            motion: Motion::none(),
            extra: serde_json::Map::new(),
        }
    }

    fn on(leds: u32, arrangement: Arrangement) -> BuiltOn {
        BuiltOn { led_count: leds, arrangement, grid: None }
    }

    #[test]
    fn a_one_frame_pattern_round_trips() {
        let p = pattern(12, 1);
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""motion":{"type":"none"}"#), "{json}");
        assert_eq!(serde_json::from_str::<Pattern>(&json).unwrap(), p);
        assert_eq!(p.name(), "Sunrise");
        assert_eq!(validate_pattern(&p), Ok(()));
    }

    #[test]
    fn a_file_with_two_frames_and_a_motion_loads_keeps_unknown_fields_and_plays_its_first_frame() {
        let text = r##"{
            "schema_version": 1,
            "future": {"anything": [1, 2]},
            "patterns": [{
                "id": "pattern:Spin",
                "built_on": {"led_count": 2, "arrangement": "ring"},
                "frames": [
                    {"colors": ["#ff0000", "#00ff00"], "hold": 250, "label": "first"},
                    {"colors": ["#0000ff", "#ffffff"], "hold": 250}
                ],
                "motion": {"type": "rotate", "speed": 2},
                "tags": ["warm"]
            }]
        }"##;
        let file: PatternsFile = serde_json::from_str(text).unwrap();
        let p = &file.patterns[0];
        assert_eq!(validate_pattern(p), Ok(()));
        assert_eq!(p.first_frame(), &p.frames[0]);
        assert_eq!(p.first_frame().colors[0], Rgb8 { r: 255, g: 0, b: 0 });
        assert!(!p.motion.is_none());
        let back = serde_json::to_string(&file).unwrap();
        assert_eq!(serde_json::from_str::<PatternsFile>(&back).unwrap(), file);
        for kept in ["label", "rotate", "speed", "tags", "future"] {
            assert!(back.contains(kept), "{kept} was dropped: {back}");
        }
    }

    #[test]
    fn a_missing_motion_reads_as_none() {
        let p: Pattern = serde_json::from_str(
            r##"{"id":"pattern:A","built_on":{"led_count":1,"arrangement":"single"},"frames":[{"colors":["#000000"]}]}"##,
        )
        .unwrap();
        assert!(p.motion.is_none());
        assert_eq!(p.frames[0].hold, 0);
        assert_eq!(p.built_on.grid, None);
    }

    #[test]
    fn validate_refuses_a_frame_with_the_wrong_number_of_colours() {
        let mut p = pattern(12, 2);
        p.frames[1].colors.pop();
        assert_eq!(validate_pattern(&p), Err(PatternError::FrameLength { frame: 1, found: 11, expected: 12 }));
    }

    #[test]
    fn validate_refuses_no_frames_zero_leds_and_more_than_the_cap() {
        assert_eq!(validate_pattern(&pattern(12, 0)), Err(PatternError::NoFrames));
        assert_eq!(validate_pattern(&pattern(0, 1)), Err(PatternError::LedCount { count: 0 }));
        let mut big = pattern(1, 1);
        big.built_on.led_count = PATTERN_LEDS_MAX + 1;
        assert_eq!(validate_pattern(&big), Err(PatternError::LedCount { count: PATTERN_LEDS_MAX + 1 }));
        let many = pattern(1, PATTERN_FRAMES_MAX + 1);
        assert_eq!(validate_pattern(&many), Err(PatternError::TooManyFrames { count: PATTERN_FRAMES_MAX + 1 }));
        assert_eq!(validate_pattern(&pattern(1, PATTERN_FRAMES_MAX)), Ok(()));
        assert_eq!(validate_pattern(&pattern(PATTERN_LEDS_MAX, 1)), Ok(()));
    }

    #[test]
    fn validate_refuses_an_id_without_the_prefix_and_an_empty_or_long_name() {
        let mut p = pattern(3, 1);
        p.id = "stack:Sunrise".into();
        assert_eq!(validate_pattern(&p), Err(PatternError::BadId));
        p.id = "pattern:  ".into();
        assert_eq!(validate_pattern(&p), Err(PatternError::NameEmpty));
        p.id = format!("pattern:{}", "é".repeat(PATTERN_NAME_MAX));
        assert_eq!(validate_pattern(&p), Ok(()));
        p.id = format!("pattern:{}", "é".repeat(PATTERN_NAME_MAX + 1));
        assert_eq!(validate_pattern(&p), Err(PatternError::NameTooLong));
    }

    #[test]
    fn fit_kind_follows_the_rule_order() {
        let ring12 = on(12, Arrangement::Ring);
        assert_eq!(fit_kind(&ring12, 12, Arrangement::Ring), FitKind::Exact);
        assert_eq!(fit_kind(&ring12, 24, Arrangement::Ring), FitKind::Stretched);
        assert_eq!(fit_kind(&ring12, 12, Arrangement::Strip), FitKind::OtherShape);
        assert_eq!(fit_kind(&ring12, 1, Arrangement::Single), FitKind::OneLed);
        assert_eq!(fit_kind(&on(1, Arrangement::Single), 1, Arrangement::Single), FitKind::Exact);
        assert_eq!(fit_kind(&ring12, 1, Arrangement::Ring), FitKind::OneLed);
    }
}
