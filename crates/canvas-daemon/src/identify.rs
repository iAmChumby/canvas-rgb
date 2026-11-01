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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canvas_proto::api::Power;
use canvas_proto::{DeviceId, DeviceInfo, Rgb8};

use crate::state::App;

/// How long after the last request the override ends by itself. The web resends every ~2 s.
pub const WATCHDOG: Duration = Duration::from_secs(4);
/// How often the ticker re-sends the override frames (the device may drop one).
pub const TICK: Duration = Duration::from_millis(100);
/// The colour of the target run.
pub const TARGET: Rgb8 = Rgb8::WHITE;
/// How long after the last `active: true` an onboarding session ends by itself.
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The id prefix of devices whose adapter lights the whole device (the ROYUAN keyboard).
const WHOLE_DEVICE_PREFIX: &str = "royuan:";

#[derive(Debug, Clone)]
struct Target {
    device: DeviceId,
    zone: u32,
    led: u32,
    count: u32,
    last_request: Instant,
}

#[derive(Default)]
struct Inner {
    target: Option<Target>,
    /// The last `active: true` of an onboarding session; `Some` while it holds the lights.
    session_last: Option<Instant>,
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
        inner.target.is_some() || inner.session_last.is_some()
    }

    /// True while an onboarding session holds the lights.
    pub fn session_active(&self) -> bool {
        self.lock().session_last.is_some()
    }

    /// Starts or renews the onboarding session. Returns `(was_new, start_ticker)`.
    pub fn start_session(&self, now: Instant) -> (bool, bool) {
        let mut inner = self.lock();
        let was_new = inner.session_last.is_none();
        inner.session_last = Some(now);
        let start = !inner.ticker_running;
        inner.ticker_running = true;
        (was_new, start)
    }

    /// Ends the onboarding session. Returns true when one was active.
    pub fn end_session(&self) -> bool {
        self.lock().session_last.take().is_some()
    }

    /// Registers a device's writer channel so the ticker can reach it.
    pub fn register(&self, device: DeviceId, tx: tokio::sync::mpsc::Sender<Vec<Rgb8>>) {
        self.lock().sinks.insert(device, tx);
    }

    /// Sets (or replaces) the target. Returns true when the caller must start the ticker.
    pub fn start(&self, device: DeviceId, zone: u32, led: u32, count: u32, now: Instant) -> bool {
        let mut inner = self.lock();
        inner.target = Some(Target { device, zone, led, count: count.max(1), last_request: now });
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
        if inner.target.is_none() && inner.session_last.is_none() {
            inner.ticker_running = false;
            return Tick::Idle;
        }
        if inner.target.as_ref().is_some_and(|t| now.saturating_duration_since(t.last_request) >= WATCHDOG) {
            inner.target = None;
        }
        if inner.session_last.is_some_and(|l| now.saturating_duration_since(l) >= SESSION_TIMEOUT) {
            inner.session_last = None;
        }
        if inner.target.is_none() && inner.session_last.is_none() {
            inner.ticker_running = false;
            return Tick::Expired;
        }
        Tick::Frames(frames(inner.target.as_ref(), devices))
    }

    /// Sends frames to the registered writers. A full channel drops the frame; the next
    /// tick sends it again.
    ///
    /// Returns what happened to every frame (#81): a frame that the writer's channel did not
    /// take was not sent, and the log may not say it was.
    pub fn push(&self, frames: &[(DeviceId, Vec<Rgb8>)]) -> Vec<(DeviceId, PushOutcome)> {
        let mut inner = self.lock();
        let mut outcomes = Vec::with_capacity(frames.len());
        for (device, colors) in frames {
            let mut closed = false;
            let outcome = match inner.sinks.get(device) {
                Some(tx) => match tx.try_send(colors.clone()) {
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
                inner.sinks.remove(device);
            }
            outcomes.push((device.clone(), outcome));
        }
        outcomes
    }
}

