# Chain setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-LED walk with a three-action chain setup (drag, type a number, confirm) that tells the app what is plugged into each resizable ARGB header and writes the parts to the hardware map of lights.

**Architecture:** The daemon gains one setup session beside the identify override: a frame composer that lights several coloured runs of one zone, its own timeout, and a restore record that survives a crash. A header is raised to its longest length once, held against the OpenRGB supervisor's saved-size re-apply, and resized to its final length once at commit. Commit is one operation: resize, then write `zones.json`, `lights.json` and `scenes.json` after a snapshot that lets start-up undo a half-done write. The editor gets a takeover with three steps and Devices rows per header, and loses the walker.

**Tech Stack:** Rust (canvas-proto, canvas-adapters fake OpenRGB server, canvas-daemon), axum, serde, ts-rs; Next.js static export, React, vitest, CSS modules with the tokens in `app/web/src/styles/`.

**Spec:** `docs/superpowers/specs/2026-11-12-chain-setup-design.md`. Read it first. This plan argues from it and changes none of its decisions. It builds on `docs/superpowers/specs/2026-11-12-scenes-design.md` and `docs/superpowers/plans/2026-11-12-scenes.md`. Issue: #338. Release: 0.15.0.

**Baseline read for this plan:** `feat/scenes` at `df4cc41a`, which holds Scenes tasks 0 to 13 (checked with `git log`, `ls`, and the greps cited below). Every path and line in this plan was read on that commit. Where Scenes changed a name since the spec was written, the plan uses the current name.

## Global Constraints

- Repo rules are in `CLAUDE.md` in the main checkout (gitignored, so absent from worktrees). The ones that bind every task are restated here.
- Every cargo or npm build runs through the lock: `powershell -NoProfile -File tools/with-build-lock.ps1 <command>`. Never `-j`, `--jobs`, `CARGO_BUILD_JOBS` or `RUSTC_WRAPPER`. Never start a dev server.
- `crates/canvas-proto` is the spine. Only the main agent edits it (Task 1). A builder that needs a spine change reports it back. After any spine change run `cargo test -p canvas-proto` to regenerate `app/web/src/proto/`.
- Tests run on the fakes: the fake OpenRGB server (`crates/canvas-adapters/src/fake.rs`, `FakeOpenRgbServer`) and the daemon's `FakeAdapter` (`crates/canvas-daemon/tests/common/mod.rs:249`). Real headers are touched only in the batched hardware session at the end, with Luke's OK in chat. No PawnIO, no SignalRGB change outside that session. Do not run tests that need port 9430 while Luke's daemon holds it; CI settles those.
- At most 2 builders at once, on disjoint files. One heavy build at a time per lock slot.
- All work lands on the integration branch `feat/chain-setup`, cut from `feat/scenes`, through task PRs that target it. Issue: #338. Every PR body says `Part of #338`.
- Spec constants, verbatim: a zone is resized at most twice per setup (once up, once to the final length), never while the slider moves; a row has at least 1 LED; the first-run default header length stays 120 (`crates/canvas-daemon/src/first_run.rs:15`).
- Words: chain, header, part, light, group, scene. The word "layout" appears nowhere in new UI text, CLI output or API names. The owner is never asked about position or facing.
- UI: `.build/ui-taste.md` in the main checkout, and `app/web/src/styles/hardware.css`. Phosphor icons only. Every colour a token with light and dark values. Radii 6 to 10px. Component layer, no raw HTML controls. Nothing moves under the pointer. Every state change has a transition.
- UI proof: screenshots at 1920x1040, 1440x900, 1280x860, 1024x768, 960x1032 and 390x844, light and dark, taken with `tools/ui-capture` against `tools/run-fake-daemon.ps1`, and looked at. Headless only: no window on Luke's screen.
- A user-visible change adds a line under `## [Unreleased]` in `CHANGELOG.md` in the same commit. Never edit a version by hand. Never push a `v*` tag without Luke's OK; Luke cuts 0.15.0.
- A bug found on the way follows `docs/bug-classes.md`: name the issue, regression test on the fakes, search the siblings.
- Nothing public (commits, PRs, issues, committed files, this plan) carries an absolute local path or a `claude.ai` link. Commit by path, never `git add -A`. No Co-Authored-By lines (project rule for this repo).
- `zones.json` is never given a raised length (spec "Restoring the length survives a crash", rule 2).

## Review Focus

Inputs the spec implies and that are most likely to bite. Each has a test in the task named.

1. **The OpenRGB supervisor undoes the raise while the owner drags.** Every 5 s it re-applies a saved size when the device reports a different one (`crates/canvas-daemon/src/openrgb_supervisor.rs:314-331`, called from `:575-582`). A header that first run sized to 120 has a `zones.json` entry (`first_run.rs:361-395`), so the raised length is "wrong" to the supervisor and is resized back mid-drag. (Task 0 proves it, Task 3 `refresh_does_not_undo_a_held_raise`)
2. **The window closes, the app crashes or the daemon restarts mid-setup.** The header must return to its earlier length and the lights to the scene. A crash with OpenRGB still running leaves the raised length in OpenRGB, and a header with no `zones.json` entry gets no re-apply. (Task 5 `session_expiry_ends_the_hold_and_asks_for_the_restore`, Task 7a `start_up_replays_a_leftover_restore_record`)
3. **Commit half-lands.** The resize succeeds and a file write fails, or the first file writes and the second fails. The owner must find the header and all three files as they were. (Task 7b `a_failed_store_write_resizes_back_and_leaves_the_files`, `start_up_rolls_back_a_leftover_intent`)
4. **Set up again with a different part count.** Parts keep identity by position; a part that vanishes leaves its groups; a new part goes to Unassigned; the first-run Everything group keeps lighting every remaining part. (Task 6 `shrinking_a_chain_removes_only_the_missing_parts_from_groups`, `growing_a_chain_adds_unassigned_parts`)
5. **Numbers the owner can type.** A part count above the chain length, 0 parts, a row set to 0 LEDs, a slider that reaches the device maximum with the chain still going. (Task 6 `a_count_above_the_length_is_refused`, Task 9 `the_slider_stops_at_the_maximum_and_says_why`)

## File Structure

| File | Responsibility |
|---|---|
| `crates/canvas-proto/src/scene.rs`, `api.rs` | Spine: chain records, part kind and ids, setup routes and payloads, constants (Task 1) |
| `crates/canvas-adapters/src/fake.rs` | Fake server hooks: resize count, resize failure, resize delay (Task 2) |
| `crates/canvas-daemon/src/openrgb_supervisor.rs`, `state.rs`, `device_sync.rs` | Zone holds: the supervisor stops undoing a raise (Task 3) |
| `crates/canvas-daemon/src/chain_records.rs` (new) | Restore record, intent record: write, read, replay plan (Task 4) |
| `crates/canvas-daemon/src/identify.rs` | The setup frame composer and its timeout, beside the identify target (Task 5) |
| `crates/canvas-daemon/src/chain_plan.rs` (new) | Pure: even split, remainder, identity by position, scene rewrite (Task 6) |
| `crates/canvas-daemon/src/api/chains.rs` (new) | Handlers: list, start, light, cancel, empty, commit; start-up replay (Tasks 7a, 7b) |
| `app/web/src/views/chain/` (new) | `ChainSetup.tsx`, `LengthStep.tsx`, `CountStep.tsx`, `ConfirmStep.tsx`, `PartRow.tsx`, `LengthSlider.tsx`, `ChainTakeover.tsx`, `ChainRow.tsx`, `useChainSetup.ts`, `split.ts`, `partColours.ts`, `chain.module.css` |
| `app/web/src/lib/endpoints.ts`, `store.ts`, `demo.ts`, `demoScenarios.ts`, `fixtures.ts` | Data layer (Task 8) |
| `app/web/src/views/DevicesView.tsx`, `app/web/src/shell/GlobalBanners.tsx`, `app/web/src/views/editor/flow.ts` | Wiring and removals (Tasks 10, 11) |

