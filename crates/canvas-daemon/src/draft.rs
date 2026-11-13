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
//! structure joins it, a structure that comes back rejoins the build still making it, and a PUT
//! with a different structure starts its own build beside the one in flight rather than waiting
//! for it. The engine is still written only under the gate, so no two builds hand it an effect
//! at once.
//!
//! The editor renews the slot every [`canvas_proto::consts::DRAFT_HEARTBEAT`]; the slot expires
//! [`canvas_proto::consts::DRAFT_LEASE`] after the last PUT, and the lease starts when the draft
//! reaches the lights, not when its build started. Every accepted PUT renews it, including one
//! that joins a build in flight, and the lease does not run out while a build the owning page
//! started is still loading. Stop and expiry put the effect the draft took over from back on the
//! lights; an activate or a layout switch drops the slot without restoring, because the owner
//! chose something else.
//!
//! The gate orders transitions, not builds: a build runs outside it, so Stop and expiry answer
//! while a slow HTML layer loads. The gate is taken to read the slot, to swap it and to apply.

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
    /// The effect the draft took over: the id a group's `Plays` names, when the draft is that
    /// effect's own source. `None` when the draft names no saved effect, so it plays on one
    /// temporary group of every light.
    pub effect_id: Option<String>,
    /// The groups the draft is on: the scene's own groups that play its effect, or the one
    /// temporary group of every light ([`crate::scenes::TAKEOVER_GROUP`]).
    pub groups: Vec<String>,
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

    /// The source hash the daemon stored for this draft, for an HTML draft.
    fn stored_source_hash(&self) -> Option<&str> {
        match &self.kind {
            SlotKind::Html { source_hash, .. } => Some(source_hash),
            SlotKind::Stack { .. } => None,
        }
    }

    /// True when this slot holds the draft of `html`, whose hash is `source_hash`.
    ///
    /// The hash is a short label the editor shows, and a hash can collide: the source itself
    /// is compared as well, so a different file that happens to hash the same is a different
    /// draft and takes the lights (#94).
    pub fn is_same_html(&self, html: &str, source_hash: &str) -> bool {
        self.stored_source_hash() == Some(source_hash) && self.html_source() == Some(html)
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

/// One build running for the draft route.
///
/// A PUT whose draft equals a running build joins it: the props and the live map move, the lease
/// is left for the landing, and no second build starts. A PUT with a different draft starts a
/// build of its own and makes it current; the older build finishes and lands nowhere.
#[derive(Clone)]
struct InFlight {
    /// Identifies this build; only the token in the book may land.
    token: u64,
    kind: Building,
    /// The newest props a PUT (or a beat that joined) gave the draft.
    props: Props,
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

/// Every build running for the draft route, and which one may land.
///
/// A newer PUT starts its own build beside an older one instead of waiting for it: only the
/// current token lands, and the older result is discarded. A draft that comes back is made
/// current again, so its still-running build lands instead of a third one starting.
#[derive(Default)]
struct Builds {
    /// The builds still running. A build is removed when it lands or lets go.
    running: Vec<InFlight>,
    /// The token of the build whose result may land. A newer PUT, a Stop, an expiry or a
    /// replacement takes it away.
    current: Option<u64>,
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
    /// The builds running for this route and the one allowed to land.
    builds: Mutex<Builds>,
    /// The token the next build takes.
    tokens: AtomicU64,
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
            builds: Mutex::new(Builds::default()),
            tokens: AtomicU64::new(0),
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

    fn lock_builds(&self) -> MutexGuard<'_, Builds> {
        self.builds.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A running build this request matches, copied. A draft already building is joined rather
    /// than built a second time.
    fn find_build(&self, matches: impl Fn(&InFlight) -> bool) -> Option<InFlight> {
        self.lock_builds().running.iter().find(|build| matches(build)).cloned()
    }

    /// Whether `token` is still the build allowed to land.
    fn is_current(&self, token: u64) -> bool {
        self.lock_builds().current == Some(token)
    }

    /// Makes a build the one allowed to land: a request for its draft again re-tokenises it,
    /// so Y -> X -> Y rejoins the build still making Y. A stack build that is joined takes the
    /// joining page's epoch, so the lease of the draft the build will replace is held open for
    /// that page, whoever started the build.
    fn adopt_build(&self, token: u64, epoch: Option<String>) {
        let mut builds = self.lock_builds();
        if let Some(epoch) = epoch {
            if let Some(building) = builds.running.iter_mut().find(|b| b.token == token) {
                building.epoch = Some(epoch);
            }
        }
        builds.current = Some(token);
    }

    /// Starts a build and returns its token. A build already running is superseded: its result
    /// will find another current token and land nowhere.
    fn begin_build(
        &self,
        kind: Building,
        props: &Props,
        epoch: Option<String>,
    ) -> u64 {
        let token = self.tokens.fetch_add(1, Ordering::SeqCst) + 1;
        let mut builds = self.lock_builds();
        builds.running.push(InFlight { token, kind, props: props.clone(), epoch });
        builds.current = Some(token);
        token
    }

    /// Gives a beat's props to the build `token`, if it is still running.
    fn update_in_flight(&self, token: u64, props: &Props) {
        if let Some(building) = self.lock_builds().running.iter_mut().find(|b| b.token == token) {
            building.props = props.clone();
        }
    }

    /// Takes the build `token` out of the book so it can land; `None` when a Stop, an expiry, a
    /// replacement or a newer PUT superseded it. A superseded build is still removed, so a
    /// finished build never lingers for a later request to join.
    fn take_in_flight_if(&self, token: u64) -> Option<InFlight> {
        let mut builds = self.lock_builds();
        let at = builds.running.iter().position(|b| b.token == token)?;
        let building = builds.running.remove(at);
        if builds.current == Some(token) {
            builds.current = None;
            Some(building)
        } else {
            None
        }
    }

    /// Forgets a build that ended without landing (an error, a join error, a panic).
    fn forget_build(&self, token: u64) {
        let mut builds = self.lock_builds();
        builds.running.retain(|b| b.token != token);
        if builds.current == Some(token) {
            builds.current = None;
        }
    }

    /// A PUT whose draft is already on the lights supersedes any build for another one, but
    /// leaves the builds running: one can be rejoined if its draft comes back.
    fn supersede_build(&self) {
        self.lock_builds().current = None;
    }

    /// Keeps the draft on the lights alive. A beat from the page that owns it renews the slot's
    /// lease even when it starts or joins a build, so the slot does not run out from under a
    /// build that is about to land (D95.4.4, item 3).
    fn renew_lease(&self) {
        let lease_until = self.now() + canvas_proto::consts::DRAFT_LEASE;
        if let Some(slot) = self.lock().as_mut() {
            slot.lease_until = lease_until;
        }
    }

    /// True while the current build, the one the owning page's last PUT started or rejoined, is
    /// in flight for the live epoch: the slot's lease does not end one then.
    fn building_for_the_live_epoch(&self) -> bool {
        let Some((live, _)) = self.epochs.live() else {
            return false;
        };
        let builds = self.lock_builds();
        builds.current.is_some_and(|token| {
            builds
                .running
                .iter()
                .any(|b| b.token == token && b.epoch.as_deref() == Some(live.as_str()))
        })
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
        self.draft.forget_build(self.token);
    }
}

/// Why a PUT did not leave a draft on the lights.
#[derive(Debug)]
pub enum PutError {
    /// The draft's HTML did not build: the editor's problem (422), and a running draft is left
    /// alone.
    Build(String),
    /// The stack's name is another stack's, ignoring case: the answer is a 409, as Save's is.
    Conflict(String),
    /// The stack breaks a rule a Save enforces (422).
    Validation(String),
    /// The daemon could not hand the effect over: not the editor's problem (500).
    Internal(String),
    /// The daemon is already rendering its limit of previews: the answer is a 503 with
    /// `Retry-After`, like the preview routes give, and nothing starts.
    Busy,
}

impl std::fmt::Display for PutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PutError::Build(message)
            | PutError::Conflict(message)
            | PutError::Validation(message)
            | PutError::Internal(message) => f.write_str(message),
            PutError::Busy => f.write_str("the daemon is already rendering its limit of previews"),
        }
    }
}

