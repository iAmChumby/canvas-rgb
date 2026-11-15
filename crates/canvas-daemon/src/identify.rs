//! The identify override: while it is active the engine's frames reach no device.
//!
//! Luke could not find the light because `identify` wrote one LED to the adapter and the
//! render loop then wrote effect frames over it (or, with output paused, the frozen frame stayed
//! next to it). The override fixes the order of things: while a target is set, every
//! [`DeviceSink`](crate::state::DeviceSink) drops the engine's frames, and a ticker owned by
//! the daemon sends every device an all-black frame, except the target run, which is bright
//! white at full brightness. The ticker does not care whether output is paused.
//!
//! It ends on `stop: true`, on a newer identify replacing it, or [`WATCHDOG`] after the last
//! request, so a closed window can never leave the PC dark. Ending restores the normal
//! frames: the engine's next frame when running, one black frame when paused.
//!
//! A chain setup rides the same override with its own state: the owner is building a chain by
//! hand, so one zone of one device is lit in several colours at once ([`crate::identify::Run`]
//! carries no colour, and widening it would change every identify caller). Every other LED is
//! black, and the setup's frames outrank a flash. It ends when its session ends, or
//! [`SETUP_TIMEOUT`] after the last light request, and the ticker restores on that end.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canvas_proto::api::{Power, SetupRun};
use canvas_proto::scene::SETUP_TIMEOUT_MS;
use canvas_proto::{DeviceId, DeviceInfo, Rgb8};

use crate::state::App;

/// How long after the last request the override ends by itself. The page asks again every ~2 s,
/// and every 125 ms while it chases a part.
pub const WATCHDOG: Duration = Duration::from_secs(4);
/// How often the ticker re-sends the override frames (the device may drop one).
pub const TICK: Duration = Duration::from_millis(100);
/// The colour of the target run.
pub const TARGET: Rgb8 = Rgb8::WHITE;
/// How long after the last light request a chain setup ends by itself (the spine's number).
pub const SETUP_TIMEOUT: Duration = Duration::from_millis(SETUP_TIMEOUT_MS);

/// The id prefix of devices whose adapter lights the whole device (the ROYUAN keyboard).
const WHOLE_DEVICE_PREFIX: &str = "royuan:";

/// One run of LEDs the override lights: `count` LEDs from `led` in `zone` of `device`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub device: DeviceId,
    pub zone: u32,
    pub led: u32,
    pub count: u32,
}

/// What the override lights: one or more runs (a light flash names several at once).
#[derive(Debug, Clone)]
struct Target {
    runs: Vec<Run>,
    last_request: Instant,
}

/// A chain setup: one zone of one device, lit in several colours until it ends.
///
/// The runs are the spine's [`SetupRun`] (LED indexes inside the zone) and the device and zone
/// are the setup's own: a light request names neither, because the route that opened the setup
/// already chose them.
#[derive(Debug, Clone)]
struct Setup {
    /// The device whose zone is open.
    device: DeviceId,
    /// The zone that is open: every run's indexes are inside it.
    zone: u32,
    /// The LEDs the last light request named, in their own colours.
    runs: Vec<SetupRun>,
    /// The id the setup answers to: a request from any other session changes nothing.
    session: String,
    /// When the last `start_setup`/`set_runs` arrived; [`SETUP_TIMEOUT`] runs from here.
    last_request: Instant,
}

#[derive(Default)]
struct Inner {
    target: Option<Target>,
    /// The chain setup while one is open; one at a time.
    setup: Option<Setup>,
    /// The writer channel of every device the daemon drives.
    sinks: HashMap<DeviceId, tokio::sync::mpsc::Sender<Vec<Rgb8>>>,
    ticker_running: bool,
}

/// What one tick of the ticker found.
#[derive(Debug, PartialEq)]
pub enum Tick {
    /// Nothing is active.
    Idle,
    /// The override frames to send now.
    Frames(Vec<(DeviceId, Vec<Rgb8>)>),
    /// The watchdog ended the override.
    Expired,
    /// The setup's timeout ended it: the ticker restores the chain and stops.
    SetupExpired,
}

/// What happened to one frame of the override (#81).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// The frame is in the writer's channel: the writer will send it.
    Queued,
    /// The frame was not sent.
    Dropped(DropReason),
}

/// Why a frame of the override did not reach the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// The device has no writer: it is never driven, or its writer is gone.
    NoSink,
    /// The writer's channel is full: the device is behind.
    Full,
    /// The writer's channel is closed: the device is gone.
    Closed,
}

impl DropReason {
    /// The sentence part of the dropped line: why the frame did not go out.
    pub fn describe(self) -> &'static str {
        match self {
            Self::NoSink => "the device has no writer",
            Self::Full => "the device's writer is behind",
            Self::Closed => "the device's writer is gone",
        }
    }
}

