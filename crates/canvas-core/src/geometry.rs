//! One LED's sample region.
//!
//! Scenes place their lights with the arrangement planner (`arrangement.rs`), which hands back
//! `LedSample`s in the group canvas's units; `sampling.rs` reads the canvas at them. This module
//! holds that one type, and nothing places zones any more: Layout is gone (#310) and a 0.14.0
//! migration (`crates/canvas-daemon/src/scene_migration.rs`) turns saved layouts into scenes.

// The type moved to the spine in #427 (an effect receives it); core keeps the old path.
pub use canvas_proto::geometry::LedSample;
