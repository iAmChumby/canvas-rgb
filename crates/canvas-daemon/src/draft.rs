//! The draft on the lights (#2, D2.5): try an editor draft without saving anything.
//!
//! A draft is built with the user-effect builder and handed to the engine the same way a saved
//! effect is (`crate::apply_built`), but it is never written: `save_effects`, `save_layouts` and
//! `layout.effect` are not touched. The slot lives in memory only, so a daemon restart loses it
//! and the layout's saved effect runs again.
//!
//! #93's draft is one HTML file. #95 (D95.4.5) adds a whole stack as a draft, built against the
//! main engine with its layer settings from the request. A stack draft carries a session epoch
//! and a sequence, as the preview does (D95.4.4): a PUT from an older epoch, or a lower sequence
//! in the accepted one, is superseded and answers 200 with `applied: false`.
//!
//! A stack's identity is its structure hash (D95.4.6), so a PUT whose structure is the one
//! already on the lights (or already building) moves values only and starts no build. A build
//! slower than the editor's beat is still the build that lands: every beat with the same
//! structure joins it, and at most one build runs per route.
//!
//! The editor renews the slot every [`canvas_proto::consts::DRAFT_HEARTBEAT`]; the slot expires
//! [`canvas_proto::consts::DRAFT_LEASE`] after the last PUT, and the lease starts when the draft
//! reaches the lights, not when its build started. Stop and expiry put the effect the draft took
//! over from back on the lights; an activate or a layout switch drops the slot without
//! restoring, because the owner chose something else.
//!
//! The gate orders transitions, not builds: a build runs outside it, so Stop and expiry answer
//! while a slow HTML layer loads. The gate is taken to read the slot, to swap it and to apply.

use canvas_proto::layout::ActiveEffect;
use canvas_proto::props::Props;
use canvas_proto::stack::Stack;
use canvas_proto::traits::Effect;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use crate::epoch::EpochBook;
use crate::stack::{LiveMap, LiveStack};
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

/// What a draft is: one HTML file (#2) or a whole stack (#95, D95.4.5).
#[derive(Clone)]
pub enum SlotKind {
    /// `draft:` + the source hash. Never a saved effect's id.
    Html {
        /// The HTML the effect was built from. A beat with the same source only updates the
        /// props, so the balance of a five second beat does not reload the file every time.
        source: Arc<str>,
        source_hash: String,
    },
    /// A stack as the builder holds it, with its structure hash and live map.
    Stack {
        stack_id: String,
        structure: String,
        live: LiveStack,
    },
}

/// A draft on the lights, with what it took over from.
#[derive(Clone)]
pub struct Slot {
    /// `draft:` + the source hash for HTML, or `stack:` + the name for a stack.
    pub id: String,
    pub props: Props,
    /// The effect the active layout had before the draft: what Stop and expiry put back.
    /// `None` when the layout had no effect (the lights go black, as after onboarding).
    pub prior: Option<ActiveEffect>,
    /// When the lease ends, on the draft clock.
    pub lease_until: Duration,
    pub kind: SlotKind,
}

impl Slot {
    /// The identity a `DraftStatus` reports: the source hash for HTML, the structure hash for a
    /// stack (neither is a props or live-value hash, D95.4.6).
    pub fn source_hash(&self) -> &str {
        match &self.kind {
            SlotKind::Html { source_hash, .. } => source_hash,
            SlotKind::Stack { structure, .. } => structure,
        }
    }

    /// The HTML a beat compares, for an HTML draft.
    pub fn html_source(&self) -> Option<&str> {
        match &self.kind {
            SlotKind::Html { source, .. } => Some(source),
            SlotKind::Stack { .. } => None,
        }
    }

    /// The structure hash, for a stack draft.
    pub fn structure(&self) -> Option<&str> {
        match &self.kind {
            SlotKind::Html { .. } => None,
            SlotKind::Stack { structure, .. } => Some(structure),
        }
    }
}

