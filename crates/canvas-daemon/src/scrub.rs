//! The shared scrubber, re-exported so older callers keep `canvas_daemon::scrub::Scrubber`.
//!
//! The implementation moved to `canvas-diag` (R1) so the tray and the daemon use one copy.

pub use canvas_diag::scrub::{Scrubber, MACHINE_TOKEN, PROFILE_TOKEN, USER_TOKEN};
