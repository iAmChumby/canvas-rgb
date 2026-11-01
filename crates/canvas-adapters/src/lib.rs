//! canvas-rgb device adapters. Spine stub: the Adapters area replaces the bodies, never
//! the signatures listed in `canvas_proto::traits`.

pub mod attack_shark_x6;
pub mod chione;
pub mod eightbitdo_ultimate2;
pub mod fake;
pub mod openrgb;
mod protocol;
pub mod resume;
pub mod royuan;
pub mod zones;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use canvas_proto::peripheral::ReadProgress;

/// Where an adapter reports the stage of a long device read (see `ReadProgress`). Called from the
/// adapter's blocking thread, so it must be quick and must not wait on the device.
pub type ReadSink = Arc<dyn Fn(ReadProgress) + Send + Sync>;

/// Bumped on every resume from sleep. An adapter that set a hardware mode when it connected
/// compares its own copy with this one and sets the mode again when they differ (#5).
static RESUME_EPOCH: AtomicU64 = AtomicU64::new(0);

/// The current resume count.
pub(crate) fn resume_epoch() -> u64 {
    RESUME_EPOCH.load(Ordering::SeqCst)
}

/// The system resumed from sleep: every device that was put into a host-driven mode may have
/// fallen back to its built-in one. Tells the OpenRGB adapters (SET_CUSTOM_MODE again on the next
/// frame) and bumps the epoch the USB adapters watch. Returns how many OpenRGB adapters were told.
pub fn note_resume() -> usize {
    RESUME_EPOCH.fetch_add(1, Ordering::SeqCst);
    openrgb::reset_custom_mode_all()
}
