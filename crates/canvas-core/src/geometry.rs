//! One LED's sample region.
//!
//! Scenes place their lights with the arrangement planner (`arrangement.rs`), which hands back
//! `LedSample`s in the group canvas's units; `sampling.rs` reads the canvas at them. This module
//! holds that one type, and nothing places zones any more: Layout is gone (#310) and a 0.14.0
//! migration (`crates/canvas-daemon/src/scene_migration.rs`) turns saved layouts into scenes.

/// One LED's sample region, in the canvas's own units.
///
/// `center` is where nearest/bilinear sample. `bounds` is the axis-aligned region
/// area-average averages over (already the rotated cell's bounding box when the zone
/// is rotated).
///
/// A LED with no owner is black: `sampled` is false, and it is never sampled. Before
/// parts existed an unclaimed LED still read the canvas's top-left pixel, which is
/// what the adversarial review called out; this flag is what stops it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LedSample {
    /// Sample point (float, may be fractional for bilinear).
    pub center_x: f32,
    /// Sample point (float, may be fractional for bilinear).
    pub center_y: f32,
    /// Area-average region, as `[x0, y0, x1, y1]`.
    pub bounds: [f32; 4],
    /// False when the LED has no owner: it is black and is never sampled.
    pub sampled: bool,
}

impl LedSample {
    /// The LED's sample region, or `None` when it has no owner and is black.
    pub fn region(&self) -> Option<(f32, f32, [f32; 4])> {
        if self.sampled {
            Some((self.center_x, self.center_y, self.bounds))
        } else {
            None
        }
    }
}

impl Default for LedSample {
    /// A LED with no owner: black, and never sampled.
    fn default() -> Self {
        LedSample { center_x: 0.0, center_y: 0.0, bounds: [0.0; 4], sampled: false }
    }
}