What this plan builds on, all present on the baseline: `Light`, `LightsFile`, `zone_light_id`, `validate_scene`, `unassigned` (`crates/canvas-proto/src/scene.rs:57,91,112,228,265`); `sync_lights` and `repair` (`crates/canvas-daemon/src/scenes.rs:108,149`); the scene order lock and applies (`scenes.rs:381` `SceneRuntime::lock`, `:588` `apply_active`, `:640` `apply_for_devices`, `:666` `apply_with`); the identify override with several runs (`crates/canvas-daemon/src/identify.rs:38` `Run`, `:172` `start_runs`) used by the flash handler (`crates/canvas-daemon/src/api/scenes.rs:467` `flash_lights`); the lights routes (`api/lights.rs:79` `patch_light`); and on the web `api.lights` (`app/web/src/lib/endpoints.ts:198`), `snap.lights` and `refreshLights` (`app/web/src/lib/store.ts:86,356`), and `useNewLights` (`app/web/src/views/scenes/newLights.ts`).

---

### Task 0: Settle the riskiest unknown, cut the branch, baseline

The riskiest unknown that the fakes can settle is Review Focus 1: **does the supervisor's saved-size re-apply undo a raised header?** Code reading says yes (`openrgb_supervisor.rs:327` resizes when `current != saved`; `:575-582` runs it every 10th 500 ms tick). It is UNKNOWN until a test runs it. Whether a real MSI header drives 300 LEDs fast enough for a live slider is the larger unknown, and only the hardware session settles it (see the end of this plan).

**Files:**
- Create: `crates/canvas-daemon/tests/chain_clash.rs`
- Modify: none

- [ ] **Step 1:** From `feat/scenes` create and push `feat/chain-setup`. Open a draft PR to `feat/scenes` titled "Chain setup (0.15.0)" that names #338.
- [ ] **Step 2:** Baseline, each through the lock: `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `npm --prefix app/web run build`, `npm --prefix app/web test`. Record pass and fail counts in the PR body. A red baseline is fixed or reported first. Record `wc -l app/web/src/views/editor/IdentifyWalker.tsx app/web/src/views/DevicesView.tsx app/web/src/shell/GlobalBanners.tsx` for the removal proof.
- [ ] **Step 3: Write the clash test**, `#[tokio::test] async fn the_supervisor_resizes_a_raised_header_back_to_its_saved_size()`. Start `FakeOpenRgbServer` with the JARGB board from `crates/canvas-adapters/tests/adapters_tests.rs` (`test_pc_devices`, max 300). Build the supervisor the way `crates/canvas-daemon/tests/rescan_supervisor.rs` does, with a real `OpenRgbAdapter` and a `zone_sizes` map holding `{"openrgb:mb": {0: 120}}`. Wait for the connect to apply 120. Call `adapter.set_zone_size(mb, 0, 300)` (the raise). Wait 6 s of real time.
- [ ] **Step 4:** Run `powershell -NoProfile -File tools/with-build-lock.ps1 cargo test -p canvas-daemon --test chain_clash -- --nocapture`. Assert `server.zone_leds(0, 0)` and print it. Expected on the current code: `Some(120)` (the clash is TRUE). If it prints `Some(300)` the clash is FALSE; write that in the PR, delete Task 3, and keep the test as a guard.
- [ ] **Step 5:** Mark the test `#[ignore = "proves the clash; flipped by Task 3"]`, commit only the test. Message: `Tests: the supervisor undoes a raised header (#338)`. Report TRUE or FALSE to the main agent in the PR body.

### Task 1: The spine (main agent only)

**Files:**
- Modify: `crates/canvas-proto/src/scene.rs`, `crates/canvas-proto/src/api.rs`
- Test: unit tests in `scene.rs`; regenerated `app/web/src/proto/`

**Name collision, settled by reading.** The spec's "what it is" kind wants the name `PartKind`. That name is taken: `crates/canvas-proto/src/layout.rs:137` defines `PartKind` (Fan, Radiator, Pump, Ram, Gpu, Cpu, Strip, Keyboard, …, Generic) for the old layout, `layout.rs` stays for the migration, and `app/web/src/proto/PartKind.ts` is generated from it. A second `PartKind` would overwrite that file. The new type is `ChainPartKind`. It stores the owner's kind with its own `Other` value.

**Interfaces (produced; every later task uses these exact names):**

```rust
// scene.rs
pub const CHAIN_RESTORE_FILE: &str = "chain-restore.json"; // beside lights.json
pub const CHAIN_INTENT_FILE: &str = "chain-intent.json";
pub const PART_NAME_MAX: usize = GROUP_NAME_MAX;            // 40
pub const SETUP_TIMEOUT_MS: u64 = 6_000;                    // session ends this long after the last light request

/// `"{zone_light_id}p{n}"`, n counted from 1 by position along the chain.
pub fn part_light_id(device: &DeviceId, zone: u32, n: u32) -> LightId;

#[derive(..., TS)] #[serde(rename_all = "snake_case")]
pub enum ChainState { Empty, SetUp }                        // absent from the file = not set up
pub struct ChainRecord { pub device_id: DeviceId, pub zone_index: u32, pub state: ChainState }
// LightsFile gains: #[serde(default)] pub chains: Vec<ChainRecord>   (schema stays LIGHTS_SCHEMA 1)

#[derive(..., TS)] #[serde(rename_all = "snake_case")]
pub enum ChainPartKind { Fan, Strip, Other }
impl ChainPartKind { pub fn arrangement(self) -> Arrangement; }   // Fan -> Ring; Strip and Other -> Strip
// Light gains: #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] pub kind: Option<ChainPartKind>
//   None for a whole-zone light and for a light from before 0.15.0.
```

```rust
// api.rs: LightPatch gains `kind: Option<ChainPartKind>`; patching `kind` sets `arrangement` from it.
// Patching `arrangement` alone leaves `kind` as it was (the Devices Strip/Ring key keeps working for lights with no kind).

// api.rs routes ({device} is the DeviceId, {zone} the zone index, as in DEVICE_ZONE)
pub const CHAINS: &str = "/api/v1/chains";                                  // GET
pub const CHAIN_SETUP: &str = "/api/v1/chains/{device}/{zone}/setup";       // POST start, DELETE cancel
pub const CHAIN_SETUP_LIGHT: &str = "/api/v1/chains/{device}/{zone}/setup/light";   // POST
pub const CHAIN_SETUP_COMMIT: &str = "/api/v1/chains/{device}/{zone}/setup/commit"; // POST
pub const CHAIN_SETUP_EMPTY: &str = "/api/v1/chains/{device}/{zone}/setup/empty";   // POST

// api.rs payloads
pub enum ChainSetupState { NotSetUp, Empty, SetUp }                         // snake_case
pub struct ChainView { pub device_id: DeviceId, pub device_name: String, pub zone_index: u32,
                       pub zone_name: String, pub state: ChainSetupState, pub leds: u32,
                       pub min_leds: u32, pub max_leds: u32, pub parts: Vec<LightView> }
pub struct ChainsResponse { pub chains: Vec<ChainView> }                    // resizable zones only
pub struct SetupStarted { pub session: String, pub restore_to: u32, pub raised_to: u32 }
pub struct SetupRun { pub start: u32, pub count: u32, pub color: Rgb8 }     // LED indexes inside the zone
pub struct SetupLight { pub session: String, pub runs: Vec<SetupRun> }     // also the heartbeat
pub struct SetupPart { pub name: String, pub kind: ChainPartKind, pub leds: u32 }
pub struct SetupCommit { pub session: String, pub parts: Vec<SetupPart> }  // in chain order; sum of leds is the final length
pub struct SetupEmpty { pub session: String }
pub struct SetupCommitted { pub lights: Vec<LightView>, pub final_leds: u32 }
```