/// Every device's frame under this target: black everywhere but the target run.
fn frames(target: Option<&Target>, devices: &[DeviceInfo]) -> Vec<(DeviceId, Vec<Rgb8>)> {
    devices
        .iter()
        .filter(|d| d.connected)
        .map(|d| {
            let mut colors = vec![Rgb8::BLACK; d.led_count() as usize];
            if let Some(target) = target.filter(|t| d.id == t.device) {
                if d.id.0.starts_with(WHOLE_DEVICE_PREFIX) {
                    colors.fill(TARGET);
                } else if let Some(run) = run_of(d, target) {
                    colors[run].fill(TARGET);
                }
            }
            (d.id.clone(), colors)
        })
        .collect()
}

/// The range of the device's frame the target lights, clamped to its zone.
fn run_of(device: &DeviceInfo, target: &Target) -> Option<std::ops::Range<usize>> {
    let mut offset = 0u32;
    for zone in &device.zones {
        if zone.index == target.zone {
            let start = target.led.min(zone.led_count);
            let end = target.led.saturating_add(target.count).min(zone.led_count);
            return (start < end).then(|| (offset + start) as usize..(offset + end) as usize);
        }
        offset += zone.led_count;
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
/// per request (the page resends about every 2 s), not per tick.
pub fn log_sent(device: &DeviceInfo, frames: &[(DeviceId, Vec<Rgb8>)], outcomes: &[(DeviceId, PushOutcome)]) {
    let Some((_, frame)) = frames.iter().find(|(id, _)| id == &device.id) else {
        tracing::warn!(device = %device.id, "identify: no frame was made for {}; it is not connected", device.id);
        return;
    };
    let outcome = outcomes.iter().find(|(id, _)| id == &device.id).map(|(_, outcome)| *outcome);
    match outcome {
        Some(PushOutcome::Queued) => {
            let lit: Vec<usize> = frame.iter().enumerate().filter(|(_, c)| **c != Rgb8::BLACK).map(|(i, _)| i).collect();
            let others = queued_others(device, outcomes);
            let (first, count) = (lit.first().copied(), lit.len());
            let run = match first {
                Some(first) => format!("{count} of {} LEDs white from LED {first}", frame.len()),
                None => format!("no LED white of {}", frame.len()),
            };
            tracing::info!(
                device = %device.id, adapter = %device.adapter, controller = device.adapter_index,
                name = %device.name, location = %device.location, leds = frame.len(), lit = count, others,
                "identify queued for {} controller {} ({}; {}, {}): {run}; {others} other devices get an all-black frame",
                device.adapter, device.adapter_index, device.id, device.name, device.location
            );
        }
        Some(PushOutcome::Dropped(reason)) => {
            tracing::info!(
                device = %device.id, adapter = %device.adapter, controller = device.adapter_index,
                name = %device.name, location = %device.location, reason = %reason.describe(),
                "identify dropped for {} controller {} ({}; {}, {}): {}; nothing was sent",
                device.adapter, device.adapter_index, device.id, device.name, device.location, reason.describe()
            );
        }
        None => {
            tracing::warn!(
                device = %device.id,
                "identify: the frame for {} was not pushed; nothing was sent",
                device.id
            );
        }
    }
}

/// How many of the other devices' frames were queued for their writers: only those really got
/// an all-black frame.
fn queued_others(device: &DeviceInfo, outcomes: &[(DeviceId, PushOutcome)]) -> usize {
    outcomes
        .iter()
        .filter(|(id, outcome)| id != &device.id && *outcome == PushOutcome::Queued)
        .count()
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
                    app.identify.push(&frames);
                }
                Tick::Expired => {
                    tracing::info!("identify or onboarding session ended by timeout (identify {WATCHDOG:?}, session {SESSION_TIMEOUT:?})");
                    restore(&app).await;
                    return;
                }
            }
        }
    });
}

