//! The OpenRGB supervisor and the rescans the owner asks for (#233).
//!
//! The supervisor is the one task that reads the device list. A rescan is a request it answers
//! with the list it read, so a rescan that arrives while another one waits must not drop the
//! first, and an owner that gave up must not make the supervisor work for nobody.

use canvas_daemon::openrgb_supervisor::{supervise_with_rescans, Rescan};
use canvas_proto::device::{AdapterCapabilities, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::traits::{AdapterError, DeviceAdapter};
use canvas_proto::DeviceId;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How the adapter's connect behaves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Connect {
    /// The link comes up.
    Works,
    /// The link never answers: the connect waits on the gate and then fails.
    Waits,
}

/// What the supervisor asked the adapter for.
#[derive(Default)]
struct Counts {
    connects: AtomicUsize,
    discoveries: AtomicUsize,
    /// How many times the supervisor asked whether the link is up: once per rescan and once
    /// per tick.
    checks: AtomicUsize,
    /// How many times the supervisor dropped the link.
    disconnects: AtomicUsize,
    connected: AtomicBool,
}

impl Counts {
    fn connects(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }

    fn discoveries(&self) -> usize {
        self.discoveries.load(Ordering::SeqCst)
    }

    fn disconnects(&self) -> usize {
        self.disconnects.load(Ordering::SeqCst)
    }
}

/// A gate the test opens and shuts: a call waits on it while it is shut.
struct Gate {
    open: AtomicBool,
    notify: tokio::sync::Notify,
}

impl Gate {
    fn new(open: bool) -> Arc<Self> {
        Arc::new(Self { open: AtomicBool::new(open), notify: tokio::sync::Notify::new() })
    }