/// What ending a draft did to the lights.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ended {
    /// A sentence for the answer when the scene could not be put back. `None` when the active
    /// scene went on normally.
    pub note: Option<String>,
}

/// Puts `html` on the lights as the draft, renews a running draft's lease and updates its props.
///
/// The draft plays on every group of the active scene whose `Plays` names its effect, or on one
/// temporary flow-across group of every light when no group plays it. Stop and expiry put the
/// active scene back (see [`restore`]).
///
/// A later PUT that replaces the draft's source keeps the same takeover: a draft is one visit to
/// the editor, however many times its code changes. A PUT that is superseded while it built (a
/// Stop, or another source) answers `false` and leaves the newer state alone. A beat whose
/// source is already building joins that build rather than starting another.
pub async fn put(app: &Arc<App>, html: &str, props: &Props) -> Result<bool, PutError> {
    let source_hash = crate::user_effects::source_hash(html);
    let id = format!("{}{source_hash}", canvas_proto::consts::DRAFT_ID_PREFIX);

    let token;
    // The preview slot this build holds until it finishes. A beat that joins or renews takes
    // no slot, so only a PUT that starts a build can be refused for lack of one.
    let slot;
    {
        let _gate = app.draft.gate.lock().await;
        expire_locked(app).await;
        let existing = app.draft.snapshot();

        // The same source and the same kind: only the lease, and maybe the props, moved. The
        // engine's instance is re-initialised rather than rebuilt, so a beat does not reload
        // the file's scripts.
        if let Some(slot) = &existing {
            if slot.is_same_html(html, &source_hash) {
                let props_changed = slot.props != *props;
                let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;
                let groups = slot.groups.clone();
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
                    for group in &groups {
                        app.engine.set_group_props(group, props);
                    }
                }
                return Ok(true);
            }
        }

        // The same source is already building: this beat joins it, and the build that started
        // lands with the newest props. A build slower than the beat still applies. The build is
        // made current again, so a source that comes back (Y -> X -> Y) lands rather than
        // queuing a third build.
        if let Some(building) = app.draft.find_build(|b| b.is_html(html)) {
            app.draft.update_in_flight(building.token, props);
            app.draft.adopt_build(building.token, None);
            app.draft.renew_lease();
            return Ok(true);
        }

        // This source failed to load the last time it was built and nothing has landed since:
        // answer that error instead of loading a file that cannot load on every beat.
        if let Some(message) = app.draft.last_failure(html) {
            return Err(PutError::Build(message));
        }

        let Ok(acquired) = app.draft_slots.clone().try_acquire_owned() else {
            return Err(PutError::Busy);
        };
        slot = acquired;
        token = app.draft.begin_build(Building::Html { source: Arc::from(html) }, props, None);
    }
    // Every exit path from here frees the route: an error, a join error, or a panic in the
    // blocking task must not leave a token for later beats to join. The lease stays in this
    // future rather than moving into the blocking task: the landing still needs the token in
    // the book, and this lease is what takes it back on an error, a join error, a panic, or a
    // client that goes away mid-build. The slot, on the other hand, is moved into the build,
    // so it is held for exactly as long as the build runs.
    let _lease = BuildLease::new(&app.draft, token);
    // The newest request does not wait for a build it has already superseded: its build runs
    // beside any older one, and the older result is discarded when it lands. The engine is
    // written only under the gate, so no two builds hand it an effect at once.
    if !app.draft.is_current(token) {
        return Ok(false);
    }

    // Building it runs the file's scripts, so it goes on a blocking thread, and the gate is not
    // held while it runs: Stop and expiry answer at once.
    let build_id = id.clone();
    let build_source = html.to_string();
    let build = app.draft.builder();
    let effect = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        build(&build_id, &build_source)
    })
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
    // The draft takes over the groups of the active scene that play its effect, or one
    // temporary group of every light when no group does.
    let saved_id = saved_effect_id(app, &source_hash);
    let effect_id = saved_id.clone().unwrap_or_else(|| id.clone());
    // The group the draft lands on, so the instance is built for it: a stack resolves its
    // areas in the group's canvas.
    let target = crate::scenes::takeover_groups(app, &effect_id).await;
    let group = target.first().cloned().unwrap_or_default();
    // The scene must not run more HTML groups than it may: the draft's effect is one of them.
    crate::scenes::check_takeover_html_limit(app, &effect_id, &target, None)
        .await
        .map_err(PutError::Validation)?;
    let builder = takeover_builder(app, &id, html);
    let groups = crate::scenes::apply_takeover(
        app,
        &effect_id,
        &building.props,
        Some((group, effect)),
        builder,
    )
    .await
    .map_err(PutError::Internal)?;
    app.draft.clear_failure();
    // The lease starts when the draft reaches the lights, not when its build started: a slow
    // build must not eat the editor's beat.
    let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;

    let slot = Slot {
        id,
        props: building.props,
        effect_id: saved_id,
        groups,
        lease_until,
        kind: SlotKind::Html { source: Arc::from(html), source_hash },
    };
    *app.draft.lock() = Some(slot.clone());
    app.draft.clear_ending();
    // A draft started after an onboarding greeting outranks its restore: the owner is choosing
    // a look, and ending the draft puts the active scene back anyway.
    app.transient.cancel();
    tracing::info!(draft = %slot.id, groups = slot.groups.len(), "a draft is on the lights");
    Ok(true)
}