Errors use the API's existing shapes (`ApiFailure`, `crates/canvas-daemon/src/api/mod.rs:175-207`): 404 unknown device or zone, 422 a rule broken (part under 1 LED, sum above `max_leds`, zone not resizable), 409 `conflict` when a session is already open for another zone or the session id is stale, 503 when the store is blocked, and `ApiFailure::internal` carrying the device's own error string when a resize fails.

- [ ] **Step 1: Write the failing tests** in `scene.rs`:

```rust
#[test] fn part_light_id_appends_p_and_counts_from_one()          // "openrgb:mb#z0p1"
#[test] fn chain_part_kind_maps_to_arrangement()                 // Fan->Ring, Strip->Strip, Other->Strip
#[test] fn other_survives_a_round_trip_as_other()                // serde "other" stays Other, not Strip
#[test] fn a_lights_file_without_chains_or_kinds_still_parses()  // an 0.14.0 lights.json
#[test] fn chain_records_round_trip()
```

- [ ] **Step 2:** Run `powershell -NoProfile -File tools/with-build-lock.ps1 cargo test -p canvas-proto scene`. Expected: FAIL, unresolved names.
- [ ] **Step 3:** Implement the types with the derives the other spine types use (`Debug, Clone, PartialEq, Serialize, Deserialize, TS`, `#[ts(export)]`). Update every `Light { .. }` and `LightsFile { .. }` struct literal in the workspace (`grep -rn "Light {" crates`, 42 hits on the baseline, many are other types) with `kind: None` and `chains: Vec::new()` or `..Default::default()`.
- [ ] **Step 4:** Run `cargo test -p canvas-proto`, then `cargo check --workspace`, through the lock. Expected: PASS, new files under `app/web/src/proto/` (`ChainRecord.ts`, `ChainView.ts`, `ChainPartKind.ts`, `SetupCommit.ts`, …) and `PartKind.ts` unchanged.
- [ ] **Step 5:** Commit the spine files and the regenerated `app/web/src/proto/`. Message: `Spine: chains, parts and the setup routes (#338)`.

### Task 2: Fake server hooks

**Can run in parallel with Task 3** (disjoint files). **Needs Task 1: no.**

**Files:**
- Modify: `crates/canvas-adapters/src/fake.rs` (resize handling at `:401`), `crates/canvas-adapters/tests/openrgb_resize.rs`
- Test: `crates/canvas-adapters/tests/openrgb_resize.rs`

**Interfaces:**
- Produces on `FakeOpenRgbServer`: `pub fn resize_count(&self, index: u32) -> u32` (RESIZEZONE requests that arrived), `pub fn fail_resizes(&self, failing: bool)` (the fake leaves the size unchanged and sends no device-list update, so `set_zone_size` returns `Err` when its re-enumerate times out), `pub fn delay_resizes(&self, ms: u64)`.

The fake counts custom-mode writes and LED updates (`fake.rs:213`, `:229`) but not resizes; `zone_leds` (`:234`) and `restart` (`:240`, forgets resizes) exist. "Both resizes happen and no others" cannot be asserted without the count. UNKNOWN until the hardware session: whether real OpenRGB refuses a resize with a reply or with silence. The fake models silence, because `RESIZE_ZONE` gets no reply (`fake.rs:412`).

- [ ] **Step 1: Write the failing tests:** `resize_count_counts_each_request`, `a_failing_resize_returns_an_error_and_keeps_the_size`, `a_delayed_resize_still_lands` (delay 300 ms, size changes afterwards).
- [ ] **Step 2:** Run `cargo test -p canvas-adapters --test openrgb_resize` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement the three methods with state in the existing guarded state struct.
- [ ] **Step 4:** Run the same command and `cargo test -p canvas-adapters`. Expected: PASS.
- [ ] **Step 5:** Commit. Message: `Fakes: count, fail and delay zone resizes (#338)`.

### Task 3: The supervisor stops undoing a raise

**Can run in parallel with Task 2.** Skip this task if Task 0 printed FALSE.

**Files:**
- Modify: `crates/canvas-daemon/src/openrgb_supervisor.rs` (`:314`, `:344`, `:475`, `:497`, `:543`, `:581`), `crates/canvas-daemon/src/state.rs` (new field beside `zones` at `:75`; `ZoneSizes` is at `:168`), `crates/canvas-daemon/src/device_sync.rs` (`supervise_app` at `:140`, the call at `:146`)
- Test: `crates/canvas-daemon/tests/chain_clash.rs` (un-ignore and flip), `crates/canvas-daemon/tests/rescan_supervisor.rs` (new argument)

**Interfaces:**
- Produces in `state.rs`: `pub type ZoneHolds = BTreeMap<(String, u32), u32>;` and `pub zone_holds: Arc<tokio::sync::RwLock<ZoneHolds>>` on `App` (device fingerprint and zone to the raised length). In `openrgb_supervisor.rs`: `pub fn effective_sizes(saved: &ZoneSizes, holds: &ZoneHolds) -> ZoneSizes` that overlays each hold onto a copy of the saved sizes. `supervise_with_rescans` gains one parameter, `holds: Arc<RwLock<ZoneHolds>>`, and builds its size map with `effective_sizes` at the three places it calls `zone_sizes.read().await.clone()` (`:497`, `:543`, `:581`).