    /// Waits until the gate is open.
    async fn wait(&self) {
        loop {
            if self.open.load(Ordering::SeqCst) {
                return;
            }
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Registered before the check, so an opening that lands in between still wakes it.
            notified.as_mut().enable();
            if self.open.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    fn open(&self) {
        self.open.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn shut(&self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

/// The adapter the supervisor talks to. It counts what the supervisor asked for, and the test
/// holds a call on a gate when it needs to.
struct TestAdapter {
    devices: Vec<DeviceInfo>,
    counts: Arc<Counts>,
    connect: Connect,
    /// The gate the first connect waits on.
    first_gate: Option<Arc<Gate>>,
    /// The gate the later connects wait on.
    gate: Option<Arc<Gate>>,
    discover_gate: Option<Arc<Gate>>,
}

impl TestAdapter {
    fn stick(serial: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(format!("openrgb:{serial}")),
            name: format!("DDR5 {serial}"),
            vendor: "fake".into(),
            kind: DeviceKind::Dram,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: serial.into(),
            location: "smbus".into(),
            zones: vec![ZoneInfo {
                index: 0,
                name: "RGB".into(),
                kind: ZoneKind::Linear,
                led_count: 8,
                min_leds: 8,
                max_leds: 8,
                resizable: false,
            }],
            connected: true,
        }
    }
}

#[async_trait::async_trait]
impl DeviceAdapter for TestAdapter {
    fn name(&self) -> &'static str {
        "test"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            resizable_zones: true,
            identify: true,
            hardware_brightness: false,
            needs_elevation: false,
        }
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        let attempt = self.counts.connects.fetch_add(1, Ordering::SeqCst);
        if self.connect == Connect::Waits {
            let gate = if attempt == 0 { self.first_gate.as_ref() } else { self.gate.as_ref() };
            if let Some(gate) = gate {
                gate.wait().await;
            }
            // The link never answers: the attempt fails when the gate opens.
            self.counts.connected.store(false, Ordering::SeqCst);
            return Err(AdapterError::NotConnected);
        }
        self.counts.connected.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        self.counts.disconnects.fetch_add(1, Ordering::SeqCst);
        self.counts.connected.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn is_connected(&self) -> bool {
        // The link is up for the first ask and gone after it: the supervisor must notice that
        // on its tick.
        self.counts.checks.fetch_add(1, Ordering::SeqCst) == 0
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        self.counts.discoveries.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.discover_gate {
            gate.wait().await;
        }
        Ok(self.devices.clone())
    }

    async fn write_frame(
        &mut self,
        _device: &DeviceId,
        _colors: &[canvas_proto::Rgb8],
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn set_zone_size(
        &mut self,
        _device: &DeviceId,
        _zone: u32,
        _leds: u32,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn identify(
        &mut self,
        _device: &DeviceId,
        _zone: u32,
        _led: u32,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn read_back(&mut self, _device: &DeviceId) -> Result<Vec<canvas_proto::Rgb8>, AdapterError> {
        Ok(Vec::new())
    }
}

/// The supervisor, its rescan inbox and the counters the test reads.
struct Supervisor {
    rescans: tokio::sync::mpsc::Sender<Rescan>,
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    /// How many times the supervisor said it was connecting.
    connecting: Arc<AtomicUsize>,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.task.abort();
    }
}

/// Starts the supervisor over `adapter`, with a rescan inbox the test holds.
fn supervisor(adapter: TestAdapter) -> Supervisor {
    let adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>> =
        Arc::new(tokio::sync::Mutex::new(Box::new(adapter)));
    let zones = Arc::new(tokio::sync::RwLock::new(std::collections::BTreeMap::new()));
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let connecting = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&connecting);
    let task = tokio::spawn(supervise_with_rescans(
        adapter,
        zones,
        |_devices| async {},
        move |link| {
            if link == canvas_proto::api::LinkState::Connecting {
                seen.fetch_add(1, Ordering::SeqCst);
            }
            async {}
        },
        Some(rx),
        stop_rx,
    ));
    Supervisor { rescans: tx, stop, task, connecting }
}

/// Asks for a rescan and hands back the channel its answer comes on.
async fn rescan(
    supervisor: &Supervisor,
    budget: Duration,
) -> tokio::sync::oneshot::Receiver<Result<Vec<DeviceInfo>, String>> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    let request = Rescan { budget, reply };
    supervisor.rescans.send(request).await.expect("the supervisor takes the rescan");
    answer
}

/// Waits until `ready` is true. The wait is on the thing the assert reads; the long limit only
/// ends a test that would hang.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

/// Waits until the connect count stops moving: the count is what the assert reads, and the quiet
/// period is how the test knows the supervisor has settled.
///
/// The period is longer than the backoff the supervisor waits between its attempts, so an
/// attempt that comes late is still seen.
async fn settle(counts: &Counts) {
    let mut last = counts.connects();
    let mut quiet = Duration::ZERO;
    while quiet < Duration::from_secs(2) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let now = counts.connects();
        if now == last {
            quiet += Duration::from_millis(50);
        } else {
            quiet = Duration::ZERO;
            last = now;
        }
    }
}

/// Two rescans that arrive together share one attempt, and both get its answer (#233).
///
/// The first rescan is on its way when the second arrives. Both must be answered, and one
/// discovery answers them both: the second must not wait for a turn of its own, or its answer
/// comes after the caller's deadline.
#[tokio::test]
async fn two_rescans_that_arrive_together_share_one_discovery_and_both_get_its_answer() {
    let counts = Arc::new(Counts::default());
    let discover_gate = Gate::new(true);
    let adapter = TestAdapter {
        devices: vec![TestAdapter::stick("one")],
        counts: Arc::clone(&counts),
        connect: Connect::Works,
        first_gate: None,
        gate: None,
        discover_gate: Some(Arc::clone(&discover_gate)),
    };
    let supervisor = supervisor(adapter);

    // The first attempt reads the devices.
    until("the first discovery", || counts.discoveries() == 1).await;
    until("the link is up", || counts.connected.load(Ordering::SeqCst)).await;

    // The next discovery waits: the first rescan's attempt is in flight.
    discover_gate.shut();
    let budget = Duration::from_secs(5);
    let first = rescan(&supervisor, budget).await;
    until("the rescan's discovery is waiting", || counts.discoveries() == 2).await;
    // The second rescan arrives while the first one waits.
    let second = rescan(&supervisor, budget).await;

    // The discovery answers.
    discover_gate.open();
    let answered = tokio::time::timeout(Duration::from_secs(30), async {
        let a = first.await.expect("the first rescan is answered");
        let b = second.await.expect("the second rescan is answered");
        (a, b)
    })
    .await
    .expect("both rescans are answered");
    assert!(answered.0.is_ok() && answered.1.is_ok(), "both got the list: {answered:?}");

    // One discovery answered them both: the second did not wait for a turn of its own.
    assert_eq!(counts.discoveries(), 2, "the two rescans shared one discovery");
}

/// An abandoned rescan starts no attempt on the link-down path (#233).
///
/// The owner gave up on the rescan (the browser went away), so the supervisor must not take the
/// adapter for it: the link is down, and every attempt is the supervisor's own.
#[tokio::test]
async fn an_abandoned_rescan_on_the_link_down_path_starts_no_attempt() {
    let counts = Arc::new(Counts::default());
    let first_gate = Gate::new(false);
    // The later attempts wait on a gate the test never opens: they wait for good.
    let later_gate = Gate::new(false);
    let adapter = TestAdapter {
        devices: vec![],
        counts: Arc::clone(&counts),
        connect: Connect::Waits,
        first_gate: Some(Arc::clone(&first_gate)),
        gate: Some(Arc::clone(&later_gate)),
        discover_gate: None,
    };
    let supervisor = supervisor(adapter);

    // The first attempt waits on the gate: the link never answers.
    until("the first attempt is waiting", || counts.connects() == 1).await;
    // The link is down: the attempt fails, and the next attempt waits on the gate the test
    // never opens, so the count only moves when the supervisor tries.
    first_gate.open();

    // A rescan whose owner gave up: the owner dropped the answer channel, so nobody waits for
    // it and the supervisor must not work for it.
    let (reply, answer) = tokio::sync::oneshot::channel::<Result<Vec<DeviceInfo>, String>>();
    drop(answer);
    supervisor
        .rescans
        .send(Rescan { budget: Duration::from_millis(100), reply })
        .await
        .expect("the supervisor takes the rescan");

    settle(&counts).await;
    // The first attempt and the supervisor's own: the abandoned rescan started none.
    assert_eq!(counts.connects(), 2, "the abandoned rescan started no attempt");
}

/// A stream of rescans does not starve the link tick (#233).
///
/// The link check runs on a tick, and the tick is what makes the supervisor notice that OpenRGB
/// went away. A stream of rescans must not push the tick back every time, or the supervisor
/// never checks the link and never refreshes the device list.
///
/// The adapter's link is up for the first ask and gone after it, so the supervisor has to
/// notice it: it says it is connecting again, and the test counts that.
#[tokio::test]
async fn a_stream_of_rescans_does_not_starve_the_link_tick() {
    let counts = Arc::new(Counts::default());
    let adapter = TestAdapter {
        devices: vec![TestAdapter::stick("one")],
        counts: Arc::clone(&counts),
        connect: Connect::Works,
        first_gate: None,
        gate: None,
        discover_gate: None,
    };
    let supervisor = supervisor(adapter);
    until("the first discovery", || counts.discoveries() == 1).await;
    let connects_before = supervisor.connecting.load(Ordering::SeqCst);

    // A stream of rescans, one after the other, while the supervisor works through them. The
    // inbox is never empty: the next rescan goes out before the last one is answered, so the
    // supervisor always has one waiting.
    let budget = Duration::from_secs(5);
    let streaming = tokio::spawn({
        let rescans = supervisor.rescans.clone();
        async move {
            let mut pending = Vec::new();
            for _ in 0..8 {
                let (reply, answer) = tokio::sync::oneshot::channel();
                if rescans.send(Rescan { budget, reply }).await.is_err() {
                    return;
                }
                pending.push(answer);
            }
            while let Some(answer) = pending.pop() {
                // The answer is the event the test waits on, not a clock.
                let _ = answer.await;
                let (reply, next) = tokio::sync::oneshot::channel();
                if rescans.send(Rescan { budget, reply }).await.is_err() {
                    return;
                }
                pending.push(next);
            }
        }
    });

    // The tick fires: the supervisor notices the link is gone and connects again.
    until("the tick fires", || {
        supervisor.connecting.load(Ordering::SeqCst) > connects_before
    })
    .await;
    streaming.abort();
}

/// A rescan that is cut in the middle drops the link (#233, review).
///
/// The budget ran out while the rescan was working: a saved zone size may have been applied
/// and the list not read again. The link is dropped, as on the connect path, so the next
/// attempt reads everything again instead of trusting half-applied state.
#[tokio::test]
async fn a_rescan_that_is_cut_in_the_middle_drops_the_link() {
    let counts = Arc::new(Counts::default());
    let discover_gate = Gate::new(true);
    let adapter = TestAdapter {
        devices: vec![TestAdapter::stick("one")],
        counts: Arc::clone(&counts),
        connect: Connect::Works,
        first_gate: None,
        gate: None,
        discover_gate: Some(Arc::clone(&discover_gate)),
    };
    let supervisor = supervisor(adapter);

    // The first attempt reads the devices.
    until("the first discovery", || counts.discoveries() == 1).await;

    // The next discovery waits: the rescan's budget runs out while it waits.
    discover_gate.shut();
    let rescan = rescan(&supervisor, Duration::from_millis(100)).await;

    // The link is dropped, so the next attempt reads everything again.
    until("the link was dropped", || counts.disconnects() == 1).await;
    let answered = tokio::time::timeout(Duration::from_secs(30), rescan)
        .await
        .expect("the rescan is answered")
        .expect("the answer came");
    assert!(answered.is_err(), "the cut rescan answered with a list: {answered:?}");
}
