//! The draft on the lights (#2, D2.5): try an editor draft without saving anything.
//!
//! A draft is built with the user-effect builder and handed to the engine the same way a saved
//! effect is (`crate::apply_built`), but it is never written: `save_effects`, `save_layouts` and
//! `layout.effect` are not touched. The slot lives in memory only, so a daemon restart loses it
//! and the layout's saved effect runs again.
//!
//! The editor renews the slot every [`canvas_proto::consts::DRAFT_HEARTBEAT`]; the slot expires
//! [`canvas_proto::consts::DRAFT_LEASE`] after the last PUT. Stop and expiry put the effect the
//! draft took over from back on the lights; an activate or a layout switch drops the slot
//! without restoring, because the owner chose something else.

use canvas_proto::layout::ActiveEffect;
use canvas_proto::props::Props;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::state::App;

/// The clock a lease is measured against. The daemon uses the time since it started; the tests
/// inject a clock they move by hand, so a lease is tested without waiting.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// The daemon's clock: time since the daemon started. It never goes backwards, so a clock being
/// set cannot extend or shorten a lease.
fn monotonic_clock() -> Clock {
    let started = std::time::Instant::now();
    Arc::new(move || started.elapsed())
}

/// A draft on the lights, with what it took over from.
#[derive(Clone)]
pub struct Slot {
    /// `draft:` + the source hash. Never a saved effect's id.
    pub id: String,
    /// The HTML the effect was built from. A beat with the same source only updates the props,
    /// so the balance of a five second beat does not reload the file every time.
    pub source: Arc<str>,
    pub source_hash: String,
    pub props: Props,
    /// The effect the active layout had before the draft: what Stop and expiry put back.
    /// `None` when the layout had no effect (the lights go black, as after onboarding).
    pub prior: Option<ActiveEffect>,
    /// When the lease ends, on the draft clock.
    pub lease_until: Duration,
}

/// The draft slot and the lock that orders its transitions.
pub struct Draft {
    slot: Mutex<Option<Slot>>,
    /// Held across a transition (put, stop, expire), so an apply and a restore never
    /// interleave. A restore that landed after a new draft would leave the lights showing the
    /// prior effect while the slot said the draft was running.
    gate: tokio::sync::Mutex<()>,
    /// The daemon's clock; the tests replace it.
    clock: Clock,
}

impl Default for Draft {
    fn default() -> Self {
        Self::with_clock(monotonic_clock())
    }
}

impl Draft {
    /// A draft slot measured against `clock`. The tests pass a clock they move by hand.
    pub fn with_clock(clock: Clock) -> Self {
        Self { slot: Mutex::new(None), gate: tokio::sync::Mutex::new(()), clock }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Slot>> {
        self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The slot, copied, so a reader never holds the lock while it waits on anything.
    pub fn snapshot(&self) -> Option<Slot> {
        self.lock().clone()
    }

    /// True while a draft holds the lights. The lease may be running out; see
    /// [`expire_if_due`].
    pub fn is_active(&self) -> bool {
        self.lock().is_some()
    }

    /// Now, on the draft's clock.
    pub fn now(&self) -> Duration {
        (self.clock)()
    }
}

/// Why a PUT did not leave a draft on the lights.
#[derive(Debug)]
pub enum PutError {
    /// The draft's HTML did not build: the editor's problem (422), and a running draft is left
    /// alone.
    Build(String),
    /// The daemon could not hand the effect over: not the editor's problem (500).
    Internal(String),
}

impl std::fmt::Display for PutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PutError::Build(message) | PutError::Internal(message) => f.write_str(message),
        }
    }
}

/// Puts `html` on the lights as the draft, renews a running draft's lease and updates its props.
///
/// The prior effect is the active layout's effect when the draft starts, and stays the same
/// when a later PUT replaces the draft's source: a draft is one visit to the editor, however
/// many times its code changes.
pub async fn put(app: &Arc<App>, html: &str, props: &Props) -> Result<Slot, PutError> {
    let _gate = app.draft.gate.lock().await;
    let source_hash = crate::user_effects::source_hash(html);
    let id = format!("{}{source_hash}", canvas_proto::consts::DRAFT_ID_PREFIX);
    let existing = app.draft.snapshot();
    // The layout is read only when the draft starts: a refresh keeps the prior it took over
    // from, even if something changed the layout meanwhile.
    let prior = match &existing {
        Some(slot) => slot.prior.clone(),
        None => app.active_layout().await.and_then(|layout| layout.effect),
    };
    let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;

    if let Some(slot) = &existing {
        if slot.source_hash == source_hash {
            // The same draft: only the lease, and maybe the props, moved. The engine's
            // instance is re-initialised rather than rebuilt, so a beat does not reload the
            // file's scripts.
            let props_changed = slot.props != *props;
            {
                let mut guard = app.draft.lock();
                if let Some(slot) = guard.as_mut() {
                    slot.props = props.clone();
                    slot.lease_until = lease_until;
                }
            }
            if props_changed {
                app.engine.set_props(props);
            }
            return app.draft.snapshot().ok_or_else(|| {
                PutError::Internal("the draft ended while its props were being set".to_string())
            });
        }
    }

    // A new source. Building it runs the file's scripts, so it goes on a blocking thread.
    let build_id = id.clone();
    let build_source = html.to_string();
    let effect = tokio::task::spawn_blocking(move || {
        let build = crate::user_effects::html_builder();
        build(&build_id, &build_source)
    })
    .await
    .map_err(|e| PutError::Internal(format!("the draft load task failed: {e}")))?
    .map_err(PutError::Build)?;
    crate::apply_built(app, effect, props)
        .await
        .map_err(PutError::Internal)?;

    let slot = Slot {
        id,
        source: Arc::from(html),
        source_hash,
        props: props.clone(),
        prior,
        lease_until,
    };
    *app.draft.lock() = Some(slot.clone());
    tracing::info!(draft = %slot.id, "a draft is on the lights");
    Ok(slot)
}

