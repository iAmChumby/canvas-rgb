//! A marker that says "this thread is inside a guarded effect call" (#99).
//!
//! The engine runs every call into effect code behind a guard (`catch_unwind`), but a
//! panic hook runs *before* `catch_unwind` catches. The daemon's hook closes its log on
//! a main-thread panic because a main-thread panic normally ends the process; at startup
//! the saved effect is restored with `apply_effect` over `runtime.block_on` on the main
//! thread, so a saved effect whose `init` panics is caught by the engine while the hook
//! has already closed logging for the rest of the daemon's life. The hook asks
//! [`in_guarded_call`] first: when it is true the panic is a caught effect panic, so the
//! hook logs it at WARN and leaves logging open.
//!
//! The marker is a thread-local counter, set for the duration of every guarded effect
//! call by [`run_guarded`]. It is an RAII guard, so a panic that unwinds through it
//! resets it (the hook itself fires before the unwind and still sees it).

use std::cell::Cell;

thread_local! {
    /// How many guarded effect calls are active on this thread.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// True while this thread is running effect code behind the engine's guard.
///
/// The daemon's panic hook asks this first: a panic with the marker set is a caught
/// effect panic, never the daemon's own fatal panic.
pub fn in_guarded_call() -> bool {
    DEPTH.with(|depth| depth.get() > 0)
}

/// Runs `f` with the marker set, clearing it when `f` returns or unwinds.
pub fn run_guarded<R>(f: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _guard = Guard;
    f()
}

#[cfg(test)]
mod tests {
    use super::{in_guarded_call, run_guarded};

    #[test]
    fn the_marker_is_set_only_inside_a_guarded_call() {
        assert!(!in_guarded_call());
        assert!(run_guarded(in_guarded_call));
        assert!(!in_guarded_call());
        run_guarded(|| {
            assert!(in_guarded_call());
            run_guarded(|| assert!(in_guarded_call()));
            assert!(in_guarded_call());
        });
        assert!(!in_guarded_call());
    }

    #[test]
    fn the_marker_clears_when_a_guarded_call_unwinds() {
        let caught = std::panic::catch_unwind(|| {
            run_guarded(|| panic!("guarded boom"))
        });
        assert!(caught.is_err());
        assert!(!in_guarded_call(), "the guard cleared while unwinding");
    }
}