/// One frame of the override and what happened to it (#87).
///
/// The frame itself is not kept: it went to the writer's channel. What the log needs is how
/// many LEDs it had, how many of them were white, and where the white run starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    /// The device the frame was for.
    pub device: DeviceId,
    /// What the writer's channel did with the frame.
    pub outcome: PushOutcome,
    /// The LEDs the frame had.
    pub leds: usize,
    /// The LEDs of the frame that were not black.
    pub lit: usize,
    /// The index of the first LED that was not black, in the device's frame.
    pub first: Option<usize>,
}

/// The override state, shared by the API handler, the sinks and the ticker.
#[derive(Default)]
pub struct IdentifyGate {
    inner: Mutex<Inner>,
}

impl IdentifyGate {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// True while the engine's frames must not reach the devices.
    pub fn is_active(&self) -> bool {
        let inner = self.lock();
        inner.target.is_some() || inner.setup.is_some()
    }

    /// True while a chain setup holds the lights: the transient restore waits for it, so a
    /// greeting never lands in the middle of a setup.
    pub fn holds_lights(&self) -> bool {
        self.lock().setup.is_some()
    }

    /// Opens a chain setup on one zone of one device.
    ///
    /// `None` when a setup is already open, on any zone of any device: one setup at a time, so
    /// two windows cannot fight over the lights. `Some(true)` means the caller starts the ticker;
    /// `Some(false)` means one is already running.
    ///
    /// The setup takes the lights: an identify or flash target that was showing ends here, so
    /// nothing outlives the setup and its timeout always reaches the ticker.
    pub fn start_setup(&self, device: DeviceId, zone: u32, session: String, now: Instant) -> Option<bool> {
        let mut inner = self.lock();
        if inner.setup.is_some() {
            return None;
        }
        inner.target = None;
        inner.setup = Some(Setup { device, zone, runs: Vec::new(), session, last_request: now });
        let start = !inner.ticker_running;
        inner.ticker_running = true;
        Some(start)
    }

    /// Replaces the setup's runs with what the last light request named, and renews its timeout.
    ///
    /// False when no setup is open, or the setup belongs to another session: a stale window
    /// changes nothing, neither its runs nor its timeout.
    pub fn set_runs(&self, session: &str, runs: Vec<SetupRun>, now: Instant) -> bool {
        let mut inner = self.lock();
        let Some(setup) = inner.setup.as_mut() else { return false };
        if setup.session != session {
            return false;
        }
        setup.runs = runs;
        setup.last_request = now;
        true
    }

    /// Ends this session's setup. True when one was open; another session's id ends nothing.
    pub fn end_setup(&self, session: &str) -> bool {
        let mut inner = self.lock();
        if inner.setup.as_ref().is_some_and(|setup| setup.session == session) {
            inner.setup = None;
            return true;
        }
        false
    }

    /// The open setup's session id, if any.
    pub fn setup_session(&self) -> Option<String> {
        self.lock().setup.as_ref().map(|setup| setup.session.clone())
    }

    /// Registers a device's writer channel so the ticker can reach it.
    pub fn register(&self, device: DeviceId, tx: tokio::sync::mpsc::Sender<Vec<Rgb8>>) {
        self.lock().sinks.insert(device, tx);
    }

    /// Sets (or replaces) the target. Returns true when the caller must start the ticker.
    pub fn start(&self, device: DeviceId, zone: u32, led: u32, count: u32, now: Instant) -> bool {
        self.start_runs(vec![Run { device, zone, led, count }], now)
    }

    /// Sets (or replaces) the target as several runs at once: the lights flash (Scenes) names every
    /// light of the lights it was asked for. Each run is at least one LED. Same watchdog, same
    /// ticker. Returns true when the caller must start the ticker.
    ///
    /// While a chain setup is open the request is refused (false, no target): the setup holds the
    /// lights, and a target set now would outlive it.
    pub fn start_runs(&self, runs: Vec<Run>, now: Instant) -> bool {
        let runs = runs.into_iter().map(|r| Run { count: r.count.max(1), ..r }).collect();
        let mut inner = self.lock();
        if inner.setup.is_some() {
            return false;
        }
        inner.target = Some(Target { runs, last_request: now });
        let start = !inner.ticker_running;
        inner.ticker_running = true;
        start
    }

    /// Ends the override. Returns true when one was active.
    pub fn stop(&self) -> bool {
        self.lock().target.take().is_some()
    }

    /// One ticker step at `now`: the frames to send, or the end of the override.
    pub fn tick(&self, now: Instant, devices: &[DeviceInfo]) -> Tick {
        let mut inner = self.lock();
        if inner.target.is_none() && inner.setup.is_none() {
            inner.ticker_running = false;
            return Tick::Idle;
        }
        // The setup's own timeout. Its end is its own answer: the ticker restores on it, and
        // the setup is cleared so the tick after it is idle.
        let setup_ended = inner
            .setup
            .as_ref()
            .is_some_and(|setup| now.saturating_duration_since(setup.last_request) >= SETUP_TIMEOUT);
        if setup_ended {
            inner.setup = None;
        }
        if inner.target.as_ref().is_some_and(|t| now.saturating_duration_since(t.last_request) >= WATCHDOG) {
            inner.target = None;
        }
        if setup_ended {
            inner.ticker_running = false;
            return Tick::SetupExpired;
        }
        if inner.target.is_none() && inner.setup.is_none() {
            inner.ticker_running = false;
            return Tick::Expired;
        }
        Tick::Frames(frames(inner.target.as_ref(), inner.setup.as_ref(), devices))
    }