Why an overlay: the supervisor then sees "current equals wanted" for a held zone and leaves it, and on an OpenRGB restart during a session it re-applies the raised length. `zones.json` and `App.zones` never see the raise, so `patch_zone` (`crates/canvas-daemon/src/api/devices.rs:62`, which writes the whole `app.zones` map at `:99-110`) and first run (`first_run.rs:383-395`) cannot write it to disk.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn effective_sizes_overlays_a_hold_on_a_saved_size()      // saved {mb:{0:120,1:60}}, hold (mb,0)->300 => {0:300,1:60}
#[test] fn effective_sizes_adds_a_hold_the_file_lacks()          // no saved entry for the device
#[test] fn effective_sizes_with_no_holds_is_the_saved_sizes()
#[tokio::test] async fn refresh_does_not_undo_a_held_raise()     // Task 0's scenario with a hold: after 6 s zone_leds(0,0) == Some(300)
#[tokio::test] async fn an_openrgb_restart_during_a_hold_reapplies_the_raised_length() // server.restart(); wait; Some(300)
#[tokio::test] async fn dropping_the_hold_lets_the_saved_size_return() // remove hold; next refresh: Some(120)
```

- [ ] **Step 2:** Run `cargo test -p canvas-daemon --test chain_clash` through the lock. Expected: FAIL (names missing).
- [ ] **Step 3:** Implement. Update every caller of `supervise_with_rescans` (`device_sync.rs:146`, `tests/rescan_supervisor.rs`) to pass the holds.
- [ ] **Step 4:** Run `cargo test -p canvas-daemon` through the lock. Expected: PASS, with Task 0's ignored test now asserting `Some(300)` and no longer ignored.
- [ ] **Step 5:** Commit. Message: `Daemon: the supervisor leaves a header that a setup holds raised (#338)`.

### Task 4: Restore and intent records

**Can run in parallel with Task 5** (disjoint files). **Needs Task 1.**

**Files:**
- Create: `crates/canvas-daemon/src/chain_records.rs`
- Modify: `crates/canvas-daemon/src/lib.rs` (`pub mod chain_records;`), `crates/canvas-daemon/src/scenes.rs` (`sync_lights` at `:108`)
- Test: unit tests in both files

**Interfaces:**
- Consumes: Task 1's `CHAIN_RESTORE_FILE`, `CHAIN_INTENT_FILE`, `ChainRecord`; `storage::{write_json, read_parsed, snapshot}` (`storage.rs:60`, `:78`, `:270`).
- Produces:

```rust
pub struct RestoreRecord { pub device_id: DeviceId, pub zone_index: u32, pub leds: u32 } // the earlier length
/// Everything commit needs to be undone. `snapshot` is the folder name `storage::snapshot` returned.
pub struct IntentRecord { pub device_id: DeviceId, pub zone_index: u32, pub restore_leds: u32,
                          pub final_leds: u32, pub snapshot: String }
pub fn write_restore(home: &Path, r: &RestoreRecord) -> io::Result<()>;   // atomic, before the first resize
pub fn read_restore(home: &Path) -> Option<RestoreRecord>;                  // None when absent or unreadable (logged, file left)
pub fn delete_restore(home: &Path);
pub fn write_intent(home: &Path, i: &IntentRecord) -> io::Result<()>;
pub fn read_intent(home: &Path) -> Option<IntentRecord>;
pub fn delete_intent(home: &Path);
/// Copies `lights.json`, `scenes.json` and `zones.json` back from the snapshot folder.
pub fn restore_files_from_snapshot(home: &Path, snapshot: &str) -> io::Result<()>;
```

Decision, because the spec says "in one store write" and the store has no multi-file write: `Store::save_lights`, `save_scenes` and `save_zones` are three separate files (`storage.rs:802`, `:809`, `:828`). Scenes added both scenes files to `storage::snapshot`'s keep list (`storage.rs:270-285`, `LIGHTS_FILE`, `SCENES_FILE`, `zones.json`), so a snapshot taken right before commit holds the prior values of all three. The intent record names that snapshot. Start-up replay copies the three files back (Task 7b). The snapshot call and the `SNAPSHOTS_KEPT` prune (`paths.rs:127`) keep it as the newest folder.

`sync_lights` change: a zone whose `ChainRecord` says `Empty` gets no whole-zone light (otherwise the next device list re-adds the light the owner said is empty).

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn a_restore_record_round_trips_and_deletes()
#[test] fn an_unreadable_restore_record_reads_as_none_and_is_left_on_disk() // never deleted unread
#[test] fn an_intent_record_round_trips()
#[test] fn restore_files_from_snapshot_puts_back_all_three_files()          // lights, scenes, zones; other files untouched
#[test] fn sync_lights_skips_a_zone_marked_empty()
#[test] fn sync_lights_still_adds_a_whole_zone_light_for_a_zone_not_set_up()
```

- [ ] **Step 2:** Run `cargo test -p canvas-daemon chain_records scenes::` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement. Writes use `write_json` (atomic: `storage.rs:31` `write_atomic`).
- [ ] **Step 4:** Run again. Expected: PASS.
- [ ] **Step 5:** Commit. Message: `Daemon: the records that make a setup survive a crash (#338)`.

### Task 5: The setup session in the identify gate

**Can run in parallel with Task 4.** **Needs Task 1.**

**Files:**
- Modify: `crates/canvas-daemon/src/identify.rs`, `crates/canvas-daemon/src/transient.rs` (the check at `:103`)
- Test: the `tests` module in `identify.rs`

**Interfaces:**
- Consumes: Task 1's `SetupRun`, `SETUP_TIMEOUT_MS`.
- Produces on `IdentifyGate` (`identify.rs:124`):

```rust
pub fn start_setup(&self, device: DeviceId, zone: u32, session: String, now: Instant) -> bool; // true: caller starts the ticker
pub fn set_runs(&self, session: &str, runs: Vec<SetupRun>, now: Instant) -> bool;              // false: stale session; also renews the timeout
pub fn end_setup(&self, session: &str) -> bool;
pub fn setup_session(&self) -> Option<String>;
// Tick gains: Tick::SetupExpired (the setup timeout ended it; the ticker replays the restore record)
```

Today the gate holds `Option<Target>` whose `runs: Vec<Run>` are all lit `TARGET` white (`identify.rs:38-52`, `frames` at `:250-266`, `start_runs` at `:172`, which the flash uses). `Run` carries no colour and the flash builds it (`api/scenes.rs:467`), so the setup gets its own state in `Inner` (device, zone, `Vec<SetupRun>`, session id, last request) instead of widening `Run`. `frames()` writes each setup run's `color` into the zone's range through the existing `run_of` clamp (`:269`) and leaves every other LED of every device black. `is_active` (`:134`) is true during a setup, so `DeviceSink::submit` (`state.rs:463`) drops engine frames, including other headers waiting their turn. Setup has a timeout of its own (`SETUP_TIMEOUT_MS`, renewed by every `set_runs`), because the identify watchdog is 4 s (`:25`). `transient.rs:103` holds a restore while `session_active()`; it must hold while a setup is open as well.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn a_setup_lights_each_run_in_its_own_colour_and_blacks_everything_else()
#[test] fn runs_are_clamped_to_the_zone()                                   // run past the zone end stops at it
#[test] fn a_stale_session_id_changes_nothing()                             // set_runs with another id returns false
#[test] fn the_setup_times_out_after_the_last_light_request()               // tick at +SETUP_TIMEOUT_MS => Tick::SetupExpired, is_active false
#[test] fn every_light_request_renews_the_timeout()
#[test] fn session_expiry_ends_the_hold_and_asks_for_the_restore()          // Review Focus 2: SetupExpired is returned once, then Idle
#[test] fn a_second_start_for_another_zone_is_refused_while_one_is_open()
#[test] fn a_flash_during_a_setup_does_not_light_over_it()                  // start_runs while a setup is open: setup frames win
#[tokio::test] async fn a_restore_waits_while_a_setup_holds_the_lights()    // transient.rs:103 sibling
```

- [ ] **Step 2:** Run `cargo test -p canvas-daemon identify:: transient::` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement. In `spawn_ticker` (`:364`) handle `Tick::SetupExpired` by calling the replay function Task 7a provides on `App`; until 7a lands, log and call `restore(&app)` (`:390`) only.
- [ ] **Step 4:** Run `cargo test -p canvas-daemon identify::` and `--test identify_log`. Expected: PASS, existing identify tests unchanged.
- [ ] **Step 5:** Commit. Message: `Daemon: a setup session lights several coloured runs (#338)`.

### Task 6: The chain plan (pure)

**Can run in parallel with Task 8** (Rust against web files). **Needs Task 1.**

**Files:**
- Create: `crates/canvas-daemon/src/chain_plan.rs`
- Modify: `crates/canvas-daemon/src/lib.rs`
- Test: unit tests in `chain_plan.rs`

**Interfaces:**
- Consumes: Task 1's `part_light_id`, `ChainPartKind`, `SetupPart`, `ChainRecord`; `scenes::repair` (`scenes.rs:149`), `validate_scene` (`scene.rs:228`), `unassigned` (`:265`).
- Produces:

```rust
/// The even split the UI shows first. The first `len % count` parts get one extra LED. Err when count == 0 or count > len.
pub fn even_split(len: u32, count: u32) -> Result<Vec<u32>, ChainPlanError>;
pub struct CommitPlan { pub lights: LightsFile, pub scenes: ScenesFile, pub final_leds: u32 }
/// Pure. Replaces the zone's lights with one light per part, in chain order, and rewrites every group of every scene.
pub fn plan_commit(lights: &LightsFile, scenes: &ScenesFile, device: &DeviceId, zone: u32,
                   parts: &[SetupPart], max_leds: u32) -> Result<CommitPlan, ChainPlanError>;
/// The empty-header plan: removes the zone's lights from lights and groups, marks the chain Empty.
pub fn plan_empty(lights: &LightsFile, scenes: &ScenesFile, device: &DeviceId, zone: u32) -> CommitPlan;
pub enum ChainPlanError { NoParts, PartUnderOneLed{index}, SumAboveMax{sum,max}, NameTooLong{index}, CountAboveLength }
```

Rules from the spec, pinned:
- A part keeps identity by position: part n of the new list reuses the old part n's id, `reversed` and group membership, and takes the name and kind the owner sent, even when its length changes.
- A part with no old part at that position is new: it joins no group (Unassigned); the web's `useNewLights` announces it (Task 10).
- An old part with no new part at its position leaves every group; a group left empty is dropped, as `repair` does (`scenes.rs:149`).
- Splitting a whole-zone light (first set up): its parts take its place in every group of every scene, in chain order.
- Ids are `part_light_id(device, zone, n)` from 1. Each light gets `leds: Some(LedSlice{start, count})`, `kind: Some(part.kind)`, `arrangement: part.kind.arrangement()`.
- The chain record becomes `SetUp`. A plan is validated by `validate_scene` for every changed scene before it returns.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn even_split_of_96_into_7_gives_14_14_14_14_14_13_13()           // remainder to the first parts; sum == len
#[test] fn a_count_above_the_length_is_refused()                           // Review Focus 5; also count 0
#[test] fn splitting_a_whole_zone_light_puts_its_parts_in_its_groups_in_chain_order()
#[test] fn the_first_run_everything_scene_keeps_lighting_every_part()
#[test] fn parts_keep_reversed_and_groups_by_position_when_lengths_change()
#[test] fn shrinking_a_chain_removes_only_the_missing_parts_from_groups()  // Review Focus 4
#[test] fn growing_a_chain_adds_unassigned_parts()                         // Review Focus 4
#[test] fn a_group_left_empty_by_a_shrink_is_dropped()
#[test] fn changing_a_parts_kind_changes_its_arrangement_but_keeps_its_id()
#[test] fn other_is_stored_as_other_with_a_strip_arrangement()
#[test] fn a_row_of_zero_leds_is_refused()
#[test] fn a_sum_above_the_zone_maximum_is_refused()
#[test] fn plan_empty_removes_the_zone_from_lights_and_groups_and_marks_it_empty()
#[test] fn every_planned_scene_validates()
```

- [ ] **Step 2:** Run `cargo test -p canvas-daemon chain_plan` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement. No I/O, no clock.
- [ ] **Step 4:** Run again. Expected: PASS.
- [ ] **Step 5:** Commit. Message: `Daemon: splitting a chain into parts, by position (#338)`.

### Task 7a: Chain routes, session lifecycle and start-up replay

**Needs Tasks 1, 2, 3, 4, 5.**

**Files:**
- Create: `crates/canvas-daemon/src/api/chains.rs`
- Modify: `crates/canvas-daemon/src/api/mod.rs` (routes beside `:86-87`), `device_sync.rs` (`apply_devices` at `:27`), `main.rs` (the quit path, `write_on_quit` at `:396-402`), `state.rs`
- Test: `crates/canvas-daemon/tests/chains.rs` (new, modelled on `tests/lights.rs`)

**Interfaces:**
- Produces handlers `list_chains`, `start_setup`, `light_setup`, `cancel_setup`, `empty_setup`; and `pub async fn replay_leftovers(app: &Arc<App>)`.
- `list_chains` returns one `ChainView` per resizable zone (`ZoneInfo.resizable`, `canvas-proto/src/device.rs:57`). State: `Empty` if a `ChainRecord` says so, `SetUp` if one does, else `NotSetUp`. First run's 120-LED default sizing (`first_run.rs:324-345`, `:361`) therefore reads as `NotSetUp`.
- `start_setup`: validates the zone is resizable (422 otherwise); refuses a second open session (409); writes the `RestoreRecord` **before** the raise; inserts the hold (Task 3); resizes to `max_leds` through `app.adapter.lock().await.set_zone_size`; re-discovers and refreshes `app.devices` the way `patch_zone` does (`devices.rs:113-122`) without calling `apply_for_devices` for the raised list (the scene is held dark by the gate anyway); starts the gate session and ticker. Answers `SetupStarted`. A failed raise deletes the record and the hold and answers the device's error.
- `light_setup`: `IdentifyGate::set_runs`, pushes the first frame at once (the pattern is `flash_lights`: `start_runs`, then one `tick` and `push`, `api/scenes.rs:523-530`), 409 on a stale session.
- `cancel_setup` and the timeout path: end the gate session, resize to `restore_to`, drop the hold, delete the restore record, `identify::restore` (`identify.rs:390`). If the resize back fails, the record stays and the answer names the error.
- `empty_setup`: restore as cancel, then `plan_empty`, then Task 7b's store write helper.
- `replay_leftovers`: called once from `apply_devices` on the first non-empty device list. Reads the intent record first (Task 7b rolls it back), then the restore record: resize to its length, delete it. Quitting the daemon mid-session calls the same restore (`main.rs:402` is where `write_on_quit` already stops the identify override).

- [ ] **Step 1: Write the failing tests** (daemon on `FakeOpenRgbServer`, real adapter):

```rust
#[tokio::test] async fn list_marks_a_first_run_sized_header_as_not_set_up()
#[tokio::test] async fn start_raises_to_the_zone_maximum_once_and_writes_the_restore_record_first()
#[tokio::test] async fn the_raise_is_never_written_to_zones_json()
#[tokio::test] async fn light_shows_the_runs_on_the_fake_hardware_and_blacks_every_other_device()
#[tokio::test] async fn dragging_causes_no_resize()                           // N light calls; server.resize_count == 1
#[tokio::test] async fn cancel_restores_the_length_and_the_lights_and_deletes_the_record()
#[tokio::test] async fn the_timeout_restores_the_length_and_the_lights()      // no heartbeat for SETUP_TIMEOUT_MS
#[tokio::test] async fn a_second_session_for_another_zone_is_a_409()
#[tokio::test] async fn a_failed_raise_leaves_the_header_and_no_record()      // fail_resizes(true)
#[tokio::test] async fn start_up_replays_a_leftover_restore_record()          // Review Focus 2: record on disk + raised fake; after the first device list zone_leds == earlier length, record gone
#[tokio::test] async fn start_up_leaves_an_unreadable_record_alone_and_says_so()
#[tokio::test] async fn an_empty_header_goes_back_to_its_length_is_not_a_light_and_is_driven_black()
#[tokio::test] async fn a_rescan_during_a_session_does_not_resize_the_header_back()  // Review Focus 1, through the API
```

- [ ] **Step 2:** Run `cargo test -p canvas-daemon --test chains` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p canvas-daemon` and `cargo clippy --workspace --all-targets`. Expected: PASS, no new warnings.
- [ ] **Step 5:** Commit with a `CHANGELOG.md` line under `### Changed`: `Chains: the daemon can hold a header while the app asks what is plugged into it, and puts the length back if the window closes.` Message: `Daemon: the chain setup session and its restore (#338)`.

### Task 7b: Commit, all or nothing

**Needs Tasks 4, 6, 7a.** The apply call is settled by reading: the scene path is `app.scene.lock()`, then `app.draft.gate().lock()`, then `scenes::apply_with(&app, &lock, false)` unless `app.scene.is_taken_over()` (`api/lights.rs:146-152`); a device refresh followed by `scenes::apply_for_devices(&app)` is the `patch_zone` pattern (`api/devices.rs:113-126`, `scenes.rs:640`).

**Files:**
- Modify: `crates/canvas-daemon/src/api/chains.rs`, `crates/canvas-daemon/src/chain_records.rs`, `crates/canvas-daemon/src/storage.rs` (a `Store::apply_chain_plan`)
- Test: `crates/canvas-daemon/tests/chains.rs`

**Interfaces:**
- Produces `commit_setup` and `Store::apply_chain_plan(&mut self, plan: &CommitPlan, zone: (&DeviceId, u32, u32)) -> io::Result<()>`.
- Order (spec "Commit order"): (1) take the scene order lock and the draft gate, validate with `plan_commit`; (2) `storage::snapshot(home, "before-chain-setup")`, then `write_intent` naming it; (3) resize to the sum of the part lengths (the second and last resize); (4) in one `apply_chain_plan` call write `scenes.json`, `lights.json` (with the `ChainRecord`), then `zones.json`; (5) re-discover, apply with `apply_with`, end the hold, end the gate session, delete the intent and the restore record, `identify::restore`. `save_lights` and `save_scenes` refuse when the store is blocked or the scenes are pending (`storage.rs:802-826`), which answers 503 and counts as a failed store write.
- Failure: the resize fails, nothing was written, the header is resized back, the answer carries the device's error and the session stays open for a retry. A store write fails after the resize: memory is put back, the three files are copied back from the snapshot, and the header is resized back. If that also fails, both records stay and start-up replays them.
- Start-up: `replay_leftovers` (Task 7a) rolls back a leftover intent record: `restore_files_from_snapshot`, resize to `restore_leds`, delete both records.

- [ ] **Step 1: Write the failing tests:**

```rust
#[tokio::test] async fn commit_resizes_exactly_twice_over_a_whole_setup()           // start + commit => server.resize_count == 2
#[tokio::test] async fn commit_writes_parts_groups_and_zone_size_together()          // files read back: 7 parts, groups hold them, zones.json holds the sum
#[tokio::test] async fn commit_ends_the_hold_and_the_scene_plays_again()             // the fake devices show the scene's frames
#[tokio::test] async fn a_failed_resize_changes_nothing_and_keeps_the_session_for_a_retry()   // Review Focus 3: fail_resizes(true)
#[tokio::test] async fn a_failed_store_write_resizes_back_and_leaves_the_files()     // Review Focus 3: make lights.json read-only or the store blocked
#[tokio::test] async fn a_failed_rollback_keeps_both_records()
#[tokio::test] async fn start_up_rolls_back_a_leftover_intent()                      // Review Focus 3
#[tokio::test] async fn setting_up_again_keeps_part_identity_in_the_files()          // 7 parts -> 6 parts: ids p1..p6 unchanged, p7 gone from groups
#[tokio::test] async fn commit_with_a_stale_session_is_a_409_and_changes_nothing()
#[tokio::test] async fn commit_during_a_takeover_saves_and_lands_when_the_takeover_ends()  // is_taken_over path
```

- [ ] **Step 2:** Run `cargo test -p canvas-daemon --test chains` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p canvas-daemon` and `cargo clippy --workspace --all-targets`. Expected: PASS.
- [ ] **Step 5:** Commit with a `CHANGELOG.md` line: `Chains: set up a header's parts in one commit that either lands whole or leaves everything as it was.` Message: `Daemon: chain setup commits all or nothing (#338)`.

### Task 8: Editor data layer and split helpers

**Can run in parallel with Task 6 or Task 7a** (web files only). **Needs Task 1.**

**Files:**
- Modify: `app/web/src/lib/endpoints.ts` (beside `lights` at `:198`), `store.ts` (beside `lights` at `:86`, `refreshLights` at `:356`), `demo.ts`, `demoScenarios.ts`, `fixtures.ts`
- Create: `app/web/src/views/chain/split.ts`, `partColours.ts`, `useChainSetup.ts`
- Test: `app/web/src/views/chain/__tests__/split.test.ts`, `useChainSetup.test.ts`

**Interfaces:**
- Consumes: generated `ChainView`, `SetupStarted`, `SetupRun`, `SetupPart`, `SetupCommit`, `ChainPartKind`; the existing `api.lights`, `snap.lights`, `refreshLights`.
- Produces:

```ts
// endpoints.ts: api.chains.list(), .start(device, zone), .light(device, zone, body), .cancel(device, zone),
//   .empty(device, zone, body), .commit(device, zone, body)
// store.ts: snap.chains: ChainView[] | null, refreshChains()
// split.ts
export function evenSplit(len: number, count: number): number[];                 // same rule as the Rust even_split (remainder to the first parts)
export function moveBoundary(lengths: number[], row: number, newLen: number): number[]; // the next row takes the difference; the last row changes the total
export function runsFor(lengths: number[]): { start: number; count: number; color: string }[];
// partColours.ts
export const PART_COLOURS: readonly string[];                                    // fixed, distinct on LEDs; starts with colours that keep distance from white and the accent
export function defaultName(kind: ChainPartKind, position: number): string;      // "Fan 1", "Strip 2", "Part 3" for other
export function renameForKind(current: string, typedByOwner: boolean, kind: ChainPartKind, position: number): string;
// useChainSetup.ts
export function useChainSetup(chain: ChainView): {
  step: "length" | "count" | "confirm" | "done" | "error";
  length: number; setLength(n: number): void;      // throttled light call; never a resize
  count: number; setCount(n: number): void;
  parts: { name: string; kind: ChainPartKind; leds: number; named: boolean }[];
  setPartLeds(i: number, n: number): void; setPartKind(i: number, k: ChainPartKind): void; setPartName(i: number, s: string): void;
  addPart(): void; removePart(i: number): void;
  back(): void; next(): void; confirm(): Promise<void>; nothingLitUp(): Promise<void>; later(): void;
  error: string | null; retry(): void; capped: boolean;                      // capped: the slider reached max_leds and the chain goes on
  foundSummary: string | null;                                               // "7 fans and 1 strip on 2 chains."
};
```

Hook rules: `start` is called on mount; `cancel` on unmount, on Back from step 1, on Later, on `pagehide` and `beforeunload` (a `fetch` with `keepalive: true`, so closing the window cancels); `light` is sent at most every 60 ms with the last value, and re-sent every 2 s as the heartbeat (inside `SETUP_TIMEOUT_MS`, 6 s). Back returns to step 2 or 1 with the entered values kept. Re-setup starts step 1 at `chain.leds` and step 2 at `chain.parts.length`; if the count is unchanged, step 3 starts from the existing parts' names, kinds (`light.kind`, else derived from `arrangement`) and LED counts. `foundSummary` counts `fan` and `strip`/`other` parts across all chains.

- [ ] **Step 1: Write the failing tests:** `evenSplit(96,7) matches the Rust vector`, `moveBoundary gives the difference to the next row`, `moveBoundary on the last row changes the total`, `a row cannot go below 1 LED`, `renameForKind renames unless the owner typed a name`, `setLength sends at most one light call per 60 ms and always the last value`, `the heartbeat resends inside the daemon timeout`, `unmount cancels the session`, `pagehide cancels with keepalive`, `back keeps what was entered`, `re-setup with the same count starts from the existing parts`, `capped is true when the length equals max_leds`, `a failed commit sets error and keeps the step for retry`, `foundSummary reads "7 fans and 1 strip on 2 chains."`.
- [ ] **Step 2:** Run `npm --prefix app/web test -- chain` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm --prefix app/web test` and `npm --prefix app/web run typecheck`. Expected: PASS.
- [ ] **Step 5:** Commit. Message: `Editor: chains in the data layer (#338)`.

### Task 9: The three steps

**Can run in parallel with Task 7b** (web only, against Task 8's hook and the fake daemon). **Needs Tasks 7a and 8.**

**Files:**
- Create: `app/web/src/views/chain/ChainSetup.tsx`, `LengthStep.tsx`, `CountStep.tsx`, `ConfirmStep.tsx`, `PartRow.tsx`, `LengthSlider.tsx`, `chain.module.css`
- Test: `app/web/src/views/chain/__tests__/ChainSetup.test.tsx`

**Interfaces:**
- Consumes: Task 8's `useChainSetup`; the `takeover` store (`app/web/src/views/editor/takeover.ts`, `setTakeover`); `NumberField`, `Button`, `AnimatedNumber` from `app/web/src/components/`.
- Produces: `<ChainSetup chain onClose />`.

Screens and exact copy (spec "The flow"): Step 1 "Drag until the coloured light is the last one on the chain." with the slider, a counting number, arrow keys by one and Shift by ten, a "Nothing lit up" key. Step 2 "How many things are plugged into this chain?" with a number field of 1 or more and one row per part in its colour. Step 3 "Does each one have its own colour?" with, per row, the colour, a name, what it is (fan, strip, other), the LED count; "Yes" finishes. Before the first screen, a line saying why: "The app cannot see what is plugged into a header." The last screen shows `foundSummary`. A failed resize shows the device's own error and a Retry key.

Before writing components, use the `frontend-design` skill with `.build/ui-taste.md` and `hardware.css` to settle the slider and the row anatomy at 1440 and at 390. The slider is a component in the component layer, not a raw `input[type=range]`. The thumb has a larger hit area than its drawing. The colour chips are the same `PART_COLOURS` the daemon is told to light.

- [ ] **Step 1: Write the failing tests:** `step 1 shows the prompt and the counting number`, `the arrow keys move the length by one and Shift by ten`, `Nothing lit up calls the empty route and closes`, `the_slider_stops_at_the_maximum_and_says_why` (Review Focus 5), `step 2 refuses 0 and a count above the length`, `one row per part with the same colour as its light`, `changing a row's LED count moves the boundary and calls light`, `Yes calls commit once`, `Back keeps the entered values`, `a commit error shows the device's text and Retry resends`, `no control is labelled Save or Revert`.
- [ ] **Step 2:** Run `npm --prefix app/web test -- ChainSetup` through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm --prefix app/web run build` and `npm --prefix app/web test`. Expected: PASS.
- [ ] **Step 5:** Capture every state at all six sizes in both themes into `.build/screens/chain/steps/`: loading, step 1 at length 1, mid and max, capped, step 2 with 1 and 7 parts, step 3 even, step 3 after a boundary move, an error, the done screen, re-setup. Look at every image. Run `tools/overflow-check.mjs`. List the images in the PR; an image not looked at is unknown.
- [ ] **Step 6:** Count owner actions from a recorded run on the fake daemon for an even 7-fan chain: drag (1), type a number (2), confirm (3). Paste the recorded request list and the count. Expected: three.
- [ ] **Step 7:** Measure layout shift on step change and on a boundary move: expected 0 px for everything outside the changed element.
- [ ] **Step 8:** Commit with a `CHANGELOG.md` line: `Chains: tell the app what is on a header in three steps: drag, type a number, confirm.` Message: `Editor: the chain setup steps (#338)`.

### Task 10: First run and Devices

**Needs Tasks 7b and 9.** Scenes is merged, so no rebase wait.

**Files:**
- Modify: `app/web/src/views/DevicesView.tsx` (`LightControls` at `:295`, the per-zone light lookup `lightOf` at `:598`, the Size keys at `:667-670` and `:719-722`), `app/web/src/views/scenes/ScenesView.tsx` (mount `ChainTakeover` beside `<Onboarding>` at `:259`, shown once the hello is closed), `app/web/src/views/editor/flow.ts` (a second "later" key beside `putOffUntilNextLaunch`, reusing `launchId`)
- Create: `app/web/src/views/chain/ChainRow.tsx`, `ChainTakeover.tsx`
- Test: `app/web/src/views/chain/__tests__/ChainTakeover.test.tsx`, `ChainRow.test.tsx`

**Interfaces:**
- Consumes: `snap.chains`, `<ChainSetup>`, `launchId` (`flow.ts`), `setTakeover` (`takeover.ts`), `useNewLights` (`app/web/src/views/scenes/newLights.ts`), `api.lights.patch`.
- Produces: `<ChainTakeover />`, mounted by the shell; `<ChainRow chain />` on Devices.

Facts read on the baseline that shape this task:
- The first run is one screen, the hello, and its button ends onboarding (`Onboarding.tsx:30-40` calls `api.completeOnboarding`, then `onDone`). So by the time a header is offered, `Status.onboarding_complete` is already true. The takeover therefore opens when `status.onboarding_complete && chains.some(not_set_up) && !chainLaterThisLaunch()`, not on `!onboarding_complete`. Existing owners whose headers were never set up see it once per launch until they set up, put off, or mark empty; that is the spec's "offered when never set up".
- Scenes is the page behind the takeover (`OnboardingGate.tsx` sends a fresh launch to `/`).
- Devices already edits a light per zone: `LightControls` (name, Strip/Ring, "Runs the other way") assumes one light per zone through `lightOf(zoneIndex)` (`DevicesView.tsx:598`). A set-up header has several lights per zone. The row must list every light of the zone, and for lights that have a `kind` the Strip/Ring key becomes Fan/Strip/Other and sends `kind`.
- A new part after a re-setup is announced by the existing Scenes notice: `useNewLights(all, unassigned)` returns lights no group holds and the page has not seen (`newLights.ts`), shown by `NewLightNotice.tsx`. No new notice is built.
- The standing "Size your ARGB headers" panel (`shell/GlobalBanners.tsx:84-98`) links to the walker (`:41`, `walkHref`). It becomes the re-entry after "Later": same dismiss (`dismissPanel`), title "Set up your headers", action "Set up" opening the takeover.

Behaviour (spec "Where it lives"): headers are offered one after another; an empty header is never offered again; when every chain is set up or empty nothing opens. "Later" closes it until the next app start. Each resizable header row on Devices shows its state (not set up, empty, or its parts), one key ("Set up" or "Set up again"), and edits for part name, kind and "runs the other way" through `PATCH /api/v1/lights/{id}`, without rerunning the flow.

- [ ] **Step 1: Write the failing tests:** `the takeover opens by itself when onboarding is complete and a chain is not set up`, `it stays shut when every chain is set up or empty`, `Later closes it and it stays shut until the next launch`, `headers are offered one after another`, `an empty header is not offered again`, `a Devices row shows not set up, empty and its parts`, `Set up again starts the flow for that header`, `a set up header lists every part with its own controls`, `renaming a part calls the lights route without opening the flow`, `a part with a kind shows Fan, Strip and Other and sends kind`, `a new part appears in the new-light notice`, `leaving the takeover by Escape cancels the session`, `the standing panel opens the takeover and names no walker`.
- [ ] **Step 2:** Run the named test files through the lock. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm --prefix app/web run build` and `npm --prefix app/web test`. Expected: PASS.
- [ ] **Step 5:** Capture the takeover over Scenes, a Devices page with every row state, the row edits and the standing panel at all six sizes in both themes into `.build/screens/chain/wiring/`. Look at every image.
- [ ] **Step 6:** Commit with a `CHANGELOG.md` line: `Chains: setup comes to you on first run, and Devices can redo any header.` Message: `Editor: first run and Devices offer chain setup (#338)`.

### Task 11: Remove the walker and the onboarding session

**Needs Task 10.** Main agent reviews the spine part. Scenes already removed the case, parts and match screens (`app/web/src/views/editor/` now holds `flow.ts`, `Greeting.tsx`, `HelloStep.tsx`, `IdentifyWalker.tsx`, `onboardEffects.ts`, `Onboarding.tsx`, `OnboardingGate.tsx`, `OnboardStep.tsx`, `takeover.ts`), so only the walker and its plumbing remain.

**Files:**
- Delete: `app/web/src/views/editor/IdentifyWalker.tsx`, `app/web/src/views/__tests__/IdentifyWalkerErrors.test.tsx`
- Modify: `app/web/src/views/DevicesView.tsx` (the `IdentifyWalker` import at `:39`, the Size keys at `:667-670` and `:719-722`, the walker block at `:731-738`, the `?walk=` handling at `:398-420`), `app/web/src/shell/GlobalBanners.tsx` (done in Task 10; here remove `walkHref`), `app/web/src/lib/endpoints.ts:240` (`onboardingSession`, which no component calls), `crates/canvas-daemon/src/api/devices.rs:189` (`onboarding_session`), `api/mod.rs:128`, `identify.rs` (`SESSION_TIMEOUT` at `:31`, `start_session`, `end_session`, `session_active`, `session_last`), `transient.rs:103`, `crates/canvas-daemon/tests/api.rs:2070-2083`, `crates/canvas-proto/src/api.rs` (`ONBOARDING_SESSION` at `:146`, `OnboardingSession` at `:341`)
- Keep: `Status.onboarding_complete` (`api.rs:298`), now meaning "first-run setup was finished or put off for good" (spec "What is removed"); `IdentifyRequest` (`api.rs:375`) and the identify route, still used by Devices' per-zone identify; the flash.

- [ ] **Step 1:** Search first: `Grep` for `IdentifyWalker`, `useIdentifyWalker`, `walk=`, `walkHref`, `onboardingSession`, `OnboardingSession`, `session_active`, `start_session`, `end_session`, `show_zone_panel` across `crates/`, `app/web/src/`, `app/src-tauri/` and `tools/`. `crates/canvas-daemon/src/bin/cli.rs:157` mentions `show_zone_panel`; read it and decide with the main agent. List each hit in the PR as removed, kept, or unrelated.
- [ ] **Step 2:** Add `no file or text names the walker` (a source grep test) and `the Devices row has no Size key`. Run them. Expected: FAIL.
- [ ] **Step 3:** Delete and modify as listed. Regenerate proto types (`cargo test -p canvas-proto`).
- [ ] **Step 4:** Through the lock: `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `npm --prefix app/web run build`, `npm --prefix app/web test`. Expected: PASS. Compare counts with Task 0's baseline and explain every test that went away.
- [ ] **Step 5:** Re-run Step 1's search. Expected: only "kept" and "unrelated" hits.
- [ ] **Step 6:** Count lines again with Task 0's command plus `wc -l` over `app/web/src/views/chain/*`, `crates/canvas-daemon/src/chain_*.rs` and `api/chains.rs`. Put removed and added totals in the PR.
- [ ] **Step 7:** Commit with a `CHANGELOG.md` line under `### Removed`: `The per-LED walk and the onboarding session. Chain setup replaces them.` Message: `Remove the walker and the onboarding session (#338)`.

### Task 12: Proof, docs and hand-off

**Files:**
- Modify: `CHANGELOG.md`, `README.md` and `docs/effects.md` where they describe the walk or the Size key, `docs/bug-classes.md` (only if a class was found), the wiki page for canvas-rgb (outside the repo; main agent)
- Create: `.build/screens/chain/final/` (main checkout, not committed)

- [ ] **Step 1:** Rebase `feat/chain-setup` on `feat/scenes`. Run all four gates through the lock. Expected: PASS.
- [ ] **Step 2:** Final capture: every state of the three steps, the takeover over Scenes, and Devices at all six sizes in both themes. Look at every image. Count them per view in the PR.
- [ ] **Step 3:** Run the fake daemon and drive a full first run through the headless capture: two resizable headers (one sized to 120 by first run, one with no `zones.json` entry), one set up as 7 fans, one marked empty. Paste the resulting `lights.json` and `scenes.json` parts and the fake server's `resize_count` per header (expected 2 for each header: the raise, then the final resize or the restore; the empty header's final length equals its start).
- [ ] **Step 4:** Time the first run on the fakes from takeover open to done for two chains. Report the measured seconds against the spec targets (under one minute for one chain; 10 to 15 minutes for the whole first run), not a verdict. A run that did not happen is UNKNOWN.
- [ ] **Step 5:** Dispatch the `adversarial-reviewer` agent with: the spec, this plan, the PR diff, and the claims "a resize happens at most twice per setup", "every leave path restores the length", "commit is all or nothing" and "nothing reads the walker". Fix what it disproves. Report what it could not check as unknown.
- [ ] **Step 6:** Mark the draft PR ready. The PR body lists: gates with counts, the Task 0 verdict, the action count, screenshot counts, layout-shift numbers, lines removed and added, and every unknown with the command that settles it. Do not cut 0.15.0: `tools/set-version.ps1` and the tag are Luke's call.

---

## Parallel schedule (max 2 builders)

| Wave | Builder A | Builder B |
|---|---|---|
| 0 | Task 0 (main agent), Task 1 (main agent) | none |
| 1 | Task 2 (fakes) | Task 3 (supervisor) |
| 2 | Task 4 (records) | Task 5 (gate session) |
| 3 | Task 6 (plan) | Task 8 (web data layer) |
| 4 | Task 7a (routes, lifecycle) | none: shares `chains.rs` and `identify.rs` with the next |
| 5 | Task 7b (commit) | Task 9 (steps UI) |
| 6 | Task 10 | none |
| 7 | Task 11, then Task 12 | none |

## Hardware checks queued for Luke (not part of any task)

One batched session, with his OK in chat, after the PR is ready and while he is away from the PC. It also covers the already-queued sizing of his JARGB headers. Lighting only: SignalRGB is quit, OpenRGB runs non-elevated, the fans stay on BIOS control for those minutes (`CLAUDE.md`).

1. **Speed of the raise and the slider (settles the spec's unknown).** Measure with the daemon's log timestamps: time from `start` to the first lit LED at the header maximum, and time from a `light` call to the light changing while the slider moves. If the slider lags more than 250 ms, apply the spec's fallback: cap the raise and let the slider's top end raise the cap in steps. This changes Task 7a and Task 9; it is a follow-up PR, named in advance.
2. **Colours.** Light 7 parts with `PART_COLOURS` on the real fans. The agent confirms from the OpenRGB readback that each run received its colour. Luke answers one yes or no with the lights in front of him: "do any two neighbouring fans look the same colour?" That item is his to answer, because only his eyes see the physical LEDs.
3. **Cancel and crash restore.** Read the header length from OpenRGB before, start a setup, cancel; read again. Start a setup, kill the daemon, start it; read again. Expected: both equal the first reading.
4. **Even chain.** Set up the 8-LED chain of seven fans as 7 parts and confirm `lights.json` holds 7 parts and the Everything group holds them.
5. **Real MSI maximum.** Drag to the device maximum and confirm the capped message appears. Record `max_leds` the device reports.

Restore step, always, even when a check fails: relaunch SignalRGB with the launcher path written in `CLAUDE.md` (main checkout), then confirm in the process list that `SignalRgb` is running again. Then read each header's length back and compare with the first reading from check 3. Release step: when Luke is satisfied he runs `tools/set-version.ps1 0.15.0` and pushes the `v0.15.0` tag himself (`docs/releasing.md`).

## Self-review notes

- Spec coverage: three steps (Tasks 6, 8, 9); first run and Devices (10); leaving by every route (3, 5, 7a); restore across crash (4, 7a, 7b); commit order (7b); longest allowed length and the capped message (7a, 9); empty header (4, 6, 7a); one setup session (5); step 3 details (6, 8, 9); set up again (6, 8); scene effects (6); removals (11); proof (0, 9, 12 and the hardware session). Spec "Left to the builder": the colour set (`PART_COLOURS`, Task 8, checked in hardware check 2), the remainder rule (`even_split`, Task 6), slider feel (Task 9).
- Decisions made in this plan that the spec left open, for the main agent to confirm: header state lives in `lights.json` as `chains`; the kind is stored on the light as `ChainPartKind` with its own `Other` value (named `ChainPartKind` because `PartKind` is taken by `layout.rs:137`); the remainder goes to the first parts; the zone hold is an in-memory overlay on the supervisor's sizes; commit rolls back from a `storage::snapshot` that Scenes extended to cover the scenes files; the first-run takeover trigger is `onboarding_complete` with a chain not set up, because the hello now ends onboarding before any header is offered.