/// Puts the lights back after the override: the engine resumes by itself when running; a
/// paused output needs its one black frame again.
pub async fn restore(app: &App) {
    let power = app.store.read().await.config.settings.power;
    if power == Power::Paused {
        app.engine.write_solid(Rgb8::BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::{DeviceKind, ZoneInfo, ZoneKind};

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
    fn a_session_makes_every_device_black() {
        let (gate, devices, t0) = setup();
        assert_eq!(gate.start_session(t0), (true, true));
        assert!(gate.is_active());
        let frames = frames_of(gate.tick(t0, &devices));
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|(_, f)| f.iter().all(|c| *c == Rgb8::BLACK)));
    }

    #[test]
    fn an_identify_during_a_session_lights_only_its_target() {
        let (gate, devices, t0) = setup();
        gate.start_session(t0);
        gate.start(DeviceId("openrgb:board".into()), 1, 3, 4, t0);
        let frames = frames_of(gate.tick(t0, &devices));
        let board = &frames.iter().find(|(id, _)| id.0 == "openrgb:board").unwrap().1;
        let ram = &frames.iter().find(|(id, _)| id.0 == "openrgb:ram").unwrap().1;
        assert_eq!(lit(board), vec![13, 14, 15, 16]);
        assert!(ram.iter().all(|c| *c == Rgb8::BLACK));
        // The identify ends by its watchdog; the renewed session still holds the lights black.
        let later = t0 + Duration::from_secs(4);
        gate.start_session(later);
        let frames = frames_of(gate.tick(later, &devices));
        assert!(frames.iter().all(|(_, f)| lit(f).is_empty()));
        assert!(gate.session_active());
    }

    #[test]
    fn the_session_times_out_and_can_be_ended() {
        let (gate, devices, t0) = setup();
        gate.start_session(t0);
        assert!(matches!(gate.tick(t0 + Duration::from_millis(4900), &devices), Tick::Frames(_)));
        assert_eq!(gate.tick(t0 + SESSION_TIMEOUT, &devices), Tick::Expired);
        assert!(!gate.is_active());
        gate.start_session(t0);
        assert!(gate.end_session());
        assert!(!gate.end_session());
        assert_eq!(gate.tick(t0, &devices), Tick::Idle);
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
        let outcomes = gate.push(&frames);
        let (_, text) = crate::test_logs::captured(|| log_sent(&devices[1], &frames, &outcomes));
        assert!(text.contains("INFO"), "{text}");
        assert!(text.contains("identify dropped for openrgb controller 4 (openrgb:ram; openrgb:ram, SMBus 0x71)"), "{text}");
        assert!(text.contains("the device has no writer"), "{text}");
        assert!(text.contains("nothing was sent"), "{text}");
        assert!(!text.contains("identify queued"), "nothing was queued: {text}");

        // A device with no frame (not connected) is a warning, not a silent line.
        let (_, text) = crate::test_logs::captured(|| log_sent(&devices[1], &[], &outcomes));
        assert!(text.contains("WARN") && text.contains("no frame was made for openrgb:ram"), "{text}");
    }

    /// An identify that the writer took is logged as queued, with the controller and the lit run
    /// (#81).
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
        let outcomes = gate.push(&frames);
        assert!(outcomes.iter().all(|(_, outcome)| *outcome == PushOutcome::Queued), "{outcomes:?}");
        let (_, text) = crate::test_logs::captured(|| log_sent(&devices[1], &frames, &outcomes));
        assert!(text.contains("INFO"), "{text}");
        assert!(text.contains("identify queued for openrgb controller 4 (openrgb:ram; openrgb:ram, SMBus 0x71)"), "{text}");
        assert!(text.contains("2 of 8 LEDs white from LED 5"), "{text}");
        assert!(text.contains("1 other devices get an all-black frame"), "{text}");
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
        gate.push(&frames_of(gate.tick(t0, &devices)));
        let frame = rx.try_recv().expect("the override frame reached the writer");
        assert_eq!(lit(&frame), vec![2, 3, 4]);

        // Ended: engine frames pass again.
        gate.stop();
        sink.submit(vec![Rgb8::new(9, 9, 9); 8]);
        assert_eq!(rx.try_recv().unwrap()[0], Rgb8::new(9, 9, 9));
    }
}
