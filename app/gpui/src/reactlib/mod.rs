//! The React web app's libraries, ported as plain Rust.
//!
//! `app/web/src/lib/{stackLayers,stackPreview,stackRoute,stackFrame}.ts` and
//! `app/web/src/views/effects/stackDraft.ts` are pure modules (no DOM), so
//! their rules port one for one and are pinned by unit tests.

pub mod stack_frame;
pub mod stack_layers;
pub mod stack_preview;
pub mod stack_route;
