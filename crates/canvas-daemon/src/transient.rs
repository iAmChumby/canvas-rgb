//! The onboarding effects are transient.
//!
//! `onboard-hello` and `onboard-sweep` are a greeting, not a look the owner chose. When one is
//! activated the layout keeps its own saved effect (nothing about the onboarding effect is
//! written to the layouts or the effects file), and a timer restores that effect once the
//! onboarding effect has played and [`GRACE`] has passed with no new activate or identify
//! request. A window closed mid-flow therefore never leaves the lights on the greeting.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use canvas_proto::Rgb8;

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
            if app.identify.session_active() {
                // An onboarding session holds the lights; check again after another grace.
                drop(inner);
                spawn_timer(app.clone(), generation, GRACE);
                return;
            }
            inner.pending = false;
        }
        restore(&app).await;
    });
}

/// Puts the layout's saved effect back on the lights (black when the layout has none).
pub async fn restore(app: &Arc<App>) {
    let saved = app.active_layout().await.and_then(|l| l.effect);
    let result = match saved {
        Some(effect) => crate::apply_effect(app, &effect.id, &effect.props).await,
        None => {
            let mut props = canvas_proto::Props::new();
            props.insert("color".to_string(), canvas_proto::PropValue::Color(Rgb8::BLACK));
            crate::apply_effect(app, "solid", &props).await
        }
    };
    match result {
        Ok(()) => tracing::info!("the onboarding effect ended; the saved effect is back"),
        Err(e) => tracing::warn!(error = %e, "could not restore the saved effect after onboarding"),
    }
}
