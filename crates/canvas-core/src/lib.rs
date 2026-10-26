//! canvas-rgb engine. Spine stub: the Engine area replaces the body, never the signature.

use canvas_proto::{Engine, EngineSettings};
use std::sync::Arc;

/// Starts the render thread and returns its handle.
pub fn spawn(settings: EngineSettings) -> Arc<dyn Engine> {
    let _ = settings;
    todo!("canvas-core: render loop")
}