/// The build that last failed and the draft it was for. A beat that carries a failed draft's
/// source again answers that build's error (422) instead of loading a file that cannot load
/// every five seconds; the note is dropped when a draft lands.
struct FailedBuild {
    /// The HTML source the build was given, exactly as the beat gave it.
    source: Arc<str>,
    message: String,
}

/// What a build in flight is building.
#[derive(Clone)]
enum Building {
    /// One HTML file, identified by its source.
    Html { source: Arc<str> },
    /// A whole stack, identified by its structure hash.
    Stack { stack_id: String, structure: String, live: LiveStack },
}

/// The build in flight for the draft route, if one is.
///
/// At most one is here at a time. A PUT whose draft equals it joins it: the props and the live
/// map move, the lease is left for the landing, and no second build starts. A PUT with a
/// different draft re-tokens it, and the build that finishes after that lands nowhere.
#[derive(Clone)]
struct InFlight {
    /// Identifies this build; only the token in the book may land.
    token: u64,
    kind: Building,
    /// The newest props a PUT (or a beat that joined) gave the draft.
    props: Props,
    /// The effect the draft takes over from, read when the build starts.
    prior: Option<ActiveEffect>,
    /// The page whose PUT started a stack build. A stack draft's lease does not run out under
    /// a build the owning page has in flight (D95.4.4). `None` for an HTML draft, which has no
    /// epoch.
    epoch: Option<String>,
}

impl InFlight {
    fn is_html(&self, html: &str) -> bool {
        matches!(&self.kind, Building::Html { source } if &**source == html)
    }

    fn is_stack(&self, stack_id: &str, structure: &str) -> bool {
        matches!(
            &self.kind,
            Building::Stack { stack_id: building, structure: built, .. }
                if building == stack_id && built == structure
        )
    }
}