/// The id of the saved effect the draft's source is, when the draft is that effect's own
/// source: a draft of a saved effect plays on the groups that play it. `None` when the draft is
/// a new file, or one whose text has changed since it was saved.
fn saved_effect_id(app: &App, source_hash: &str) -> Option<String> {
    app.user_effects
        .schemas()
        .iter()
        .filter(|schema| schema.id.starts_with(crate::user_effects::USER_EFFECT_PREFIX))
        .find(|schema| {
            app.user_effects
                .source_and_file_name(&schema.id)
                .is_some_and(|(source, _)| crate::user_effects::source_hash(&source) == source_hash)
        })
        .map(|schema| schema.id.clone())
}

/// The builder a takeover makes its extra instances with: the same file, built again.
fn takeover_builder(app: &Arc<App>, id: &str, html: &str) -> crate::scenes::EffectBuilder {
    let build = app.draft.builder();
    let id = id.to_string();
    let html = html.to_string();
    Arc::new(move |_group| build(&id, &html))
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
    // The one check a Save also runs, on the canonical stack (D95.4.4, D95.4.5): a layer written
    // by its display name is the id it becomes, so the HTML cap counts it, the name rule leaves
    // out this stack by exact id as `Stacks::put` does, and the stack that is built and hashed is
    // the one a Save would store.
    let mut canonical = stack.clone();
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    crate::stacks::canonicalise_and_validate(&mut canonical, &app.stacks, &known).map_err(
        |refusal| match refusal {
            crate::stacks::StackRefusal::Conflict(message) => PutError::Conflict(message),
            crate::stacks::StackRefusal::Invalid(message) => PutError::Validation(message),
        },
    )?;
    let stack = &canonical;

    let structure = crate::preview::structure_of(app, stack);
    let mut props = crate::preview::props_of(app, stack);
    props.extend(overrides.clone());

    let token;
    // The preview slot this build holds until it finishes. A beat that joins or renews takes
    // no slot, so only a PUT that starts a build can be refused for lack of one.
    let slot;
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
                        let groups = slot.groups.clone();
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
                        for group in &groups {
                            app.engine.set_group_props(group, &props);
                        }
                        app.stacks.note_running(
                            &stack.id,
                            &structure,
                            Arc::clone(live),
                            groups.first().map(String::as_str),
                        );
                        return Ok(true);
                    }
                }
            }
        }

        // The same structure is already building: this beat joins it. The live map is the one
        // the building effect reads, so the values are there when it lands; its props are the
        // newest, and no second build starts. The build is made current again, so a structure
        // that comes back (Y -> X -> Y) lands rather than queuing a third build. The beat also
        // renews the lease of the draft on the lights, which is still the one the build will
        // replace.
        if let Some(building) = app.draft.find_build(|b| b.is_stack(&stack.id, &structure)) {
            if let Building::Stack { live, .. } = &building.kind {
                crate::stack::replace_live(live, stack);
            }
            app.draft.update_in_flight(building.token, &props);
            app.draft.adopt_build(building.token, Some(epoch.to_string()));
            app.draft.renew_lease();
            tracing::info!(stack = %stack.id, seq, "a beat joined the stack draft build in flight");
            return Ok(true);
        }

        let Ok(acquired) = app.draft_slots.clone().try_acquire_owned() else {
            return Err(PutError::Busy);
        };
        slot = acquired;
        token = app.draft.begin_build(
            Building::Stack {
                stack_id: stack.id.clone(),
                structure: structure.clone(),
                live: Arc::new(RwLock::new(LiveMap::from_stack(stack, 1))),
            },
            &props,
            Some(epoch.to_string()),
        );
    }
    // Every exit path from here frees the route: an error, a join error, or a panic in the
    // blocking task must not leave a token for later beats to join. The lease stays in this
    // future rather than moving into the blocking task: the landing still needs the token in
    // the book, and this lease is what takes it back on an error, a join error, a panic, or a
    // client that goes away mid-build. The slot, on the other hand, is moved into the build,
    // so it is held for exactly as long as the build runs.
    let _lease = BuildLease::new(&app.draft, token);
    // The newest request does not wait for a build it has already superseded: its build runs
    // beside any older one, and the older result is discarded when it lands. The engine is
    // written only under the gate, so no two builds hand it an effect at once.
    if !app.draft.is_current(token) {
        return Ok(false);
    }
    let Some(building) = app.draft.find_build(|b| b.token == token) else {
        return Ok(false);
    };
    let live = match &building.kind {
        Building::Stack { live, .. } => Arc::clone(live),
        Building::Html { .. } => unreachable!("this PUT started a stack build"),
    };

    // The group the draft lands on, so the stack is built for it: its layers resolve in that
    // group's canvas.
    let target = crate::scenes::takeover_groups(app, &stack.id).await;
    let group = target.first().cloned().unwrap_or_default();
    // The scene must not run more HTML groups than it may: a stack with an HTML layer is one.
    // The draft's own layers count, not the saved stack's: the draft is what runs.
    crate::scenes::check_takeover_html_limit(app, &stack.id, &target, Some(stack))
        .await
        .map_err(PutError::Validation)?;

    // A new structure. Its HTML layers load here, outside the gate.
    let build_app = Arc::clone(app);
    let build_stack = stack.clone();
    let build_group = group.clone();
    let effect = tokio::task::spawn_blocking(move || -> Result<Box<dyn Effect>, String> {
        let _slot = slot;
        let mut effect = crate::stack::StackEffect::new(
            &build_stack,
            crate::leaf_builder(&build_app.user_effects),
            live,
            build_app.stacks.areas(),
        )
        .with_clock(build_app.stacks.clock())
        .with_fps(build_app.stacks.fps_handle())
        .with_status_sink(build_app.stacks.status_sink());
        if !build_group.is_empty() {
            effect = effect.with_group(&build_group);
        }
        Ok(Box::new(effect))
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
    // The draft takes over the groups of the active scene that play this stack, or one
    // temporary group of every light when no group does.
    let builder = stack_takeover_builder(app, stack, Arc::clone(&live));
    let groups = crate::scenes::apply_takeover(
        app,
        &stack.id,
        &building.props,
        Some((group, effect)),
        builder,
    )
    .await
    .map_err(PutError::Internal)?;
    app.draft.clear_failure();
    // The stack is what the main engine runs: a save-then-activate with the same structure
    // reuses it instead of building again (D95.4.3, D95.4.6). It resolves in the group the
    // draft landed on.
    app.stacks.note_running(
        &stack.id,
        &structure,
        Arc::clone(&live),
        groups.first().map(String::as_str),
    );
    // The lease starts when the draft reaches the lights, not when its build started.
    let lease_until = app.draft.now() + canvas_proto::consts::DRAFT_LEASE;

    let slot = Slot {
        id: stack.id.clone(),
        props: building.props,
        effect_id: Some(stack.id.clone()),
        groups,
        lease_until,
        kind: SlotKind::Stack { stack_id: stack.id.clone(), structure, live },
    };
    *app.draft.lock() = Some(slot.clone());
    app.draft.clear_ending();
    app.transient.cancel();
    tracing::info!(draft = %slot.id, seq, groups = slot.groups.len(), "a stack draft is on the lights");
    Ok(true)
}

/// The builder a stack takeover makes its extra instances with: the same stack, with the same
/// live map, built again.
fn stack_takeover_builder(
    app: &Arc<App>,
    stack: &Stack,
    live: LiveStack,
) -> crate::scenes::EffectBuilder {
    let app = Arc::clone(app);
    let stack = stack.clone();
    Arc::new(move |group| {
        let mut effect = crate::stack::StackEffect::new(
            &stack,
            crate::leaf_builder(&app.user_effects),
            Arc::clone(&live),
            app.stacks.areas(),
        )
        .with_clock(app.stacks.clock())
        .with_fps(app.stacks.fps_handle())
        .with_status_sink(app.stacks.status_sink());
        if !group.is_empty() {
            effect = effect.with_group(group);
        }
        Ok(Box::new(effect) as Box<dyn Effect>)
    })
}

/// What `DELETE /effects/draft` did.
pub struct StopOutcome {
    /// False when the page that asked does not own the draft: nothing ended, and the draft of
    /// the page that does own it is left alone (D95.4.4, item 4).
    pub applied: bool,
    /// What ending the draft did to the lights, when one ended here.
    pub ended: Option<Ended>,
}

/// Stops the draft and puts the prior effect back (the editor's Stop).
///
/// `epoch` is the page's session epoch when the caller sends one: a page that does not own the
/// draft ends nothing and is answered `applied: false`, so a stale tab's Stop cannot take the
/// lights off the page that owns them.
///
/// The slot always goes: a Stop, an expiry or a GET after the lease is over never leaves a
/// draft behind. When the prior effect cannot be built (its file was deleted mid-draft, say),
/// black goes on the lights and [`Ended::note`] says so; the answer is still a success.
pub async fn stop(app: &Arc<App>, epoch: Option<&str>) -> StopOutcome {
    let _gate = app.draft.gate.lock().await;
    stop_locked(app, epoch).await
}

/// [`stop`] with the gate already held.
///
/// The caller holds the gate across the whole replace: the end of the draft and whatever
/// takes the lights after it. A PUT that starts in between would read the prior from the
/// store that is about to be replaced, and its later restore would put the old effect back
/// over the new one (#94).
pub(crate) async fn stop_locked(app: &Arc<App>, epoch: Option<&str>) -> StopOutcome {
    if let Some(epoch) = epoch {
        if !app.draft.epochs.holds(epoch) {
            return StopOutcome { applied: false, ended: None };
        }
    }
    // An explicit Stop cancels a build that is still running, even when the first draft has
    // not landed yet: a builder that stopped does not want it on the lights afterwards.
    app.draft.supersede_build();
    // The live sequence goes, the epoch order stays: the tab that stopped can start again with
    // its next PUT, and an older tab still cannot take the lights (D95.4.4, item 4).
    app.draft.epochs.clear_live();
    let Some(slot) = app.draft.lock().take() else {
        return StopOutcome { applied: true, ended: None };
    };
    let ended = restore(app, &slot).await;
    app.draft.remember_ending(&ended);
    tracing::info!(
        draft = %slot.id,
        note = ended.note.as_deref().unwrap_or(""),
        "the draft stopped"
    );
    StopOutcome { applied: true, ended: Some(ended) }
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
    // A build the page that owns the draft has in flight keeps the slot: the lease is renewed
    // when that build lands, and expiring the slot now would put the prior effect back under a
    // draft that is about to reach the lights (D95.4.4, item 3).
    let building_for_it = app.draft.building_for_the_live_epoch();
    let due = {
        let mut guard = app.draft.lock();
        match guard.as_ref() {
            Some(slot) if now >= slot.lease_until && !building_for_it => guard.take(),
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
    /// The draft gate, when this guard took it. `None` when the caller holds it.
    _gate: Option<tokio::sync::MutexGuard<'a, ()>>,
    /// True when a draft was on the lights and was dropped here.
    pub dropped: bool,
    /// True when a takeover held the lights when this guard was taken: the engine's groups are
    /// the takeover's, so the caller's work must be a full scene apply from the store, never a
    /// diff and never a single effect.
    ///
    /// The takeover flag stays set until that full apply clears it: ending a takeover always
    /// ends it on the engine.
    ended_takeover: bool,
    /// True when the takeover was ended on the engine. The full apply is the only thing that
    /// clears the flag, so the guard remembers that it was paid.
    settled: std::sync::atomic::AtomicBool,
    /// The draft that was dropped here. If the caller's own apply then fails, the engine is
    /// left showing a draft nothing owns, so the guard can put this slot's prior effect back
    /// (or black, with a note) instead (D95.4.5, test 12).
    dropped_slot: Option<Slot>,
}

impl ReplaceGuard<'_> {
    /// True when a takeover held the lights and it has not been ended on the engine yet.
    ///
    /// The engine's groups are then the takeover's, and the caller's work has to be a full
    /// scene apply from the store: a diff would leave the takeover's effect on a group the
    /// scene names, and a single effect would take the captures the other groups need.
    pub fn ended_takeover(&self) -> bool {
        self.ended_takeover && !self.settled.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Puts the whole scene on again: the takeover ends on the engine.
    ///
    /// The takeover flag is cleared by the full apply and by nothing else, so this is the only
    /// way to end one. A route that fails before it comes here leaves the guard to do it (see
    /// [`ReplaceGuard::fail`]), so the lights are never left on a takeover nothing owns.
    pub async fn end_takeover(&self, app: &Arc<App>, fade: bool) {
        if self.settled.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.settled.store(true, std::sync::atomic::Ordering::SeqCst);
        if self.ended_takeover {
            tracing::info!("the takeover ended: the scene went back on the lights");
        }
        crate::scenes::apply_active_ungated(app, fade).await;
    }

    /// The caller did a full scene apply itself: the takeover is ended on the engine.
    pub fn settle(&self) {
        self.settled.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The route failed before it could end the takeover on the engine: it happens here,
    /// before the error goes out.
    ///
    /// The scene goes back on the lights even though the route failed: the takeover is over,
    /// and nothing owns it any more.
    pub async fn fail(&self, app: &Arc<App>) {
        if !self.ended_takeover || self.settled.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.end_takeover(app, false).await;
    }

    /// Puts the dropped draft's prior back on the lights, exactly as Stop would, and
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
            "the draft ended and the active scene went back after the replacement failed"
        );
        Some(ended)
    }
}

/// Ends any draft (without restoring it) and holds the gate until the guard is dropped.
///
/// The takeover flag is left alone: ending a takeover always ends it on the engine, so the
/// caller that ends one does a full scene apply, and that apply clears the flag. A caller that
/// only diffs the scene must not leave the takeover's effect on the lights with the flag
/// cleared.
///
/// A takeover is anything that holds the lights: the editor's draft, the onboarding greeting,
/// or a stack draft. The guard does not care which: it records that the lights were held, so
/// the caller puts the whole scene on again.
pub async fn hold_for_replace(app: &Arc<App>) -> ReplaceGuard<'_> {
    let gate = app.draft.gate.lock().await;
    let guard = take_guard(app, true);
    ReplaceGuard { _gate: Some(gate), ..guard }
}

/// Looks at the lights and owes the full apply that ends a takeover, with the draft gate
/// already held.
///
/// The props route uses this one: it refuses to change anything while the editor's draft holds
/// the lights, so it must not end that draft, and a build that is still running must not be
/// superseded either. What it does need is to know whether a takeover (the onboarding
/// greeting) holds the lights, and to put the whole scene on again when the route ends it.
pub fn hold_for_takeover(app: &Arc<App>) -> ReplaceGuard<'_> {
    take_guard(app, false)
}