    /// Sends frames to the registered writers. A full channel drops the frame; the next
    /// tick sends it again.
    ///
    /// Returns what happened to every frame (#81): a frame that the writer's channel did not
    /// take was not sent, and the log may not say it was. Every frame comes back with its
    /// own outcome, so a frame and its outcome can never disagree (#87). The frames are
    /// moved into the writers' channels, not copied.
    pub fn push(&self, frames: Vec<(DeviceId, Vec<Rgb8>)>) -> Vec<Pushed> {
        let mut inner = self.lock();
        let mut pushed = Vec::with_capacity(frames.len());
        for (device, colors) in frames {
            let leds = colors.len();
            let lit = colors.iter().filter(|c| **c != Rgb8::BLACK).count();
            let first = colors.iter().position(|c| *c != Rgb8::BLACK);
            let mut closed = false;
            let outcome = match inner.sinks.get(&device) {
                Some(tx) => match tx.try_send(colors) {
                    Ok(()) => PushOutcome::Queued,
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        PushOutcome::Dropped(DropReason::Full)
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        closed = true;
                        PushOutcome::Dropped(DropReason::Closed)
                    }
                },
                None => PushOutcome::Dropped(DropReason::NoSink),
            };
            if closed {
                inner.sinks.remove(&device);
            }
            pushed.push(Pushed {
                device,
                outcome,
                leds,
                lit,
                first,
            });
        }
        pushed
    }
}

/// Every device's frame under this target or setup: black everywhere but the lit runs.
///
/// A setup's frames win over a flash: while one is open the target is not drawn at all, so a
/// Scenes flash cannot light over what the owner is setting up.
fn frames(
    target: Option<&Target>,
    setup: Option<&Setup>,
    devices: &[DeviceInfo],
) -> Vec<(DeviceId, Vec<Rgb8>)> {
    devices
        .iter()
        .filter(|d| d.connected)
        .map(|d| {
            let mut colors = vec![Rgb8::BLACK; d.led_count() as usize];
            if let Some(setup) = setup {
                if d.id == setup.device {
                    for run in &setup.runs {
                        if let Some(range) = run_of(d, setup.zone, run.start, run.count) {
                            colors[range].fill(run.color);
                        }
                    }
                }
            } else {
                for run in target.iter().flat_map(|t| &t.runs).filter(|r| d.id == r.device) {
                    if d.id.0.starts_with(WHOLE_DEVICE_PREFIX) {
                        colors.fill(TARGET);
                    } else if let Some(range) = run_of(d, run.zone, run.led, run.count) {
                        colors[range].fill(TARGET);
                    }
                }
            }
            (d.id.clone(), colors)
        })
        .collect()
}

/// The range of the device's frame a run lights, clamped to its zone.
fn run_of(device: &DeviceInfo, zone: u32, led: u32, count: u32) -> Option<std::ops::Range<usize>> {
    let mut offset = 0u32;
    for info in &device.zones {
        if info.index == zone {
            let start = led.min(info.led_count);
            let end = led.saturating_add(count).min(info.led_count);
            return (start < end).then(|| (offset + start) as usize..(offset + end) as usize);
        }
        offset += info.led_count;
    }
    None
}

/// Says what one identify request did, and whether the frame went out (#81). OpenRGB answers a
/// frame with nothing, so when a stick ignores an identify, the log of what went out is the only
/// record: the controller index (the one OpenRGB uses), the device, the LEDs lit, and that every
/// other device got an all-black frame.
///
/// The line says the frame was queued only when the writer's channel took it. A frame that was
/// dropped (no writer, or a writer that is behind) gets its own line saying so and why. One line
/// per request (the page chases a part every 125 ms and lights a whole part every 2 s), not per
/// tick.
///
/// Every pushed frame carries its own outcome, so a frame that was made is always a frame that
/// was pushed: there is no third case to report (#87).
pub fn log_sent(device: &DeviceInfo, pushed: &[Pushed]) {
    let Some(frame) = pushed.iter().find(|frame| frame.device == device.id) else {
        tracing::warn!(device = %device.id, "identify: no frame was made for {}; it is not connected", device.id);
        return;
    };
    match frame.outcome {
        PushOutcome::Queued => {
            let others = queued_others(device, pushed);
            let others_text = others_text(others);
            let run = match frame.first {
                Some(index) => {
                    let (zone, led) = zone_and_led(device, index);
                    format!(
                        "{} of {} LEDs white from zone {zone} LED {led}",
                        frame.lit, frame.leds
                    )
                }
                None => format!("no LED white of {}", frame.leds),
            };
            tracing::info!(
                device = %device.id, adapter = %device.adapter, controller = device.adapter_index,
                name = %device.name, location = %device.location, leds = frame.leds, lit = frame.lit, others,
                "identify queued for {} controller {} ({}; {}, {}): {run}; {others_text}",
                device.adapter, device.adapter_index, device.id, device.name, device.location
            );
        }
        PushOutcome::Dropped(reason) => {
            tracing::info!(
                device = %device.id, adapter = %device.adapter, controller = device.adapter_index,
                name = %device.name, location = %device.location, reason = %reason.describe(),
                "identify dropped for {} controller {} ({}; {}, {}): {}; nothing was sent",
                device.adapter, device.adapter_index, device.id, device.name, device.location, reason.describe()
            );
        }
    }
}

