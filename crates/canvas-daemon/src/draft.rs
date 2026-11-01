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
    /// How a draft is built. The daemon uses [`crate::user_effects::html_builder`]; a test
    /// swaps in a fake, and one that can be held open to interleave a PUT with an activation.
    builder: crate::user_effects::Builder,
}

impl Default for Draft {
    fn default() -> Self {
        Self::with_clock(monotonic_clock())
    }
}

impl Draft {
    /// A draft slot measured against `clock`. The tests pass a clock they move by hand.
    pub fn with_clock(clock: Clock) -> Self {
        Self::with_builder(clock, crate::user_effects::html_builder())
    }

    /// A draft slot with its own builder, for a test that holds a build open.
    pub fn with_builder(clock: Clock, builder: crate::user_effects::Builder) -> Self {
        Self { slot: Mutex::new(None), gate: tokio::sync::Mutex::new(()), clock, builder }
    }

    /// The builder a draft's HTML is built with.
    pub fn builder(&self) -> crate::user_effects::Builder {
        self.builder.clone()
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

/// What ending a draft did to the lights.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ended {
    /// A sentence for the answer when the prior effect could not be put back and black was
    /// used instead. `None` when the prior (or the no-prior black) went on normally.
    pub note: Option<String>,
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
    let mut existing = app.draft.snapshot();

    // An expired slot is over, whatever its source: end it before this PUT is handled, so the
    // new draft takes the effect that is on the lights now as its prior instead of renewing a
    // draft whose lights should already be back.
    let now = app.draft.now();
    if existing.as_ref().is_some_and(|slot| now >= slot.lease_until) {
        let ended = app.draft.lock().take();
        if let Some(slot) = ended {
            let outcome = restore(app, &slot).await;
            tracing::info!(
                draft = %slot.id,
                note = outcome.note.as_deref().unwrap_or(""),
                "the expired draft ended before a new PUT was handled"
            );
        }
        existing = None;
    }

    // The layout is read only when the draft starts: a refresh keeps the prior it took over
    // from, even if something changed the layout meanwhile.
    let prior = match &existing {
        Some(slot) => slot.prior.clone(),
        None => app.active_layout().await.and_then(|layout| layout.effect),
    };
    let lease_until = now + canvas_proto::consts::DRAFT_LEASE;

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
    let build = app.draft.builder();
    let effect = tokio::task::spawn_blocking(move || build(&build_id, &build_source))
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
    // A draft started after an onboarding greeting outranks its restore: the owner is choosing
    // a look, and ending the draft puts the layout's effect back anyway.
    app.transient.cancel();
    tracing::info!(draft = %slot.id, "a draft is on the lights");
    Ok(slot)
}

/// Stops the draft and puts the prior effect back (the editor's Stop).
///
/// The slot always goes: a Stop, an expiry or a GET after the lease is over never leaves a
/// draft behind. When the prior effect cannot be built (its file was deleted mid-draft, say),
/// black goes on the lights and [`Ended::note`] says so; the answer is still a success.
pub async fn stop(app: &Arc<App>) -> Ended {
    let _gate = app.draft.gate.lock().await;
    let Some(slot) = app.draft.lock().take() else {
        return Ended::default();
    };
    let ended = restore(app, &slot).await;
    tracing::info!(
        draft = %slot.id,
        note = ended.note.as_deref().unwrap_or(""),
        "the draft stopped"
    );
    ended
}

/// Ends a draft whose lease ran out: Stop, without the editor asking. Returns what happened
/// when a draft ended here, `None` when none was due.
///
/// The daemon calls this once a second (see [`spawn_sweeper`]), and `GET /effects/draft` calls it
/// before it answers, so the answer never reports a lease that is over.
pub async fn expire_if_due(app: &Arc<App>) -> Option<Ended> {
    let _gate = app.draft.gate.lock().await;
    let now = app.draft.now();
    let due = {
        let mut guard = app.draft.lock();
        match guard.as_ref() {
            Some(slot) if now >= slot.lease_until => guard.take(),
            _ => None,
        }
    };
    let slot = due?;
    let ended = restore(app, &slot).await;
    tracing::info!(
        draft = %slot.id,
        note = ended.note.as_deref().unwrap_or(""),
        "the draft lease ran out"
    );
    Some(ended)
}

/// Holds the draft gate while a saved effect is put on the lights, after ending a draft.
///
/// A caller that replaces the effect (activate, a layout switch, the first run) keeps this
/// guard across its apply: `put` waits on the gate, so a beat cannot start a draft in the gap
/// between "the draft is gone" and "the saved effect is on", which the apply would then
/// overwrite while the slot said the draft was running. Dropping a draft here restores nothing.
pub struct ReplaceGuard<'a> {
    _gate: tokio::sync::MutexGuard<'a, ()>,
    /// True when a draft was on the lights and was dropped here.
    pub dropped: bool,
}

/// Ends any draft (without restoring it) and holds the gate until the guard is dropped.
pub async fn hold_for_replace(app: &Arc<App>) -> ReplaceGuard<'_> {
    let gate = app.draft.gate.lock().await;
    let dropped = app.draft.lock().take();
    if let Some(slot) = &dropped {
        tracing::info!(draft = %slot.id, "the draft ended: a saved effect is being put on");
    }
    ReplaceGuard { _gate: gate, dropped: dropped.is_some() }
}

/// The answer `GET`/`PUT`/`DELETE /effects/draft` give. A `note` is only set by an ending that
/// could not put the prior effect back.
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
            note: None,
        },
        None => canvas_proto::api::DraftStatus {
            active: false,
            id: None,
            source_hash: None,
            props: Props::new(),
            lease_remaining_ms: 0,
            heartbeat_ms,
            lease_ms,
            note: None,
        },
    }
}

/// Puts the effect the draft took over from back on the lights. Never fails: when the prior
/// effect cannot be built (its file was deleted mid-draft), black goes on the lights and the
/// returned note says why, so the draft can still end.
async fn restore(app: &Arc<App>, slot: &Slot) -> Ended {
    if let Some(effect) = &slot.prior {
        match crate::apply_effect(app, &effect.id, &effect.props).await {
            Ok(()) => return Ended::default(),
            Err(error) => {
                tracing::warn!(
                    draft = %slot.id,
                    effect = %effect.id,
                    error = %error,
                    "the prior effect could not be restored; the lights are black"
                );
                let note = format!(
                    "the prior effect {} could not be restored, so the lights are black: {error}",
                    effect.id
                );
                let black_note = set_black(app).await;
                return Ended {
                    note: Some(match black_note {
                        Some(black_error) => format!("{note}; and {black_error}"),
                        None => note,
                    }),
                };
            }
        }
    }
    // No prior effect: black is the normal end, as after onboarding.
    Ended { note: set_black(app).await }
}

/// Puts black on the lights. `Some` is why it failed, which is a failure of the daemon and not
/// of the draft.
async fn set_black(app: &Arc<App>) -> Option<String> {
    match crate::apply_effect(app, "solid", &black()).await {
        Ok(()) => None,
        Err(error) => {
            tracing::warn!(error = %error, "the lights could not be set to black");
            Some(format!("the lights could not be set to black: {error}"))
        }
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