/// The guard's own state: what holds the lights, and what the route owes.
fn take_guard(app: &Arc<App>, drop_the_draft: bool) -> ReplaceGuard<'_> {
    let dropped = if drop_the_draft {
        let dropped = app.draft.lock().take();
        // The replacement outranks any build still running, landed or not.
        app.draft.supersede_build();
        // The live sequence goes, the epoch order stays, so an older tab cannot re-take the
        // slot after a layout switch or an activation.
        app.draft.epochs.clear_live();
        if let Some(slot) = &dropped {
            tracing::info!(draft = %slot.id, "the draft ended: a saved effect is being put on");
        }
        dropped
    } else {
        None
    };
    // Something held the lights: the engine's groups are the takeover's, and the caller has to
    // put the whole scene on again.
    let ended_takeover = app.scene.is_taken_over();
    ReplaceGuard {
        _gate: None,
        dropped: dropped.is_some(),
        ended_takeover,
        settled: std::sync::atomic::AtomicBool::new(false),
        dropped_slot: dropped,
    }
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

/// Puts the active scene back on the lights: the draft's prior is the scene, not one effect.
///
/// The scene order is not taken here (see [`crate::scenes::apply_active_ungated`]): the draft
/// gate is held, and a scene write holds the scene order while it waits for that gate.
async fn restore(app: &Arc<App>, _slot: &Slot) -> Ended {
    crate::scenes::apply_active_ungated(app, false).await;
    // The scene is back. A group whose effect cannot be built is dark, and the answer says so:
    // the owner sees why the lights are not what they set.
    let failures = app.scene.failures();
    if failures.is_empty() {
        return Ended::default();
    }
    let groups: Vec<String> = failures
        .iter()
        .map(|(group, why)| format!("{group}: {why}"))
        .collect();
    Ended {
        note: Some(format!(
            "the active scene is back, but a group of it is dark: {}",
            groups.join("; ")
        )),
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
            sweep_once(&app).await;
        }
    });
}

