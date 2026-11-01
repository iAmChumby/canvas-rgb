//! What every adapter does when the machine resumes from sleep.
//!
//! The class `cause:sleep-state` (#5, #29): hardware that falls back to its own state over a
//! sleep must be put back into the host-driven one, and an adapter that has no such state must
//! say so instead of growing a fake mode to restore.
//!
//! Every adapter implements [`ResumeBehavior`]. The method has no default, so an adapter that
//! is added has to choose before it can be used. The guard test
//! (`crates/canvas-daemon/tests/resume_guard.rs`) then checks that the choice is true: an
//! adapter that says it sets its mode again really does set it again after a resume, and one
//! that says it has nothing to restore stays quiet.

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
/// There is no default on purpose: an adapter has to say what it does on a resume, so "I forgot
/// the resume" cannot reach a user. The daemon's guard test walks every adapter the daemon can
/// build and checks the answer.
pub trait ResumeBehavior {
    /// What this adapter does when the machine resumes from sleep.
    fn on_resume(&self) -> OnResume;
}