/// How many of the other devices' frames were queued for their writers: only those really got
/// an all-black frame.
fn queued_others(device: &DeviceInfo, pushed: &[Pushed]) -> usize {
    pushed
        .iter()
        .filter(|pushed| pushed.device != device.id && pushed.outcome == PushOutcome::Queued)
        .count()
}

/// The sentence about the other devices, in the singular when there is one.
fn others_text(count: usize) -> String {
    match count {
        0 => "no other device gets an all-black frame".to_string(),
        1 => "1 other device gets an all-black frame".to_string(),
        many => format!("{many} other devices get an all-black frame"),
    }
}

/// The zone and the LED inside it of an index in the device's frame. The frame is the zones in
/// order, so the index says which zone it is in.
fn zone_and_led(device: &DeviceInfo, index: usize) -> (u32, u32) {
    let mut offset = 0usize;
    for zone in &device.zones {
        let end = offset + zone.led_count as usize;
        if index < end {
            return (zone.index, (index - offset) as u32);
        }
        offset = end;
    }
    (0, index as u32)
}

/// Starts the ticker task for the app.
pub fn spawn_ticker(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let devices = app.devices.read().await.clone();
            match app.identify.tick(Instant::now(), &devices) {
                Tick::Idle => return,
                Tick::Frames(frames) => {
                    app.identify.push(frames);
                }
                Tick::Expired => {
                    tracing::info!("identify ended by timeout ({WATCHDOG:?})");
                    restore(&app).await;
                    return;
                }
                Tick::SetupExpired => {
                    tracing::info!("the chain setup ended by timeout ({SETUP_TIMEOUT:?} after the last light request); restoring");
                    // The same restore a cancel runs: the header's earlier length, the hold, the
                    // record and the scene (#338). A resize the device refuses keeps the record
                    // for the next start to replay. A setup clears any identify target when it
                    // opens and refuses a flash while it is open, so nothing else outlives it.
                    if let Err(e) = crate::api::chains::finish_setup(&app).await {
                        tracing::warn!(error = %e.message, "the header could not be put back; the restore record stays for the next start");
                    }
                    return;
                }
            }
        }
    });
}