/// One sweep: the draft whose lease ran out ends.
///
/// The sweep runs in a task of its own, so a panic in the expiry is a failed join and not
/// the end of the sweeper: the panic is logged and the next sweep runs. Without this, one
/// panic in [`expire_if_due`] would stop the draft expiry for the rest of the daemon's life
/// (#94).
async fn sweep_once(app: &Arc<App>) {
    let sweep_app = Arc::clone(app);
    match tokio::spawn(async move { expire_if_due(&sweep_app).await }).await {
        Ok(_) => {}
        Err(error) if error.is_panic() => {
            let payload = error.into_panic();
            tracing::error!(
                panic = %crate::panic_payload_text(&payload),
                "the draft sweeper panicked; the sweep continues"
            );
        }
        Err(error) => {
            tracing::error!(
                error = %error,
                "the draft sweeper's sweep did not finish; the sweep continues"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot holding `source`, with the hash the daemon stores beside it.
    fn html_slot(source: &str, source_hash: &str) -> Slot {
        Slot {
            id: format!("{}{source_hash}", canvas_proto::consts::DRAFT_ID_PREFIX),
            props: Props::new(),
            effect_id: None,
            groups: Vec::new(),
            lease_until: Duration::ZERO,
            kind: SlotKind::Html {
                source: Arc::from(source),
                source_hash: source_hash.to_string(),
            },
        }
    }

    #[test]
    fn a_slot_with_the_same_hash_but_another_source_is_not_the_same_draft() {
        // The hash is a label and two files can hash the same, so the source decides: a
        // file that collides with the one on the lights is a new draft (#94).
        let slot = html_slot("the first file", "0123456789abcdef");
        assert!(
            slot.is_same_html("the first file", "0123456789abcdef"),
            "the same source with the same hash is the same draft"
        );
        assert!(
            !slot.is_same_html("another file", "0123456789abcdef"),
            "the same hash with another source is another draft"
        );
        assert!(
            !slot.is_same_html("the first file", "fedcba9876543210"),
            "another hash is another draft"
        );
    }

    #[test]
    fn a_stack_draft_is_never_the_same_html_draft() {
        let slot = Slot {
            id: "stack:one".to_string(),
            props: Props::new(),
            effect_id: None,
            groups: Vec::new(),
            lease_until: Duration::ZERO,
            kind: SlotKind::Stack {
                stack_id: "one".to_string(),
                structure: "0123456789abcdef".to_string(),
                live: Arc::new(RwLock::new(LiveMap {
                    version: 0,
                    layers: std::collections::BTreeMap::new(),
                })),
            },
        };
        assert!(
            !slot.is_same_html("any source", "0123456789abcdef"),
            "a stack draft is not an HTML draft"
        );
    }
}
