//! What every adapter does when the machine resumes from sleep.
//!
//! The class `cause:sleep-state` (#5, #29): hardware that falls back to its own state over a
//! sleep must be put back into the host-driven one, and an adapter that has no such state must
//! say so instead of growing a fake mode to restore.
//!
//! Every adapter implements [`ResumeBehavior`]. The method has no default, and the places where
//! the daemon takes an adapter ask for the trait in a bound (`App::new`, `App::with_displays`,
//! `DisplayHub::new`, and the daemon's `Peripheral` trait in `crates/canvas-daemon/src/`). An
//! adapter the daemon is given without it does not compile. The bound lives in the daemon and
//! not on the spine traits (`DeviceAdapter` and the others in `canvas-proto`), because the spine
//! is the shared contract of every area and cannot depend on this crate. An adapter that is
//! never handed to the daemon is not checked by the compiler. The guard test (`crates/canvas-daemon/tests/resume_guard.rs`) covers the rest: it
//! drives the adapters on the list, and a scan fails when an adapter is missing from it.
//!
//! The choice is checked too. The guard test shows that an adapter that sets its mode again
//! really does after a resume. For the adapters with nothing to restore, each adapter's own
//! tests call the production `note_resume()` on its fake transport and show that the next
//! operation sends what it would have sent anyway.

/// What an adapter does when the machine resumes from sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnResume {
    /// The adapter has mode state that a sleep can reset: it sets the mode again after a
    /// resume, before the next frame or the next write.
    SetsModeAgain,
    /// The adapter has no mode state: a sleep cannot reset anything, so nothing is restored.
    /// An adapter with this must not write anything on a resume.
    NothingToRestore,
}

/// The resume contract of every adapter.
///
/// There is no default on purpose: an adapter has to say what it does on a resume. The daemon
/// asks for this trait wherever it takes an adapter, so an adapter without it does not compile.
/// The daemon's guard test walks every adapter the daemon can build and checks the answer.
pub trait ResumeBehavior {
    /// What this adapter does when the machine resumes from sleep.
    fn on_resume(&self) -> OnResume;
}