/// The draft slot and the lock that orders its transitions.
pub struct Draft {
    slot: Mutex<Option<Slot>>,
    /// The note from the last ending, kept until the next draft starts. The sweeper ends a
    /// draft without an answer of its own, so the note has to outlive it for the next
    /// `GET /effects/draft` to say the lights fell back to black.
    last_note: Mutex<Option<String>>,
    /// Held across a transition (put, stop, expire), so an apply and a restore never
    /// interleave. A build does not run under it.
    gate: tokio::sync::Mutex<()>,
    /// The daemon's clock; the tests replace it.
    clock: Clock,
    /// How a draft is built. The daemon uses [`crate::user_effects::html_builder`]; a test
    /// swaps in a fake, and one that can be held open to interleave a PUT with an activation.
    builder: crate::user_effects::Builder,
    /// The epoch order of a stack draft: which page owns the slot and which ones are older.
    epochs: EpochBook,
    /// The build in flight, if one is.
    in_flight: Mutex<Option<InFlight>>,
    /// The token the next build takes.
    tokens: AtomicU64,
    /// Held across a build, so one build runs per route even when drafts change quickly.
    build_lock: tokio::sync::Mutex<()>,
    /// The build that last failed, until a draft lands.
    last_failure: Mutex<Option<FailedBuild>>,
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
        Self {
            slot: Mutex::new(None),
            last_note: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
            clock,
            builder,
            epochs: EpochBook::default(),
            in_flight: Mutex::new(None),
            tokens: AtomicU64::new(0),
            build_lock: tokio::sync::Mutex::new(()),
            last_failure: Mutex::new(None),
        }
    }

    /// Keeps the note of an ending that just happened (or clears it when the prior went back
    /// normally), for the next `status`.
    fn remember_ending(&self, ended: &Ended) {
        *self.last_note.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            ended.note.clone();
    }

    /// A new draft is on the lights: the last ending's note is no longer news.
    fn clear_ending(&self) {
        *self.last_note.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// The note from the last ending, until the next draft starts.
    pub fn last_note(&self) -> Option<String> {
        self.last_note.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// The builder a draft's HTML is built with.
    pub fn builder(&self) -> crate::user_effects::Builder {
        self.builder.clone()
    }

    /// The gate every draft transition takes. A caller that must look at the draft and then
    /// apply its own effect without ending a draft (the onboarding restore) holds this across
    /// both, so no PUT lands between them.
    pub fn gate(&self) -> &tokio::sync::Mutex<()> {
        &self.gate
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

    /// The newest `(epoch, seq)` of a stack draft.
    pub fn watermark(&self) -> Option<(String, u64)> {
        self.epochs.live()
    }

    fn lock_in_flight(&self) -> MutexGuard<'_, Option<InFlight>> {
        self.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The build in flight, copied.
    fn in_flight(&self) -> Option<InFlight> {
        self.lock_in_flight().clone()
    }

    fn in_flight_token(&self) -> Option<u64> {
        self.lock_in_flight().as_ref().map(|building| building.token)
    }

    /// Starts a build and returns its token. A build already in flight is superseded: its
    /// result will find another token and land nowhere.
    fn begin_build(
        &self,
        kind: Building,
        props: &Props,
        prior: Option<ActiveEffect>,
        epoch: Option<String>,
    ) -> u64 {
        let token = self.tokens.fetch_add(1, Ordering::SeqCst) + 1;
        *self.lock_in_flight() =
            Some(InFlight { token, kind, props: props.clone(), prior, epoch });
        token
    }

    /// Gives a beat's props to the build in flight, when `token` still owns it.
    fn update_in_flight(&self, token: u64, props: &Props) {
        if let Some(building) = self.lock_in_flight().as_mut() {
            if building.token == token {
                building.props = props.clone();
            }
        }
    }

    /// Takes the build in flight when `token` still owns it, so the build can land; `None`
    /// when a Stop, an expiry, a replacement or a newer PUT superseded it.
    fn take_in_flight_if(&self, token: u64) -> Option<InFlight> {
        let mut guard = self.lock_in_flight();
        match guard.as_ref() {
            Some(building) if building.token == token => guard.take(),
            _ => None,
        }
    }

    /// A PUT whose draft is already on the lights supersedes any build for another one.
    fn supersede_build(&self) {
        *self.lock_in_flight() = None;
    }

    /// The error the last build of `html` failed with, while nothing has landed since.
    fn last_failure(&self, html: &str) -> Option<String> {
        self.lock_failure()
            .as_ref()
            .filter(|failed| &*failed.source == html)
            .map(|failed| failed.message.clone())
    }

    /// The build of `html` failed: a later beat of the same source answers this, not a reload.
    fn record_failure(&self, html: &str, message: &str) {
        *self.lock_failure() = Some(FailedBuild {
            source: Arc::from(html),
            message: message.to_string(),
        });
    }

    fn clear_failure(&self) {
        *self.lock_failure() = None;
    }

    fn lock_failure(&self) -> MutexGuard<'_, Option<FailedBuild>> {
        self.last_failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Frees the route when a build ends without landing: an error, a join error, or a panic in the
/// blocking task. Every exit path after [`Draft::begin_build`] holds one, so a failed build's
/// token never stays in the book for later beats to join. A build that lands has already taken
/// its token, so the guard is a no-op there; a newer PUT's token is left alone.
struct BuildLease<'a> {
    draft: &'a Draft,
    token: u64,
}

impl<'a> BuildLease<'a> {
    fn new(draft: &'a Draft, token: u64) -> Self {
        Self { draft, token }
    }
}

impl Drop for BuildLease<'_> {
    fn drop(&mut self) {
        self.draft.take_in_flight_if(self.token);
    }
}

/// Why a PUT did not leave a draft on the lights.
#[derive(Debug)]
pub enum PutError {
    /// The draft's HTML did not build: the editor's problem (422), and a running draft is left
    /// alone.
    Build(String),
    /// The stack breaks a rule a Save enforces (422).
    Validation(String),
    /// The daemon could not hand the effect over: not the editor's problem (500).
    Internal(String),
}

impl std::fmt::Display for PutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PutError::Build(message)
            | PutError::Validation(message)
            | PutError::Internal(message) => f.write_str(message),
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
/// many times its code changes. A PUT that is superseded while it built (a Stop, or another
/// source) answers `false` and leaves the newer state alone. A beat whose source is already
/// building joins that build rather than starting another.
pub async fn put(app: &Arc<App>, html: &str, props: &Props) -> Result<bool, PutError> {
    let source_hash = crate::user_effects::source_hash(html);
    let id = format!("{}{source_hash}", canvas_proto::consts::DRAFT_ID_PREFIX);

    let token;
    {
        let _gate = app.draft.gate.lock().await;
        expire_locked(app).await;
        let existing = app.draft.snapshot();

        // The same source and the same kind: only the lease, and maybe the props, moved. The
        // engine's instance is re-initialised rather than rebuilt, so a beat does not reload
        // the file's scripts.
        if let Some(slot) = &existing {
            if slot.html_source() == Some(html) {
                let props_changed = slot.props != *props;
                let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;
                {
                    let mut guard = app.draft.lock();
                    if let Some(slot) = guard.as_mut() {
                        slot.props = props.clone();
                        slot.lease_until = lease_until;
                    }
                }
                // A newer PUT for the draft on the lights supersedes a build for another one.
                app.draft.supersede_build();
                if props_changed {
                    app.engine.set_props(props);
                }
                return Ok(true);
            }
        }

        // The same source is already building: this beat joins it, and the build that started
        // lands with the newest props. A build slower than the beat still applies.
        if let Some(building) = app.draft.in_flight() {
            if building.is_html(html) {
                app.draft.update_in_flight(building.token, props);
                return Ok(true);
            }
        }

        // This source failed to load the last time it was built and nothing has landed since:
        // answer that error instead of loading a file that cannot load on every beat.
        if let Some(message) = app.draft.last_failure(html) {
            return Err(PutError::Build(message));
        }

        // The layout is read only when the draft starts: a refresh keeps the prior it took over
        // from, even if something changed the layout meanwhile.
        let prior = match &existing {
            Some(slot) => slot.prior.clone(),
            None => app.active_layout().await.and_then(|layout| layout.effect),
        };
        token = app.draft.begin_build(
            Building::Html { source: Arc::from(html) },
            props,
            prior,
            None,
        );
    }
    // Every exit path from here frees the route: an error, a join error, or a panic in the
    // blocking task must not leave a token for later beats to join.
    let _lease = BuildLease::new(&app.draft, token);
    // One build per route: a PUT for another source waits for the one in flight to let go.
    let _build = app.draft.build_lock.lock().await;
    if app.draft.in_flight_token() != Some(token) {
        return Ok(false);
    }

    // Building it runs the file's scripts, so it goes on a blocking thread, and the gate is not
    // held while it runs: Stop and expiry answer at once.
    let build_id = id.clone();
    let build_source = html.to_string();
    let build = app.draft.builder();
    let effect = tokio::task::spawn_blocking(move || build(&build_id, &build_source))
        .await
        .map_err(|e| PutError::Internal(format!("the draft load task failed: {e}")))?
        .map_err(|message| {
            app.draft.record_failure(html, &message);
            PutError::Build(message)
        })?;

    let _gate = app.draft.gate.lock().await;
    // A Stop, an expiry, or another PUT landed while this built: this one is stale.
    let Some(building) = app.draft.take_in_flight_if(token) else {
        return Ok(false);
    };
    crate::apply_built(app, effect, &building.props)
        .await
        .map_err(PutError::Internal)?;
    app.draft.clear_failure();
    // The lease starts when the draft reaches the lights, not when its build started: a slow
    // build must not eat the editor's beat.
    let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;

    let slot = Slot {
        id,
        props: building.props,
        prior: building.prior,
        lease_until,
        kind: SlotKind::Html { source: Arc::from(html), source_hash },
    };
    *app.draft.lock() = Some(slot.clone());
    app.draft.clear_ending();
    // A draft started after an onboarding greeting outranks its restore: the owner is choosing
    // a look, and ending the draft puts the layout's effect back anyway.
    app.transient.cancel();
    tracing::info!(draft = %slot.id, "a draft is on the lights");
    Ok(true)
}

/// Puts a whole stack on the lights as the draft (D95.4.5).
///
/// The stack's identity is its structure hash (D95.4.6), so a props, blend, opacity, visibility
/// or area change updates the live map and writes the props through `engine.set_props`, and
/// nothing rebuilds. A structure that differs rebuilds the effect; a build whose epoch/seq is
/// no longer the accepted one lands nowhere and answers `false`.
pub async fn put_stack(
    app: &Arc<App>,
    stack: &Stack,
    overrides: &Props,
    epoch: &str,
    seq: u64,
) -> Result<bool, PutError> {
    // The rules a Save enforces, so the builder never tries a stack the Save would refuse
    // (D95.4.5, item 2): the HTML cap, the layer cap, names, ranges and nested stacks.
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    crate::stacks::validate_preview(&app.stacks, stack, &known).map_err(PutError::Validation)?;

    let structure = crate::preview::structure_of(app, stack);
    let mut props = crate::preview::props_of(app, stack);
    props.extend(overrides.clone());

    let token;
    {
        let _gate = app.draft.gate.lock().await;
        expire_locked(app).await;
        // The newest page owns the slot: an older epoch, or a sequence the route already saw,
        // is superseded (D95.4.4's rule).
        if !app.draft.epochs.offer(epoch, seq) {
            return Ok(false);
        }
        let existing = app.draft.snapshot();

        // The same stack with the same structure: the values changed, not the layers.
        if let Some(slot) = &existing {
            if slot.structure() == Some(structure.as_str()) {
                if let SlotKind::Stack { stack_id, live, .. } = &slot.kind {
                    if stack_id == &stack.id {
                        crate::stack::replace_live(live, stack);
                        let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;
                        {
                            let mut guard = app.draft.lock();
                            if let Some(slot) = guard.as_mut() {
                                slot.props = props.clone();
                                slot.lease_until = lease_until;
                            }
                        }
                        // The structure on the lights is the one the page wants; a build for
                        // another one that is still running is superseded.
                        app.draft.supersede_build();
                        // The live map's lock is dropped before the props reach the engine.
                        app.engine.set_props(&props);
                        app.stacks.note_running(&stack.id, &structure, Arc::clone(live));
                        return Ok(true);
                    }
                }
            }
        }

        // The same structure is already building: this beat joins it. The live map is the one
        // the building effect reads, so the values are there when it lands; its props are the
        // newest, and no second build starts.
        if let Some(building) = app.draft.in_flight() {
            if building.is_stack(&stack.id, &structure) {
                if let Building::Stack { live, .. } = &building.kind {
                    crate::stack::replace_live(live, stack);
                }
                app.draft.update_in_flight(building.token, &props);
                tracing::info!(stack = %stack.id, seq, "a beat joined the stack draft build in flight");
                return Ok(true);
            }
        }

        let prior = match &existing {
            Some(slot) => slot.prior.clone(),
            None => app.active_layout().await.and_then(|layout| layout.effect),
        };
        token = app.draft.begin_build(
            Building::Stack {
                stack_id: stack.id.clone(),
                structure: structure.clone(),
                live: Arc::new(RwLock::new(LiveMap::from_stack(stack, 1))),
            },
            &props,
            prior,
            Some(epoch.to_string()),
        );
    }
    // Every exit path from here frees the route: an error, a join error, or a panic in the
    // blocking task must not leave a token for later beats to join.
    let _lease = BuildLease::new(&app.draft, token);
    // One build per route: a PUT for another structure waits for the one in flight to let go.
    let _build = app.draft.build_lock.lock().await;
    if app.draft.in_flight_token() != Some(token) {
        return Ok(false);
    }
    let building = app.draft.in_flight().expect("the build in flight was read under the build lock");
    let live = match &building.kind {
        Building::Stack { live, .. } => Arc::clone(live),
        Building::Html { .. } => unreachable!("this PUT started a stack build"),
    };

    // A new structure. Its HTML layers load here, outside the gate.
    let build_app = Arc::clone(app);
    let build_stack = stack.clone();
    let effect = tokio::task::spawn_blocking(move || -> Result<Box<dyn Effect>, String> {
        Ok(Box::new(
            crate::stack::StackEffect::new(
                &build_stack,
                crate::leaf_builder(&build_app.user_effects),
                live,
                build_app.stacks.areas(),
            )
            .with_clock(build_app.stacks.clock())
            .with_fps(build_app.stacks.fps_handle())
            .with_status_sink(build_app.stacks.status_sink()),
        ))
    })
    .await
    .map_err(|e| PutError::Internal(format!("the stack draft load task failed: {e}")))?
    .map_err(PutError::Internal)?;

    let _gate = app.draft.gate.lock().await;
    // An older epoch or sequence, a Stop, or another draft that replaced this one while it
    // built: this PUT is stale and must not land over the newer state.
    let Some(building) = app.draft.take_in_flight_if(token) else {
        return Ok(false);
    };
    let live = match building.kind {
        Building::Stack { live, .. } => live,
        Building::Html { .. } => unreachable!("this PUT started a stack build"),
    };
    crate::apply_built(app, effect, &building.props)
        .await
        .map_err(PutError::Internal)?;
    app.draft.clear_failure();
    // The stack is what the main engine runs: a save-then-activate with the same structure
    // reuses it instead of building again (D95.4.3, D95.4.6).
    app.stacks.note_running(&stack.id, &structure, Arc::clone(&live));
    // The lease starts when the draft reaches the lights, not when its build started.
    let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;

    let slot = Slot {
        id: stack.id.clone(),
        props: building.props,
        prior: building.prior,
        lease_until,
        kind: SlotKind::Stack { stack_id: stack.id.clone(), structure, live },
    };
    *app.draft.lock() = Some(slot.clone());
    app.draft.clear_ending();
    app.transient.cancel();
    tracing::info!(draft = %slot.id, seq, "a stack draft is on the lights");
    Ok(true)
}

/// Stops the draft and puts the prior effect back (the editor's Stop).
///
/// The slot always goes: a Stop, an expiry or a GET after the lease is over never leaves a
/// draft behind. When the prior effect cannot be built (its file was deleted mid-draft, say),
/// black goes on the lights and [`Ended::note`] says so; the answer is still a success.
pub async fn stop(app: &Arc<App>) -> Option<Ended> {
    let _gate = app.draft.gate.lock().await;
    // An explicit Stop cancels a build that is still running, even when the first draft has
    // not landed yet: a builder that stopped does not want it on the lights afterwards.
    app.draft.supersede_build();
    // The live sequence goes, the epoch order stays: the tab that stopped can start again with
    // its next PUT, and an older tab still cannot take the lights (D95.4.4, item 4).
    app.draft.epochs.clear_live();
    let slot = app.draft.lock().take()?;
    let ended = restore(app, &slot).await;
    app.draft.remember_ending(&ended);
    tracing::info!(
        draft = %slot.id,
        note = ended.note.as_deref().unwrap_or(""),
        "the draft stopped"
    );
    Some(ended)
}

/// Ends a draft whose lease ran out: Stop, without the editor asking. Returns what happened
/// when a draft ended here, `None` when none was due.
///
/// The daemon calls this once a second (see [`spawn_sweeper`]), and `GET /effects/draft` calls it
/// before it answers, so the answer never reports a lease that is over.
pub async fn expire_if_due(app: &Arc<App>) -> Option<Ended> {
    let _gate = app.draft.gate.lock().await;
    expire_locked(app).await
}

/// [`expire_if_due`] with the gate already held.
async fn expire_locked(app: &Arc<App>) -> Option<Ended> {
    let now = app.draft.now();
    let due = {
        let mut guard = app.draft.lock();
        match guard.as_ref() {
            Some(slot) if now >= slot.lease_until => guard.take(),
            _ => None,
        }
    };
    let slot = due?;
    app.draft.supersede_build();
    app.draft.epochs.clear_live();
    let ended = restore(app, &slot).await;
    app.draft.remember_ending(&ended);
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
    /// The draft that was dropped here. If the caller's own apply then fails, the engine is
    /// left showing a draft nothing owns, so the guard can put this slot's prior effect back
    /// (or black, with a note) instead (D95.4.5, test 12).
    dropped_slot: Option<Slot>,
}

impl ReplaceGuard<'_> {
    /// Puts the dropped draft's prior effect back on the lights, exactly as Stop would, and
    /// remembers the ending's note. `None` when no draft was dropped.
    ///
    /// Called once, by the caller whose apply failed.
    pub async fn restore_dropped(&self, app: &Arc<App>) -> Option<Ended> {
        let slot = self.dropped_slot.as_ref()?;
        let ended = restore(app, slot).await;
        app.draft.remember_ending(&ended);
        tracing::info!(
            draft = %slot.id,
            note = ended.note.as_deref().unwrap_or(""),
            "the draft ended and its prior effect went back after the layout's effect failed"
        );
        Some(ended)
    }
}

