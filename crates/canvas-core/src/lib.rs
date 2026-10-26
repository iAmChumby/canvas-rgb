//! canvas-rgb engine: the canvas, the layout → per-LED sample regions, the sampling,
//! the render loop, parking, pause, brightness and the preview downscale.
//!
//! The engine owns one thread. It renders at a fixed step (10..=60 fps), parks when
//! nothing consumes frames, and writes one black frame when it is paused.
//!
//! The public surface is [`spawn`], which returns the [`canvas_proto::Engine`] handle
//! the daemon drives. Everything else is private.

mod engine;
mod geometry;
mod preview;
mod sampling;
mod timer;

#[cfg(test)]
mod bench;
#[cfg(all(test, windows))]
mod cpu;
#[cfg(test)]
mod tests;

use canvas_proto::{Engine, EngineSettings};
use std::sync::Arc;

/// Starts the render thread and returns its handle.
///
/// The settings are the starting point: `fps` is clamped to 10..=60, `brightness` to
/// 0..=100. Everything else can be changed later through the [`Engine`] methods, all
/// of which take effect without a restart.
pub fn spawn(settings: EngineSettings) -> Arc<dyn Engine> {
    engine::spawn(settings)
}