/// Stops the draft and puts the prior effect back (the editor's Stop).
///
/// A restore that fails keeps the slot, so the banner's Retry (another DELETE) or the sweeper
/// tries again instead of leaving the draft on the lights with nothing to end it.
pub async fn stop(app: &Arc<App>) -> Result<(), String> {
    let _gate = app.draft.gate.lock().await;
    let Some(slot) = app.draft.lock().take() else {
        return Ok(());
    };
    match restore(app, &slot).await {
        Ok(()) => {
            tracing::info!(draft = %slot.id, "the draft stopped; the prior effect is back");
            Ok(())
        }
        Err(error) => {
            let mut guard = app.draft.lock();
            if guard.is_none() {
                *guard = Some(slot);
            }
            Err(error)
        }
    }
}

/// Ends a draft whose lease ran out: Stop, without the editor asking. Returns true when a draft
/// ended here.
///
/// The daemon calls this once a second (see [`spawn_sweeper`]), and `GET /effects/draft` calls it
/// before it answers, so the answer never reports a lease that is over.
pub async fn expire_if_due(app: &Arc<App>) -> bool {
    let _gate = app.draft.gate.lock().await;
    let now = app.draft.now();
    let due = {
        let mut guard = app.draft.lock();
        match guard.as_ref() {
            Some(slot) if now >= slot.lease_until => guard.take(),
            _ => None,
        }
    };
    let Some(slot) = due else {
        return false;
    };
    match restore(app, &slot).await {
        Ok(()) => {
            tracing::info!(draft = %slot.id, "the draft lease ran out; the prior effect is back");
            true
        }
        Err(error) => {
            tracing::warn!(
                draft = %slot.id,
                error = %error,
                "the draft lease ran out but the prior effect could not be restored"
            );
            // Keep it, so the next sweep tries again.
            let mut guard = app.draft.lock();
            if guard.is_none() {
                *guard = Some(slot);
            }
            false
        }
    }
}

/// Ends the draft without restoring: an activate or a layout switch chose something else, and
/// that something is already (or is about to be) on the lights. Returns true when a draft ended.
pub async fn drop_slot(app: &Arc<App>) -> bool {
    let _gate = app.draft.gate.lock().await;
    let Some(slot) = app.draft.lock().take() else {
        return false;
    };
    tracing::info!(draft = %slot.id, "the draft ended: something else was chosen");
    true
}

/// The answer `GET`/`PUT`/`DELETE /effects/draft` give.
pub fn status(app: &App) -> canvas_proto::api::DraftStatus {
    let now = app.draft.now();
    let heartbeat_ms = canvas_proto::consts::DRAFT_HEARTBEAT.as_millis() as u64;
    let lease_ms = canvas_proto::consts::DRAFT_LEASE.as_millis() as u64;
    match app.draft.snapshot() {
        Some(slot) => canvas_proto::api::DraftStatus {
            active: true,
            id: Some(slot.id),
            source_hash: Some(slot.source_hash),
            props: slot.props,
            lease_remaining_ms: slot.lease_until.saturating_sub(now).as_millis() as u64,
            heartbeat_ms,
            lease_ms,
        },
        None => canvas_proto::api::DraftStatus {
            active: false,
            id: None,
            source_hash: None,
            props: Props::new(),
            lease_remaining_ms: 0,
            heartbeat_ms,
            lease_ms,
        },
    }
}

/// Puts the effect the draft took over from back on the lights.
async fn restore(app: &Arc<App>, slot: &Slot) -> Result<(), String> {
    match &slot.prior {
        Some(effect) => crate::apply_effect(app, &effect.id, &effect.props).await,
        None => crate::apply_effect(app, "solid", &black()).await,
    }
}

/// The props of the effect that shows when a layout has none: black.
pub fn black() -> Props {
    let mut props = Props::new();
    props.insert(
        "color".to_string(),
        canvas_proto::PropValue::Color(canvas_proto::Rgb8::BLACK),
    );
    props
}

/// How often the daemon checks a lease. One second: the lights come back within
/// [`canvas_proto::consts::DRAFT_LEASE`] plus this, and the check itself is a lock and a compare.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

/// Starts the task that ends a draft whose lease ran out, so the prior effect comes back even
/// when the editor is gone and nobody asks the API.
pub fn spawn_sweeper(app: Arc<App>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            ticker.tick().await;
            if app.shutting_down.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                return;
            }
            expire_if_due(&app).await;
        }
    });
}