/// Ends any draft (without restoring it) and holds the gate until the guard is dropped.
pub async fn hold_for_replace(app: &Arc<App>) -> ReplaceGuard<'_> {
    let gate = app.draft.gate.lock().await;
    let dropped = app.draft.lock().take();
    // The replacement outranks any build still running, landed or not.
    app.draft.supersede_build();
    // The live sequence goes, the epoch order stays, so an older tab cannot re-take the slot
    // after a layout switch or an activation.
    app.draft.epochs.clear_live();
    if let Some(slot) = &dropped {
        tracing::info!(draft = %slot.id, "the draft ended: a saved effect is being put on");
    }
    ReplaceGuard { _gate: gate, dropped: dropped.is_some(), dropped_slot: dropped }
}

/// The answer `GET`/`PUT`/`DELETE /effects/draft` give. A `note` is only set by an ending that
/// could not put the prior effect back.
pub fn status(app: &App) -> canvas_proto::api::DraftStatus {
    let now = app.draft.now();
    let heartbeat_ms = canvas_proto::consts::DRAFT_HEARTBEAT.as_millis() as u64;
    let lease_ms = canvas_proto::consts::DRAFT_LEASE.as_millis() as u64;
    let watermark = app.draft.watermark();
    match app.draft.snapshot() {
        Some(slot) => {
            let (epoch, seq) = match (&slot.kind, watermark) {
                (SlotKind::Stack { .. }, Some((epoch, seq))) => (Some(epoch), Some(seq)),
                _ => (None, None),
            };
            canvas_proto::api::DraftStatus {
                active: true,
                id: Some(slot.id.clone()),
                source_hash: Some(slot.source_hash().to_string()),
                props: slot.props.clone(),
                lease_remaining_ms: slot.lease_until.saturating_sub(now).as_millis() as u64,
                heartbeat_ms,
                lease_ms,
                note: app.draft.last_note(),
                applied: None,
                epoch,
                seq,
            }
        }
        None => canvas_proto::api::DraftStatus {
            active: false,
            id: None,
            source_hash: None,
            props: Props::new(),
            lease_remaining_ms: 0,
            heartbeat_ms,
            lease_ms,
            note: app.draft.last_note(),
            applied: None,
            epoch: None,
            seq: None,
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
