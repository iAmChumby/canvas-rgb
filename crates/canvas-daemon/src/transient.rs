//! The onboarding effects are transient.
//!
//! `onboard-hello` and `onboard-sweep` are a greeting, not a look the owner chose. When one is
//! activated the layout keeps its own saved effect (nothing about the onboarding effect is
//! written to the layouts or the effects file), and a timer restores that effect once the
//! onboarding effect has played and [`GRACE`] has passed with no new activate or identify
//! request. A window closed mid-flow therefore never leaves the lights on the greeting.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::identify::IdentifyGate;
use crate::state::App;

/// How long after the effect's own run the daemon waits for another activate or identify.
pub const GRACE: Duration = Duration::from_secs(8);

/// How long an onboarding effect plays before it settles, or `None` for any other effect.
pub fn run_length(id: &str) -> Option<Duration> {
    match id {
        canvas_effects::onboard::HELLO_ID => Some(Duration::from_millis(2_000)),
        canvas_effects::onboard::SWEEP_ID => Some(Duration::from_millis(2_500)),
        _ => None,
    }
}

#[derive(Default)]
struct Inner {
    /// Bumped by every event that cancels or re-arms the timer.
    generation: u64,
    /// True while an onboarding effect is running and a restore is armed.
    pending: bool,
    /// Test hook: every wait is multiplied by this (1.0 in the daemon).
    scale: Option<f64>,
}

/// The transient-effect state, shared by the activate route, identify and the timer.
#[derive(Default)]
pub struct Transient {
    inner: Mutex<Inner>,
}

impl Transient {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Scales every wait (tests only).
    pub fn set_time_scale(&self, scale: f64) {
        self.lock().scale = Some(scale);
    }

    /// True while a restore is armed.
    pub fn is_pending(&self) -> bool {
        self.lock().pending
    }

    fn wait(&self, base: Duration) -> Duration {
        base.mul_f64(self.lock().scale.unwrap_or(1.0))
    }

    /// Any non-onboarding activation: the owner chose something, so no restore is due.
    pub fn cancel(&self) {
        let mut inner = self.lock();
        inner.generation += 1;
        inner.pending = false;
    }
}

/// An onboarding effect was just applied: arms the restore for the end of its run plus the grace.
pub fn arm(app: &Arc<App>, id: &str) {
    let Some(run) = run_length(id) else { return };
    let generation = {
        let mut inner = app.transient.lock();
        inner.generation += 1;
        inner.pending = true;
        inner.generation
    };
    spawn_timer(app.clone(), generation, run + GRACE);
}

/// An identify request arrived: while a restore is armed its wait starts over.
pub fn touch(app: &Arc<App>) {
    let generation = {
        let mut inner = app.transient.lock();
        if !inner.pending {
            return;
        }
        inner.generation += 1;
        inner.generation
    };
    spawn_timer(app.clone(), generation, GRACE);
}

fn spawn_timer(app: Arc<App>, generation: u64, base: Duration) {
    let wait = app.transient.wait(base);
    tokio::spawn(async move {
        tokio::time::sleep(wait).await;
        {
            let mut inner = app.transient.lock();
            if inner.generation != generation || !inner.pending {
                return;
            }
            if holds_lights(&app.identify) {
                // A chain setup holds the lights; check again after another grace.
                drop(inner);
                spawn_timer(app.clone(), generation, GRACE);
                return;
            }
            inner.pending = false;
        }
        restore_armed(&app, generation).await;
    });
}

/// Whether the timer must wait instead of restoring: a chain setup holds the lights. A setup is
/// transient, so the timer asks again after another grace.
fn holds_lights(identify: &IdentifyGate) -> bool {
    identify.holds_lights()
}

/// Puts the active scene back on the lights.
///
/// The scene order is not taken (see [`crate::scenes::apply_active_ungated`]): the draft gate is
/// held across the check and the apply, and a scene write holds the scene order while it waits
/// for that gate.
///
/// A draft on the lights outranks the onboarding restore: the editor is holding them on purpose,
/// and ending that draft puts the same scene back, so this does nothing while a draft runs. The
/// draft gate is held across the check and the apply, so a PUT that is already building is seen
/// before the check and one that arrives during the apply waits for it.
pub async fn restore(app: &Arc<App>) {
    let _gate = app.draft.gate().lock().await;
    restore_locked(app).await;
}

/// [`restore`] for the onboarding timer: the generation is rechecked under the gate before
/// anything is applied (#94).
///
/// The timer releases the transient lock before the gate, so an activation can land while the
/// timer is on its way to the gate. The activating route bumps the generation (`cancel`) just
/// BEFORE it takes the gate (`hold_for_replace`), and re-arms under it. Either way a stale timer
/// applies nothing: if it reaches the gate first it already sees the bumped generation; if the
/// activation holds the gate first, the timer sees the re-armed generation when it gets in.
async fn restore_armed(app: &Arc<App>, generation: u64) {
    let _gate = app.draft.gate().lock().await;
    {
        let inner = app.transient.lock();
        if inner.generation != generation {
            tracing::info!(
                "the onboarding restore is skipped: a newer activation re-armed the timer"
            );
            return;
        }
    }
    restore_locked(app).await;
}

/// [`restore`] with the draft gate already held: the draft check and the apply, exactly as
/// before. `restore` (tests and direct callers) takes the gate and comes here; `restore_armed`
/// rechecks the generation under the gate and only then comes here.
async fn restore_locked(app: &Arc<App>) {
    if app.draft.is_active() {
        tracing::info!("the onboarding restore is skipped: a draft holds the lights");
        return;
    }
    crate::scenes::apply_active_ungated(app, false).await;
    tracing::info!("the onboarding effect ended; the active scene is back");
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::DeviceId;
    use std::time::Instant;

    /// The timer takes this checkpoint after every wait, before it restores: a chain setup in
    /// progress is the owner still choosing, so the restore waits for it (#338).
    #[test]
    fn a_restore_waits_while_a_setup_holds_the_lights() {
        let gate = crate::identify::IdentifyGate::default();
        let now = Instant::now();
        assert!(!holds_lights(&gate), "nothing holds the lights: the restore is due");
        assert!(gate.start_setup(DeviceId("openrgb:board".into()), 0, "s1".into(), now).is_some());
        assert!(holds_lights(&gate), "the setup is open: the restore waits");
        assert!(gate.end_setup("s1"));
        assert!(!holds_lights(&gate), "the setup ended: the restore is due again");
    }
}
