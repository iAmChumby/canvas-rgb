//! canvas-rgb's native effects. Spine stub: the Effects area replaces the bodies, never
//! the signatures.

use canvas_proto::{Effect, EffectFactory};

/// Exactly the 12 effects in `canvas_proto::EFFECT_IDS`, in that order.
pub fn registry() -> Vec<EffectFactory> {
    Vec::new()
}

/// Creates the effect with this id (`"poison"`), or `None`.
pub fn create(id: &str) -> Option<Box<dyn Effect>> {
    registry().into_iter().map(|f| f()).find(|e| e.schema().id == id)
}