/// Puts the lights back after the override: the engine resumes by itself when running; a
/// paused output needs its one black frame again.
///
/// The identify is a sink-level override: it never took the lights away from what the engine
/// runs. A draft on the lights stays on the lights, the onboarding effect is not killed, and
/// the scene is not rebuilt. Nothing is applied here.
pub async fn restore(app: &Arc<App>) {
    let power = app.store.read().await.config.settings.power;
    if power == Power::Paused {
        app.engine.write_solid(Rgb8::BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::api::SetupRun;
    use canvas_proto::device::{DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::scene::SETUP_TIMEOUT_MS;

    fn zone(index: u32, leds: u32) -> ZoneInfo {
        ZoneInfo {
            index,
            name: format!("z{index}"),
            kind: ZoneKind::Linear,
            led_count: leds,
            min_leds: 1,
            max_leds: 100,
            resizable: false,
        }
    }

    fn device(id: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.into()),
            name: id.into(),
            vendor: "t".into(),
            kind: DeviceKind::Motherboard,
            adapter: "fake".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones,
            connected: true,
        }
    }

    fn lit(frame: &[Rgb8]) -> Vec<usize> {
        frame.iter().enumerate().filter(|(_, c)| **c != Rgb8::BLACK).map(|(i, _)| i).collect()
    }

    fn setup() -> (IdentifyGate, Vec<DeviceInfo>, Instant) {
        let devices = vec![
            device("openrgb:board", vec![zone(0, 10), zone(1, 20)]),
            device("openrgb:ram", vec![zone(0, 8)]),
        ];
        (IdentifyGate::default(), devices, Instant::now())
    }

    fn frames_of(tick: Tick) -> Vec<(DeviceId, Vec<Rgb8>)> {
        match tick {
            Tick::Frames(f) => f,
            other => panic!("expected frames, got {other:?}"),
        }
    }

    #[test]
    fn a_non_target_device_is_all_black_during_an_override() {
        let (gate, devices, t0) = setup();
        gate.start(DeviceId("openrgb:board".into()), 1, 3, 4, t0);
        let frames = frames_of(gate.tick(t0, &devices));
        let ram = frames.iter().find(|(id, _)| id.0 == "openrgb:ram").unwrap();
        assert_eq!(ram.1.len(), 8);
        assert!(ram.1.iter().all(|c| *c == Rgb8::BLACK));
    }

    #[test]
    fn the_target_run_is_lit_and_the_rest_of_its_zone_is_black() {
        let (gate, devices, t0) = setup();
        gate.start(DeviceId("openrgb:board".into()), 1, 3, 4, t0);
        let frames = frames_of(gate.tick(t0, &devices));
        let board = &frames.iter().find(|(id, _)| id.0 == "openrgb:board").unwrap().1;
        assert_eq!(board.len(), 30);
        // Zone 1 starts at 10; the run is LEDs 3..7 of it.
        assert_eq!(lit(board), vec![13, 14, 15, 16]);
        assert!(lit(board).iter().all(|&i| board[i] == TARGET));
    }

    #[test]
    fn the_run_is_clamped_to_its_zone_and_defaults_to_one_led() {
        let (gate, devices, t0) = setup();
        gate.start(DeviceId("openrgb:board".into()), 0, 8, 5, t0);
        let board = frames_of(gate.tick(t0, &devices)).remove(0).1;
        assert_eq!(lit(&board), vec![8, 9], "LEDs 8..13 of a 10-LED zone stop at the zone end");
        gate.start(DeviceId("openrgb:board".into()), 0, 2, 1, t0);
        let board = frames_of(gate.tick(t0, &devices)).remove(0).1;
        assert_eq!(lit(&board), vec![2]);
    }

    #[test]
    fn a_whole_light_device_lights_whole() {
        let gate = IdentifyGate::default();
        let devices = vec![device("royuan:kb", vec![zone(0, 1)]), device("openrgb:ram", vec![zone(0, 8)])];
        let t0 = Instant::now();
        gate.start(DeviceId("royuan:kb".into()), 0, 0, 1, t0);
        let frames = frames_of(gate.tick(t0, &devices));
        assert_eq!(frames[0].1, vec![TARGET]);
        assert!(frames[1].1.iter().all(|c| *c == Rgb8::BLACK));
    }

    #[test]
    fn the_watchdog_restores_rendering() {
        let (gate, devices, t0) = setup();
        gate.start(DeviceId("openrgb:board".into()), 0, 0, 1, t0);
        assert!(gate.is_active());
        assert!(matches!(gate.tick(t0 + Duration::from_millis(3900), &devices), Tick::Frames(_)));
        assert!(gate.is_active());
        assert_eq!(gate.tick(t0 + WATCHDOG, &devices), Tick::Expired);
        assert!(!gate.is_active(), "engine frames flow again");
        assert_eq!(gate.tick(t0 + WATCHDOG, &devices), Tick::Idle);
    }

    #[test]
    fn a_repeat_request_keeps_the_watchdog_away() {
        let (gate, devices, t0) = setup();
        let id = DeviceId("openrgb:board".into());
        gate.start(id.clone(), 0, 0, 1, t0);
        gate.start(id, 0, 0, 1, t0 + Duration::from_secs(3));
        assert!(matches!(gate.tick(t0 + Duration::from_secs(6), &devices), Tick::Frames(_)));
        assert_eq!(gate.tick(t0 + Duration::from_secs(7), &devices), Tick::Expired);
    }

    #[test]
    fn the_sent_line_names_the_controller_and_the_lit_run_and_a_missing_frame_warns() {
        let (gate, mut devices, t0) = setup();
        devices[1].adapter = "openrgb".into();
        devices[1].adapter_index = 4;
        devices[1].location = "SMBus 0x71".into();
        gate.start(DeviceId("openrgb:ram".into()), 0, 5, 2, t0);
        let frames = frames_of(gate.tick(t0, &devices));
        // No writer is registered, so the frames go nowhere: the line must not say they were sent.
        let pushed = gate.push(frames);
        let (_, text) = crate::test_logs::captured(|| log_sent(&devices[1], &pushed));
        assert!(text.contains("INFO"), "{text}");
        assert!(text.contains("identify dropped for openrgb controller 4 (openrgb:ram; openrgb:ram, SMBus 0x71)"), "{text}");
        assert!(text.contains("the device has no writer"), "{text}");
        assert!(text.contains("nothing was sent"), "{text}");
        assert!(!text.contains("identify queued"), "nothing was queued: {text}");

        // A device with no frame (not connected) is a warning, not a silent line.
        let (_, text) = crate::test_logs::captured(|| log_sent(&devices[1], &[]));
        assert!(text.contains("WARN") && text.contains("no frame was made for openrgb:ram"), "{text}");
    }

    /// An identify that the writer took is logged as queued, with the controller and the lit run
    /// (#81). The run is named the way the request named it: the zone and the LED in it (#86).
    #[test]
    fn an_identify_that_reached_its_writer_is_logged_as_queued() {
        let (gate, mut devices, t0) = setup();
        devices[1].adapter = "openrgb".into();
        devices[1].adapter_index = 4;
        devices[1].location = "SMBus 0x71".into();
        // A writer for every device, so every frame is queued.
        let (board_tx, _board_rx) = tokio::sync::mpsc::channel(4);
        let (ram_tx, mut ram_rx) = tokio::sync::mpsc::channel(4);
        gate.register(DeviceId("openrgb:board".into()), board_tx);
        gate.register(DeviceId("openrgb:ram".into()), ram_tx);
        gate.start(DeviceId("openrgb:ram".into()), 0, 5, 2, t0);

        let frames = frames_of(gate.tick(t0, &devices));
        let pushed = gate.push(frames);
        assert!(pushed.iter().all(|pushed| pushed.outcome == PushOutcome::Queued), "{pushed:?}");
        let (_, text) = crate::test_logs::captured(|| log_sent(&devices[1], &pushed));
        assert!(text.contains("INFO"), "{text}");
        assert!(text.contains("identify queued for openrgb controller 4 (openrgb:ram; openrgb:ram, SMBus 0x71)"), "{text}");
        assert!(text.contains("2 of 8 LEDs white from zone 0 LED 5"), "{text}");
        assert!(text.contains("1 other device gets an all-black frame"), "{text}");
        assert!(!text.contains("nothing was sent"), "the frame was queued: {text}");
        // The frame really went to the writer.
        let frame = ram_rx.try_recv().expect("the writer got the frame");
        assert_eq!(lit(&frame), vec![5, 6]);
    }

    #[test]
    fn stop_restores() {
        let (gate, devices, t0) = setup();
        gate.start(DeviceId("openrgb:board".into()), 0, 0, 1, t0);
        assert!(gate.stop());
        assert!(!gate.is_active());
        assert_eq!(gate.tick(t0, &devices), Tick::Idle);
        assert!(!gate.stop(), "a second stop finds nothing to end");
    }

    #[tokio::test]
    async fn the_sink_drops_engine_frames_and_the_ticker_frames_still_arrive_when_paused() {
        use crate::state::DeviceSink;
        use canvas_proto::traits::FrameSink;
        let gate = Arc::new(IdentifyGate::default());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let id = DeviceId("openrgb:ram".into());
        let sink = DeviceSink::with_gate(id.clone(), tx, gate.clone());

        // Not active: an engine frame passes.
        sink.submit(vec![Rgb8::new(9, 9, 9); 8]);
        assert_eq!(rx.try_recv().unwrap()[0], Rgb8::new(9, 9, 9));

        // Active: the engine (running or not) is silent; the override frame arrives.
        let devices = vec![device("openrgb:ram", vec![zone(0, 8)])];
        let t0 = Instant::now();
        gate.start(id.clone(), 0, 2, 3, t0);
        sink.submit(vec![Rgb8::new(9, 9, 9); 8]);
        assert!(rx.try_recv().is_err(), "the engine's frame was dropped");
        gate.push(frames_of(gate.tick(t0, &devices)));
        let frame = rx.try_recv().expect("the override frame reached the writer");
        assert_eq!(lit(&frame), vec![2, 3, 4]);

        // Ended: engine frames pass again.
        gate.stop();
        sink.submit(vec![Rgb8::new(9, 9, 9); 8]);
        assert_eq!(rx.try_recv().unwrap()[0], Rgb8::new(9, 9, 9));
    }

    /// A writer that is behind drops the frame, and the log may not say it went out (#87).
    #[test]
    fn a_full_channel_drops_the_frame() {
        let gate = IdentifyGate::default();
        let id = DeviceId("openrgb:ram".into());
        // A channel that holds one frame.
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        gate.register(id.clone(), tx);

        let pushed = gate.push(vec![(id.clone(), vec![Rgb8::WHITE; 8])]);
        assert_eq!(pushed[0].outcome, PushOutcome::Queued);

        // The channel is full now: the frame is dropped and the next tick sends it again.
        let pushed = gate.push(vec![(id.clone(), vec![Rgb8::WHITE; 8])]);
        assert_eq!(pushed[0].outcome, PushOutcome::Dropped(DropReason::Full));
        assert_eq!(pushed[0].device, id);
    }

    /// A chip's LED index inside its zone, its own length and colour: the zone that is open
    /// decides where the run lands, so the request never names a device (#338).
    #[test]
    fn a_setup_lights_each_run_in_its_own_colour_and_blacks_everything_else() {
        let (gate, devices, t0) = setup();
        let red = Rgb8::new(255, 0, 0);
        let green = Rgb8::new(0, 255, 0);
        assert!(gate.start_setup(DeviceId("openrgb:board".into()), 0, "s1".into(), t0).is_some());
        assert!(gate.set_runs(
            "s1",
            vec![
                SetupRun { start: 0, count: 2, color: red },
                SetupRun { start: 4, count: 3, color: green },
            ],
            t0
        ));
        let frames = frames_of(gate.tick(t0, &devices));
        let board = &frames.iter().find(|(id, _)| id.0 == "openrgb:board").unwrap().1;
        let ram = &frames.iter().find(|(id, _)| id.0 == "openrgb:ram").unwrap().1;
        assert_eq!(board.len(), 30);
        assert_eq!(lit(board), vec![0, 1, 4, 5, 6]);
        assert!(board[..2].iter().all(|c| *c == red), "the first run's own colour: {:?}", &board[..2]);
        assert!(board[4..7].iter().all(|c| *c == green), "the second run's own colour: {:?}", &board[4..7]);
        assert!(ram.iter().all(|c| *c == Rgb8::BLACK), "the other devices are black");
    }

    /// The run is an LED index inside the zone: past the zone's end it stops there, and a run
    /// that starts past the end lights nothing rather than reaching another zone.
    #[test]
    fn runs_are_clamped_to_the_zone() {
        let (gate, devices, t0) = setup();
        let red = Rgb8::new(255, 0, 0);
        gate.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t0);
        assert!(gate.set_runs("s", vec![SetupRun { start: 8, count: 5, color: red }], t0));
        let board = frames_of(gate.tick(t0, &devices)).remove(0).1;
        assert_eq!(lit(&board), vec![8, 9], "LEDs 8..13 of a 10-LED zone stop at its end");
        assert!(gate.set_runs("s", vec![SetupRun { start: 12, count: 2, color: red }], t0));
        let board = frames_of(gate.tick(t0, &devices)).remove(0).1;
        assert!(lit(&board).is_empty(), "a run past the zone lights nothing: {:?}", lit(&board));
    }

    /// The session id is what one setup uses to find itself: another id changes nothing, so a
    /// window that was closed and reopened cannot light over the setup that replaced it.
    #[test]
    fn a_stale_session_id_changes_nothing() {
        let (gate, devices, t0) = setup();
        let red = Rgb8::new(255, 0, 0);
        gate.start_setup(DeviceId("openrgb:board".into()), 0, "current".into(), t0);
        assert!(!gate.set_runs("stale", vec![SetupRun { start: 0, count: 2, color: red }], t0));
        assert!(!gate.end_setup("stale"), "a stale session ends nothing");
        assert_eq!(gate.setup_session().as_deref(), Some("current"));
        let board = frames_of(gate.tick(t0, &devices)).remove(0).1;
        assert!(lit(&board).is_empty(), "the stale request lit nothing");

        // No setup at all: a light request has nothing to light.
        let (closed, _, t1) = setup();
        assert!(!closed.set_runs("current", vec![SetupRun { start: 0, count: 1, color: red }], t1));
        assert!(closed.setup_session().is_none());
        assert!(!closed.end_setup("current"));
    }

    /// The setup ends by itself when the last request is older than the spine's timeout, and
    /// the end is what the ticker turns into a restore.
    #[test]
    fn the_setup_times_out_after_the_last_light_request() {
        let (gate, devices, t0) = setup();
        let red = Rgb8::new(255, 0, 0);
        gate.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t0);
        gate.set_runs("s", vec![SetupRun { start: 0, count: 2, color: red }], t0);
        assert!(gate.is_active());
        assert!(matches!(
            gate.tick(t0 + Duration::from_millis(SETUP_TIMEOUT_MS - 1), &devices),
            Tick::Frames(_)
        ));
        assert_eq!(gate.tick(t0 + Duration::from_millis(SETUP_TIMEOUT_MS), &devices), Tick::SetupExpired);
        assert!(!gate.is_active(), "engine frames flow again");
        assert_eq!(gate.tick(t0 + Duration::from_millis(SETUP_TIMEOUT_MS), &devices), Tick::Idle);
    }

    /// Every request renews the timeout, so a window that keeps asking keeps the setup open.
    #[test]
    fn every_light_request_renews_the_timeout() {
        let (gate, devices, t0) = setup();
        let red = Rgb8::new(255, 0, 0);
        let runs = vec![SetupRun { start: 0, count: 2, color: red }];
        gate.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t0);
        assert!(gate.set_runs("s", runs, t0 + Duration::from_secs(5)));
        assert!(
            matches!(gate.tick(t0 + Duration::from_secs(6), &devices), Tick::Frames(_)),
            "the last request was 1 s ago, not 6"
        );
        assert_eq!(gate.tick(t0 + Duration::from_secs(11), &devices), Tick::SetupExpired);
    }

    /// The expiry is asked for once, and it clears the setup: the ticker restores on it.
    #[test]
    fn session_expiry_ends_the_hold_and_asks_for_the_restore() {
        let (gate, devices, t0) = setup();
        gate.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t0);
        gate.set_runs("s", vec![SetupRun { start: 0, count: 2, color: Rgb8::WHITE }], t0);
        assert!(gate.holds_lights());
        assert_eq!(gate.tick(t0 + Duration::from_millis(SETUP_TIMEOUT_MS), &devices), Tick::SetupExpired);
        assert!(!gate.holds_lights());
        assert!(!gate.is_active());
        assert!(gate.setup_session().is_none(), "the setup is cleared");
        assert_eq!(gate.tick(t0 + Duration::from_millis(SETUP_TIMEOUT_MS), &devices), Tick::Idle);

        // A setup takes the lights from an identify that was showing, so nothing outlives it.
        let (both, devices, t1) = setup();
        assert!(both.start(DeviceId("openrgb:board".into()), 0, 0, 2, t1));
        assert_eq!(both.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t1), Some(false));
        assert_eq!(
            both.tick(t1 + Duration::from_millis(SETUP_TIMEOUT_MS), &devices),
            Tick::SetupExpired
        );
        assert!(!both.is_active(), "the identify ended when the setup took the lights");
    }

    /// One setup at a time: a second `start` for another zone (or device) is refused, and the
    /// open one keeps its session.
    #[test]
    fn a_second_start_for_another_zone_is_refused_while_one_is_open() {
        let (gate, _, t0) = setup();
        let board = DeviceId("openrgb:board".into());
        assert!(gate.start_setup(board.clone(), 0, "first".into(), t0).is_some());
        assert!(gate.start_setup(board.clone(), 1, "second".into(), t0).is_none(), "another zone is refused");
        assert!(
            gate.start_setup(DeviceId("openrgb:ram".into()), 0, "third".into(), t0).is_none(),
            "another device is refused too"
        );
        assert_eq!(gate.setup_session().as_deref(), Some("first"));
        assert!(gate.end_setup("first"));
        assert!(gate.start_setup(board, 1, "second".into(), t0).is_some(), "the zone is free once the setup ended");
        assert_eq!(gate.setup_session().as_deref(), Some("second"));
    }

    /// A Scenes flash while the owner is setting a chain on: the setup's own colours stay, and
    /// the flash does not light over them.
    #[test]
    fn a_flash_during_a_setup_does_not_light_over_it() {
        let (gate, devices, t0) = setup();
        let red = Rgb8::new(255, 0, 0);
        gate.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t0);
        gate.set_runs("s", vec![SetupRun { start: 0, count: 2, color: red }], t0);
        gate.start(DeviceId("openrgb:ram".into()), 0, 0, 8, t0);
        let frames = frames_of(gate.tick(t0, &devices));
        let board = &frames.iter().find(|(id, _)| id.0 == "openrgb:board").unwrap().1;
        let ram = &frames.iter().find(|(id, _)| id.0 == "openrgb:ram").unwrap().1;
        assert_eq!(lit(board), vec![0, 1]);
        assert!(board[..2].iter().all(|c| *c == red), "the setup's colour stays: {:?}", &board[..2]);
        assert!(ram.iter().all(|c| *c == Rgb8::BLACK), "the flash lights nothing over the setup");
    }

    /// A flash asked for during a setup is refused: it would outlive the setup.
    #[test]
    fn a_flash_cannot_start_while_a_setup_is_open() {
        let (gate, devices, t0) = setup();
        assert_eq!(gate.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t0), Some(true));
        assert!(!gate.start(DeviceId("openrgb:ram".into()), 0, 0, 8, t0), "no second ticker, no target");
        assert_eq!(
            gate.tick(t0 + Duration::from_millis(SETUP_TIMEOUT_MS), &devices),
            Tick::SetupExpired,
            "nothing the flash asked for outlives the setup"
        );
        assert!(!gate.is_active());
    }

    /// Opening a setup while the identify ticker runs asks for no second ticker.
    #[test]
    fn a_setup_opened_while_the_ticker_runs_asks_for_no_second_one() {
        let (gate, _, t0) = setup();
        assert!(gate.start(DeviceId("openrgb:board".into()), 0, 0, 2, t0), "the identify starts the ticker");
        assert_eq!(gate.start_setup(DeviceId("openrgb:board".into()), 0, "s".into(), t0), Some(false));
    }

    /// A writer that is gone drops the frame, and its sink is removed (#87): the device is
    /// gone, so the next tick must not keep asking for it.
    #[test]
    fn a_closed_channel_drops_the_frame_and_removes_the_sink() {
        let gate = IdentifyGate::default();
        let id = DeviceId("openrgb:ram".into());
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        gate.register(id.clone(), tx);
        drop(rx);

        let pushed = gate.push(vec![(id.clone(), vec![Rgb8::WHITE; 8])]);
        assert_eq!(pushed[0].outcome, PushOutcome::Dropped(DropReason::Closed));

        // The sink is gone: the next frame finds no writer at all.
        let pushed = gate.push(vec![(id.clone(), vec![Rgb8::WHITE; 8])]);
        assert_eq!(pushed[0].outcome, PushOutcome::Dropped(DropReason::NoSink));
    }
}
