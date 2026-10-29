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
        self.lock().target.is_some()
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
        let Some(target) = inner.target.clone() else {
            inner.ticker_running = false;
            return Tick::Idle;
        };
        if now.saturating_duration_since(target.last_request) >= WATCHDOG {
            inner.target = None;
            inner.ticker_running = false;
            return Tick::Expired;
        }
        Tick::Frames(frames(&target, devices))
    }

    /// Sends frames to the registered writers. A full channel drops the frame; the next
    /// tick sends it again.
    pub fn push(&self, frames: Vec<(DeviceId, Vec<Rgb8>)>) {
        let mut inner = self.lock();
        for (device, colors) in frames {
            let closed = match inner.sinks.get(&device) {
                Some(tx) => matches!(
                    tx.try_send(colors),
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                ),
                None => false,
            };
            if closed {
                inner.sinks.remove(&device);
            }
        }
    }
}

/// Every device's frame under this target: black everywhere but the target run.
fn frames(target: &Target, devices: &[DeviceInfo]) -> Vec<(DeviceId, Vec<Rgb8>)> {
    devices
        .iter()
        .filter(|d| d.connected)
        .map(|d| {
            let mut colors = vec![Rgb8::BLACK; d.led_count() as usize];
            if d.id == target.device {
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

/// Starts the ticker task for the app.
pub fn spawn_ticker(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let devices = app.devices.read().await.clone();
            match app.identify.tick(Instant::now(), &devices) {
                Tick::Idle => return,
                Tick::Frames(frames) => app.identify.push(frames),
                Tick::Expired => {
                    tracing::info!("identify ended: no request for {WATCHDOG:?}");
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
}
