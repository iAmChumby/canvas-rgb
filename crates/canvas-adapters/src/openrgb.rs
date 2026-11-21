//! OpenRGB SDK client implementing `DeviceAdapter`.
//!
//! One TCP connection per adapter. A reader task owns the read half, routes replies to the
//! single in-flight request, and marks the session lost on EOF or error, so `is_connected()`
//! turns false as soon as the socket drops. Clones share the connection, so several device
//! tasks can each hold a clone and write concurrently; writes take a short lock on the socket.

use async_trait::async_trait;
use canvas_proto::{
    AdapterCapabilities, AdapterError, DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8,
    ZoneInfo, ZoneKind,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::protocol::*;

/// Most controllers one OpenRGB server may report. The count comes off the wire, so it is
/// capped before it sizes an allocation or a request loop.
const MAX_CONTROLLERS: u32 = 1024;
/// Most LEDs one controller may report. UPDATE_LEDS carries a u16 count, so a larger device
/// could not be written anyway.
const MAX_LEDS_PER_CONTROLLER: u64 = u16::MAX as u64;

/// How long a request waits for its reply before the call fails.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// How many times a protocol 6 read of the list is repeated when the list changes under it.
const ENUMERATE_ATTEMPTS: u32 = 3;

/// How a rescan waits for OpenRGB's detection to be over (#290).
///
/// **Protocol 6 and later** (OpenRGB release_1.1 and newer, what canvas-rgb negotiates with them) needs
/// no window at all: the server says when detection starts (101) and when it is over (103)
/// (`NetworkServer.cpp:4003-4015`, `:3949-3961`), names every controller by an id that is never
/// reused (`:682-711`) and drops a packet for an id it no longer has (`:1495-1531`). While a detection
/// runs the held list stays, a read of the list is answered with it, and frames keep going to the
/// held ids (the server drops them; none can reach another device); the list read after 103 replaces
/// the held one, shorter or longer. Only `cap` applies on that path, as the bound of one call. See
/// [`OpenRgbAdapter::rescan_shared`].
///
/// **Protocol 5** (an older OpenRGB) is what the rest of this comment is about, and what `first` and
/// `settle` are for. A protocol 5 client sees `DEVICE_LIST_UPDATED` (100, ungated, `:4017`): once when the list is cleared
/// (`DetectionManager.cpp:1707`, then `ResourceManager.cpp:62-65` and `:972-1067`) and once as each
/// controller registers again (`DetectionManager.cpp:491`). Nothing says detection is over, and a
/// packet 140 sent while a detection runs is dropped without a word (`DetectionManager.cpp:1753-1756`).
///
/// So the adapter does not guess when detection ends. Every list change (100) puts it in the
/// **unsettled** state, whoever began the detection and whether or not a packet 140 was sent; and
/// while it is unsettled the cached controller list never changes, a list read is answered with the
/// cached one, and no frame is written (protocol 5 names a controller by its position, so a frame
/// sent while the controllers register again can land on another device). The hold begins earlier than
/// the first 100: OpenRGB swaps its new list into the server and sends the 100 only afterwards
/// (`ResourceManager.cpp:1041`, `:1045`, `:1067`), so the adapter holds from the moment it sends packet 140
/// (`Session::rescan_requests`, #509) until a 100 or the end of that call. The list is **settled**
/// once a read finds it non-empty and no list change arrived for `settle` before it began or
/// while it ran; the list read then replaces the cached one, shorter or longer, and the frames
/// resume. A list that stays empty never settles, however long that takes: an empty list is what a
/// detection that has not found its first controller yet looks like (`DetectionManager.cpp:810-856`,
/// the DRAM scan at `:1030-1100`), and with protocol 5 it cannot be told from every device being
/// unplugged. There is no cap on that hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RescanTiming {
    /// Protocol 5 only. How long after packet 140 the first list change may take when OpenRGB's list was not
    /// already changing. Detection starts by clearing the list, which is immediate, so a silence
    /// this long means detection is off or refused: the list was not touched. 1.5 s allows a busy
    /// server to take its time. It decides only what a rescan reports, never what is applied.
    pub first: Duration,
    /// Protocol 5 only. How long the list must have been quiet before a read of it is trusted as the settled list.
    /// A longer gap between two controllers inside one detection only delays the answer: the
    /// next list change makes the list unsettled again, and the list that was cached stays
    /// cached meanwhile. Real timings are UNKNOWN until Luke's hardware check.
    pub settle: Duration,
    /// The longest a rescan call waits for the detection to be over (the list to settle, on protocol 5)
    /// before it answers that detection is still running. This bounds the wait of one call, for the editor's request, and nothing else:
    /// the list stays unsettled past it, the cached list stays as it is, and the settled list is
    /// applied whenever it arrives (the daemon's 5 s refresh finds it). The editor waits 30 s for
    /// the whole API answer (`DEVICE_RESCAN_TIMEOUT_MS`): 15 s here + 5 s re-read + 5 s queue wait
    /// leaves 5 s of margin.
    pub cap: Duration,
}

impl Default for RescanTiming {
    fn default() -> Self {
        Self { first: Duration::from_millis(1500), settle: Duration::from_secs(2), cap: Duration::from_secs(15) }
    }
}

/// How often a rescan wait looks at whether the link is still up.
const LINK_CHECK: Duration = Duration::from_millis(100);

/// The error text of a rescan whose detection never began: no list change followed packet 140.
pub const RESCAN_NOT_REDETECTED_MESSAGE: &str = "OpenRGB did not start detection";

/// The error text of a rescan whose packet 140 OpenRGB never acknowledged (protocol 6): the server is not
/// answering, which is a failure, and nothing is known about a detection.
pub const RESCAN_NOT_ACKNOWLEDGED_MESSAGE: &str = "OpenRGB did not acknowledge the rescan";

/// The error text of a rescan that gave up waiting while OpenRGB's detection was still running.
pub const RESCAN_DETECT_TIMEOUT_MESSAGE: &str = "OpenRGB is still detecting devices";

/// The error text of a request OpenRGB acknowledged without answering it: protocol 6 sends an ACK
/// after every packet, and a request for a controller id the server no longer has gets nothing else
/// (`NetworkServer.cpp:1495-1531`, `:3562-3580`).
pub const CONTROLLER_ID_GONE_MESSAGE: &str = "OpenRGB no longer has that controller id";

/// The error text of a frame that was not written because OpenRGB's list is changing.
pub const LIST_UNSETTLED_MESSAGE: &str = "OpenRGB is re-detecting its devices; no frame is written until its list settles";

/// What a rescan that did not re-detect left behind. See [`rescan_miss`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RescanMiss {
    /// Nothing was re-detected and the list was not touched: the adapter cannot (the trait
    /// default, a protocol too old) or OpenRGB started no detection. Reading the list is safe.
    NotRedetected,
    /// OpenRGB's list is still changing, or is empty, after the wait: detection is still running
    /// (or cannot be told from every device being gone). The adapter keeps the list it holds and
    /// answers a read with it; the settled list arrives later by itself.
    Unsettled,
}

/// Whether a `rescan()` error means "the hardware could not be asked to look again", and which
/// kind. `None` is a real failure.
///
/// The trait error type is in the spine, so the cases that are safe to ignore are named here
/// instead of in a new spine variant. Both leave the link up.
pub fn rescan_miss(error: &AdapterError) -> Option<RescanMiss> {
    match error {
        AdapterError::Unsupported(_) => Some(RescanMiss::NotRedetected),
        AdapterError::Protocol(reason) if reason == RESCAN_NOT_REDETECTED_MESSAGE => Some(RescanMiss::NotRedetected),
        AdapterError::Protocol(reason) if reason == RESCAN_DETECT_TIMEOUT_MESSAGE => Some(RescanMiss::Unsettled),
        _ => None,
    }
}

/// A rescan whose detection went quiet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redetected {
    /// From packet 140 to the list being settled.
    pub settled_after: Duration,
    /// How many list changes (100) arrived after packet 140.
    pub list_changes: u32,
    /// OpenRGB said so itself (101 and 103, protocol 6 and later): the detection is over, and a list
    /// read now, empty or not, is the real one. Without it (protocol 5) the end was inferred from a
    /// quiet list, and an empty list cannot be told from a detection that has found nothing yet.
    pub by_events: bool,
}

/// How often `SET_CUSTOM_MODE` is sent again to a controller that is being streamed frames.
///
/// A starting value, to be measured (#5): the ENE DRAM sticks drop back to their built-in
/// rainbow when a sleep cuts their power, and the resume event may never arrive. The cost of
/// each re-send is one 16-byte packet per controller (a header with no payload), which OpenRGB
/// turns into one SMBus mode write. Check the SMBus load on real hardware before lowering it.
pub const CUSTOM_MODE_KEEPALIVE: Duration = Duration::from_secs(10);

const ZONE_TYPE_SINGLE: i32 = 0;
const ZONE_TYPE_LINEAR: i32 = 1;
const ZONE_TYPE_MATRIX: i32 = 2;

/// The clocks the adapter reads.
///
/// `Instant` is a monotonic clock, and the standard library says that it is "not specified
/// whether system suspends count as elapsed time or not" (on Windows it reads
/// `QueryPerformanceCounter`). The keepalive therefore watches the wall clock as well: the
/// wall clock follows the RTC, so a sleep shows up there whether or not the monotonic clock
/// counted it (#5).
#[derive(Clone)]
struct Clocks {
    monotonic: Arc<dyn Fn() -> Instant + Send + Sync>,
    wall: Arc<dyn Fn() -> SystemTime + Send + Sync>,
}

impl Clocks {
    /// The clocks of the machine.
    fn system() -> Self {
        Self {
            monotonic: Arc::new(Instant::now),
            wall: Arc::new(SystemTime::now),
        }
    }

    /// The clocks of the caller: the tests move them.
    #[cfg(test)]
    fn moved(
        monotonic: Arc<dyn Fn() -> Instant + Send + Sync>,
        wall: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    ) -> Self {
        Self { monotonic, wall }
    }

    /// The monotonic clock.
    fn monotonic(&self) -> Instant {
        (self.monotonic)()
    }

    /// The wall clock.
    fn wall(&self) -> SystemTime {
        (self.wall)()
    }
}

impl Default for Clocks {
    fn default() -> Self {
        Self::system()
    }
}

/// When a controller was last switched to direct/custom mode.
#[derive(Clone, Copy)]
struct ModeStamp {
    /// The monotonic clock.
    at: Instant,
    /// The wall clock.
    wall: SystemTime,
}

#[derive(Clone)]
pub struct OpenRgbAdapter {
    host: String,
    port: u16,
    link: Arc<Link>,
    /// The clocks the keepalive reads.
    clocks: Clocks,
    /// How a rescan waits for detection to settle (#290).
    rescan_timing: RescanTiming,
}

/// A handle to an adapter's reply deadline ([`OpenRgbAdapter::reply_timeout`]). Cloning it shares
/// the deadline with the adapter and all its clones.
#[derive(Clone)]
pub struct ReplyTimeout(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl ReplyTimeout {
    /// Sets the deadline. A test raises this to a hang guard while the fake is live and lowers it
    /// just before it makes the fake go silent (#519).
    pub fn set(&self, deadline: Duration) {
        self.0.store(deadline.as_millis() as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

struct Link {
    session: Mutex<Session>,
    writer: tokio::sync::Mutex<Option<OwnedWriteHalf>>,
    /// Serializes request/reply pairs: at most one reply is ever awaited.
    request_gate: tokio::sync::Mutex<()>,
    /// Set when the link is unusable: a write was cut in the middle (the socket may hold a
    /// part of a packet), or a request went out and was not answered (a late reply would
    /// answer the next request). The next writer drops the socket instead of writing into
    /// the middle of the half-written packet, and the supervisor reconnects.
    broken: AtomicBool,
    /// Bumped by the reader of the current session on every `DEVICE_LIST_UPDATED` (100). A rescan
    /// watches it for the list changes that detection announces (#290).
    list_changed: tokio::sync::watch::Sender<u64>,
    /// How long a request waits for a reply before it breaks the link (#519). Milliseconds, so a
    /// test can raise it to a hang guard while the fake is live and lower it again when it means
    /// the fake to go silent. Production never changes it from [`REPLY_TIMEOUT`].
    reply_timeout_ms: Arc<std::sync::atomic::AtomicU64>,
}

struct Session {
    /// Bumped on every connect and disconnect, so a stale reader cannot mark a newer session lost.
    epoch: u64,
    connected: bool,
    protocol_version: u32,
    pending: Option<Pending>,
    reader: Option<JoinHandle<()>>,
    controllers: Vec<Controller>,
    /// When each controller was last switched to direct/custom mode on this connection.
    /// A missing entry means the next frame sends `SET_CUSTOM_MODE` first.
    custom_mode: HashMap<u32, ModeStamp>,
    /// The controllers `SET_CUSTOM_MODE` was ever sent to on this connection. Only the log reads
    /// it (the first send per connection is INFO, a re-send after a reset says so); it changes
    /// nothing about when the mode is sent.
    mode_sent: HashSet<u32>,
    /// How long a controller stays trusted to be in custom mode before the next frame
    /// sends `SET_CUSTOM_MODE` again.
    keepalive: Duration,
    /// Bumped by every `reset_custom_mode`, so a send in flight can tell it was overtaken.
    resets: u64,
    /// The controllers being resized, and what each resize is doing. A frame for one of them
    /// is skipped until the re-enumerate lands, so no frame of the old zone size reaches
    /// OpenRGB while it is re-configuring the controller.
    resizing: HashMap<u32, Resize>,
    /// How many discoveries are running. A frame that arrives while the list is empty and a
    /// discovery is running is skipped: it would have to wait for the request gate the
    /// discovery holds.
    discovering: u32,
    /// OpenRGB's list is changing (#290): a list change (100) arrived and no read has found the list
    /// settled since. While set, `controllers` is not replaced and no frame is written.
    unsettled: bool,
    /// Protocol 5: how many packets 140 this adapter has sent that no list change (100) has followed yet (#509).
    /// While it is above zero the list may already be swapped in OpenRGB (it swaps the list into its server
    /// and sends the 100 only afterwards, `ResourceManager.cpp:1041`, `:1045`, `:1067`), and a position names
    /// whatever sits there now, so the list is held exactly as when unsettled: no frame, no resize, no
    /// list read replaces the cached list. A 100 hands the hold over to `unsettled` and clears this.
    rescan_requests: u32,
    /// How many list changes (100) this session has seen. A read that began at one count and
    /// ended at another saw a list that changed under it.
    list_changes: u64,
    /// When the last list change arrived.
    last_list_change: Option<tokio::time::Instant>,
    /// The `list_changes` count a settle read last found the list not settled at (empty, or
    /// changed while it was read). A settle is tried again only after another list change.
    settle_failed_at: Option<u64>,
    /// A settle read is running, so no second one starts.
    settling: bool,
    /// Protocol 6: a detection is running (101 seen, 103 not yet; or a 102 below 100 %, which says one is
    /// running that began before this connection; a 102 at 100 % is a hotplug, see [`note_progress`]).
    detecting: bool,
    /// Protocol 6: how many detections this session has seen start, and how many end.
    detections_started: u64,
    detections_completed: u64,
    /// Protocol 6: a read of the list on this connection found OpenRGB's final list: no detection was
    /// running, a 103 had been heard and no list change since it. False from the connect until then, and a
    /// quiet OpenRGB that has said nothing is not final either (a join may be mid-detection: release_1.1
    /// sends a new client no detection state, `NetworkServer.cpp:886` is a broadcast); see
    /// [`OpenRgbAdapter::list_is_final`].
    list_final: bool,
    /// Protocol 6: the `list_changes` count when the last 103 arrived. A 100 after it is a hot plug: the
    /// list changed again and no 103 will say it is over.
    list_changes_at_103: u64,
    /// Protocol 6: a 103 arrived and no read of the list has followed it.
    end_unread: bool,
    /// Protocol 6: a read of the list found no detection running and no list change heard on this
    /// connection. Only the rescan's "did not start" answer uses it (`may_be_mid_detection`).
    quiet_read: bool,
    /// Protocol 6: how many 101 and detection 102 (see [`note_progress`]) this connection has heard.
    /// A detection that began before the connect sends no 101 here: its 102s count from the first one
    /// below 100 % on, and one that sends no progress is not heard at all. A count that grew says a
    /// detection is running.
    detection_signals: u64,
    /// Protocol 6: how many ACKs of packet 140 this session has seen. The server sends the ACK after
    /// the packet is handled, on the same thread and socket as the 101 it caused (`NetworkServer.cpp:1725-1730`,
    /// `DetectionManager.cpp:1776`), so a 101 for a packet 140 always arrives before its ACK.
    rescan_acks: u64,
    /// Protocol 6: for each (controller id, packet id) of a request sent, how many ACKs are still owed.
    /// A reply comes before its ACK, so an ACK that finds its request still waiting says the server
    /// answered with nothing: the controller id is gone.
    owed_acks: HashMap<(u32, u32), u32>,
}

impl Session {
    /// Protocol 6: this connection may be inside a detection it never saw start. It heard the list change
    /// (100), no 103 has come since, and no read found OpenRGB quiet: its packet 140 is dropped by a
    /// running detection (`DetectionManager.cpp:1753-1756`) and answered by an ACK alone, which looks the
    /// same as a server that started nothing. A connection that heard nothing, or that has read a final list,
    /// has no reason to think so.
    fn may_be_mid_detection(&self) -> bool {
        self.detections_completed == 0 && self.list_changes > 0 && !self.quiet_read
    }
}

/// What a controller's resize is doing.
struct Resize {
    /// Bumped on every resize of this controller. A frame that was built for the list before
    /// the resize is stale, and it may not stamp the mode back on.
    epoch: u64,
    /// A resize is in flight: the controller is being re-configured right now.
    in_flight: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            epoch: 0,
            connected: false,
            protocol_version: 0,
            pending: None,
            reader: None,
            controllers: Vec::new(),
            custom_mode: HashMap::new(),
            mode_sent: HashSet::new(),
            keepalive: CUSTOM_MODE_KEEPALIVE,
            resets: 0,
            resizing: HashMap::new(),
            discovering: 0,
            unsettled: false,
            rescan_requests: 0,
            list_changes: 0,
            last_list_change: None,
            settle_failed_at: None,
            settling: false,
            detecting: false,
            detections_started: 0,
            detections_completed: 0,
            list_final: false,
            list_changes_at_103: 0,
            end_unread: false,
            quiet_read: false,
            detection_signals: 0,
            rescan_acks: 0,
            owed_acks: HashMap::new(),
        }
    }
}

struct Pending {
    device: u32,
    packet_id: u32,
    reply: oneshot::Sender<Reply>,
}

/// What ends a request.
enum Reply {
    /// The server's answer.
    Data(Vec<u8>),
    /// The server acknowledged the request and answered nothing (protocol 6): the controller it names
    /// is gone.
    Refused,
}

/// The reply slot of one request, cleared however the request ends: a reply, an error, a
/// timeout, or its future being dropped mid-wait.
struct InFlight<'a> {
    link: &'a Link,
    /// The request went out, so the server may still answer it.
    sent: bool,
    /// The (controller id, packet id) an ACK was counted as owed for.
    owed: Option<(u32, u32)>,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut session = guard(&self.link.session);
        if let (false, Some(key)) = (self.sent, self.owed) {
            // The packet never went out, so no ACK is owed for it.
            if let Some(count) = session.owed_acks.get_mut(&key) {
                *count = count.saturating_sub(1);
            }
        }
        // A slot the reader already took means the reply arrived: nothing is owed.
        let unanswered = session.pending.take().is_some();
        if !self.sent || !unanswered || !session.connected {
            return;
        }
        // The request went out and nobody answered it. OpenRGB answers in order and puts no
        // request id in a reply, so the reply may still come and would answer the next
        // request. The link is unusable: it is broken and lost, and the supervisor
        // reconnects. A fresh connection has no stale replies.
        drop(session);
        self.link.break_link();
    }
}

/// The write that is on the socket, watched so a write that is given up on leaves the link
/// broken.
///
/// The write holds the socket lock across `write_all`, so dropping it (a caller that gave up,
/// as the rescan budget does) can leave a part of a packet on the socket. The next writer
/// would then write into the middle of it, and OpenRGB would read two packets out of one.
/// The link is broken instead: the socket is dropped and the supervisor reconnects.
///
/// The guard owns the socket lock, so the link is broken *before* the lock is released: a
/// writer that waits for the lock cannot take it, see a whole link, and write into the middle
/// of the half-written packet.
struct SendGuard<'a> {
    link: &'a Link,
    /// The socket lock, held for the write.
    writer: tokio::sync::MutexGuard<'a, Option<OwnedWriteHalf>>,
    /// The write may have put a part of the packet on the socket.
    armed: bool,
    /// The write finished (or failed): the link is whole.
    done: bool,
}

impl Drop for SendGuard<'_> {
    fn drop(&mut self) {
        if !self.armed || self.done {
            return;
        }
        // The write was cut in the middle. The socket may hold a part of a packet: it is
        // dropped, and the link is marked lost, so no other writer writes into the middle
        // of it. This happens while the socket lock is still held (the guard owns it).
        self.link.broken.store(true, Ordering::SeqCst);
        *self.writer = None;
        self.link.lose_current();
    }
}

/// A resize that is running: the frames for the controller are skipped until it ends.
///
/// The guard ends the resize however the resize ends. A handler that was cancelled (the
/// client went away in the middle of the PATCH) must not leave the controller's frames
/// skipped for the rest of the connection.
struct ResizeGuard<'a> {
    link: &'a Link,
    index: u32,
}

impl Drop for ResizeGuard<'_> {
    fn drop(&mut self) {
        let mut session = guard(&self.link.session);
        if let Some(resize) = session.resizing.get_mut(&self.index) {
            resize.in_flight = false;
        }
    }
}

/// A discovery that is running: a frame that arrives while the device list is empty is
/// skipped rather than waiting for the request gate the discovery holds.
struct Discovery<'a> {
    link: &'a Link,
}

impl Drop for Discovery<'_> {
    fn drop(&mut self) {
        let mut session = guard(&self.link.session);
        session.discovering = session.discovering.saturating_sub(1);
    }
}

#[derive(Debug, Clone)]
struct Controller {
    /// The position in the server's list.
    index: u32,
    /// What a packet for this controller names it by: its id on protocol 6 (a number the server never
    /// reuses, so a packet for a controller that was re-detected reaches nobody), its position
    /// before (`NetworkServer.cpp:1500-1512`).
    addr: u32,
    wire: WireController,
}

impl Controller {
    fn led_count(&self) -> u32 {
        self.wire.zones.iter().map(|zone| zone.leds_count).sum()
    }

    fn zone_offset(&self, zone: usize) -> usize {
        self.wire.zones.iter().take(zone).map(|z| z.leds_count as usize).sum()
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether no list change arrived for `settle` before now.
fn list_is_quiet(session: &Session, settle: Duration) -> bool {
    session.last_list_change.is_none_or(|at| tokio::time::Instant::now().saturating_duration_since(at) >= settle)
}

/// A settle read that is running: clears the flag however the read ends.
struct Settling<'a> {
    link: &'a Link,
}

impl Drop for Settling<'_> {
    fn drop(&mut self) {
        guard(&self.link.session).settling = false;
    }
}

/// The hold a protocol 5 rescan puts on the list from the moment it sends packet 140 (#509): released
/// however the call ends, unless a list change (100) already took it over.
struct RescanHold<'a> {
    link: &'a Link,
    /// The session the hold was taken in: a reconnect starts a session with no hold.
    epoch: u64,
    /// The list change count when it was taken. A 100 after that has cleared every hold taken before it.
    changes: u64,
    /// The hold has been given back already (by [`release`](Self::release)).
    released: bool,
}

impl RescanHold<'_> {
    /// Gives the hold back, once: a 100 that came since has taken it over already and is left alone.
    fn release(&mut self) {
        if std::mem::replace(&mut self.released, true) {
            return;
        }
        let mut session = guard(&self.link.session);
        if session.epoch == self.epoch && session.list_changes == self.changes {
            session.rescan_requests = session.rescan_requests.saturating_sub(1);
        }
    }
}

impl Drop for RescanHold<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

impl Link {
    /// Ends the session `epoch` if it is still the current one; aborts its reader.
    fn lose(&self, epoch: u64) {
        let reader = {
            let mut session = guard(&self.session);
            if session.epoch != epoch {
                return;
            }
            session.connected = false;
            session.pending = None;
            session.reader.take()
        };
        if let Some(reader) = reader {
            reader.abort();
        }
        // A rescan that is waiting looks at the link again.
        self.list_changed.send_modify(|n| *n += 1);
    }

    fn reset_custom_mode(&self) {
        let mut session = guard(&self.session);
        session.custom_mode.clear();
        session.resets += 1;
    }

    fn lose_current(&self) {
        let epoch = guard(&self.session).epoch;
        self.lose(epoch);
    }

    /// The link is unusable: it is marked broken and lost, so no writer writes into it and
    /// the supervisor reconnects.
    ///
    /// This is what an unanswered request does. OpenRGB answers in order and puts no request
    /// id in a reply, so a reply that arrives late would answer the next request. A fresh
    /// connection has no stale replies, so the link is dropped instead of being patched up.
    fn break_link(&self) {
        self.broken.store(true, Ordering::SeqCst);
        self.lose_current();
    }
}

/// Every live adapter's link, so a resume event (which the daemon hears far from the adapter,
/// behind `Box<dyn DeviceAdapter>`) can reach all of them without a spine change.
static LINKS: Mutex<Vec<Weak<Link>>> = Mutex::new(Vec::new());

/// The system resumed from sleep: every controller may have lost power and fallen back to its
/// built-in mode, so the next frame to each one sends `SET_CUSTOM_MODE` again. Returns how many
/// adapters were told.
pub fn reset_custom_mode_all() -> usize {
    let mut links = guard(&LINKS);
    links.retain(|link| link.strong_count() > 0);
    links.iter().filter_map(Weak::upgrade).map(|link| link.reset_custom_mode()).count()
}

impl OpenRgbAdapter {
    pub fn new(host: &str, port: u16) -> Self {
        let link = Arc::new(Link {
            session: Mutex::new(Session::default()),
            writer: tokio::sync::Mutex::new(None),
            request_gate: tokio::sync::Mutex::new(()),
            broken: AtomicBool::new(false),
            list_changed: tokio::sync::watch::channel(0_u64).0,
            reply_timeout_ms: Arc::new(std::sync::atomic::AtomicU64::new(REPLY_TIMEOUT.as_millis() as u64)),
        });
        {
            let mut links = guard(&LINKS);
            links.retain(|link| link.strong_count() > 0);
            links.push(Arc::downgrade(&link));
        }
        Self {
            host: host.to_string(),
            port,
            link,
            clocks: Clocks::system(),
            rescan_timing: RescanTiming::default(),
        }
    }

    /// Like [`new`](Self::new) with another way to wait for a rescan's detection to settle
    /// (tests use short windows). Clones made afterwards, such as the frame lane, share it.
    pub fn with_rescan_timing(mut self, timing: RescanTiming) -> Self {
        self.rescan_timing = timing;
        self
    }

    /// Like [`new`](Self::new) with another deadline for a controller's reply. The production
    /// default is [`REPLY_TIMEOUT`] (5 s); tests against the fake raise it so the verdict never
    /// depends on how fast the machine schedules the fake's reply (#519). This is a hang guard,
    /// not the thing a test waits for: a test waits on the fake's own event instead.
    pub fn with_reply_timeout(self, reply_timeout: Duration) -> Self {
        self.set_reply_timeout(reply_timeout);
        self
    }

    /// Changes the reply deadline on this adapter and every clone of it. A test raises it while
    /// the fake is live, and lowers it again just before it means the fake to go silent (#519).
    pub fn set_reply_timeout(&self, reply_timeout: Duration) {
        self.link
            .reply_timeout_ms
            .store(reply_timeout.as_millis() as u64, std::sync::atomic::Ordering::Relaxed);
    }

    /// A small handle to the reply deadline, for a test whose adapter is held behind a trait
    /// object: it can still lower the deadline at the moment it means the fake to go silent.
    pub fn reply_timeout(&self) -> ReplyTimeout {
        ReplyTimeout(Arc::clone(&self.link.reply_timeout_ms))
    }

    /// Whether the list this connection reads is OpenRGB's final one rather than a part of a detection's
    /// (#290). Protocol 6 and later only; an older OpenRGB has no events, so there is nothing to wait for
    /// and this is `true`.
    ///
    /// release_1.1 sends a client that connects no detection state (`NetworkServer.cpp:886` is a broadcast
    /// to the clients there at the time; the server starts before detection, `ResourceManager.cpp:1362`
    /// then `:1377`), and signals progress per detector, not per controller (`DetectionManager.cpp:491`,
    /// `:720`, `:1009`). A connection that heard nothing therefore cannot tell a quiet OpenRGB from one
    /// that is mid-detection, and its first read may be empty or a part list. So the list is final only when
    /// a read after a 103 found no detection running and no list change has been heard since that 103. It is
    /// `false` from the connect until then, and again after a hot plug (a 100 with no 103 after it).
    ///
    /// The daemon's rule rests on this and does not wait for it: a list that adds devices applies at once, one
    /// that drops a device applies when this is `true`, when the owner's rescan returned it, or after a quiet
    /// period (`UNPLUG_QUIET` in the daemon's supervisor), so a missing 103 never freezes the list.
    pub fn list_is_final(&self) -> bool {
        let session = guard(&self.link.session);
        // Protocol 5 hears no detection events, so its list is never final by an event: a shrink there
        // waits for the owner's rescan or the daemon's quiet period, like a protocol-6 hot plug (#290
        // review 7: calling it final let a detection gap shrink the daemon's list 3 -> 1 -> 2).
        session.protocol_version >= PROTOCOL_WITH_EVENTS
            && session.list_final
            && !session.detecting
            && session.list_changes == session.list_changes_at_103
    }

    /// Protocol 6 and later: a detection ended (103) and no read of the list has followed it. The daemon
    /// reads now, so the final list is applied without waiting for its 5 s refresh.
    pub fn detection_end_unread(&self) -> bool {
        let session = guard(&self.link.session);
        session.end_unread && !session.detecting
    }

    /// Protocol 6 and later: how many detection starts (101) and detection progress reports (102 inside a
    /// detection, or below 100 %; a hotplug's 102 at 100 % is not one, #534) this connection has heard. A
    /// count that did not move between two reads says no detection began or progressed between them.
    pub fn detection_signals(&self) -> u64 {
        guard(&self.link.session).detection_signals
    }

    /// Protocol 6 and later: how many ACKs of packet 140 this connection has heard. A test waits on it
    /// to know that the server has answered the rescan packet.
    pub fn rescan_acks(&self) -> u64 {
        guard(&self.link.session).rescan_acks
    }

    /// How a rescan on this adapter waits for detection to settle.
    pub fn rescan_timing(&self) -> RescanTiming {
        self.rescan_timing
    }

    /// Like [`new`](Self::new) with another keepalive interval (tests use a short one).
    pub fn with_keepalive(host: &str, port: u16, keepalive: Duration) -> Self {
        let adapter = Self::new(host, port);
        guard(&adapter.link.session).keepalive = keepalive;
        adapter
    }

    /// Like [`new`](Self::new) with another keepalive interval and clocks of the caller's.
    ///
    /// The tests move the clocks: a sleep that the monotonic clock did not count is a jump
    /// of the wall clock alone (#5).
    #[cfg(test)]
    fn with_clocks(host: &str, port: u16, keepalive: Duration, clocks: Clocks) -> Self {
        let mut adapter = Self::new(host, port);
        guard(&adapter.link.session).keepalive = keepalive;
        adapter.clocks = clocks;
        adapter
    }

    /// Forgets which controllers are in custom mode: the next frame to each sends
    /// `SET_CUSTOM_MODE` first. Called after a resume.
    pub fn reset_custom_mode(&self) {
        self.link.reset_custom_mode();
    }

    /// Writes one frame through the shared link, with no `&mut` on the adapter.
    ///
    /// A frame is `SET_CUSTOM_MODE` and `UPDATE_LEDS`: neither has a reply, and neither is ever
    /// awaited, so a frame cannot be taken for the reply to a request. Each packet is one write
    /// under the socket lock, so a frame lands cleanly between the packets of a request that is
    /// waiting on OpenRGB. The daemon's frame writers go through here (a clone of the adapter
    /// shares the link), so a discovery that is stalled on a reply does not hold a frame back.
    pub async fn write_frame_shared(&self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
        // The device list is empty and a discovery is running: the controller lookup below
        // would read the list again, and that takes the request gate the discovery holds. A
        // frame is cheap to lose and the discovery is about to fill the list, so the frame is
        // skipped instead of waiting behind it.
        if self.list_is_empty() && self.discovery_running() {
            return Err(AdapterError::Protocol("the device list is being read".into()));
        }
        let controller = self.controller(device).await?;
        let led_count = controller.led_count() as usize;
        if colors.len() != led_count {
            return Err(AdapterError::Protocol(format!(
                "frame has {} colors but {device} has {led_count} LEDs", colors.len())));
        }
        self.send_leds(controller.addr, colors).await
    }

    /// Asks OpenRGB to look at the hardware again, then waits for its list to settle (#290), with no
    /// `&mut` on the adapter.
    ///
    /// The wait takes seconds, so the daemon runs it on the frame lane, a clone that shares this
    /// link, and never under its adapter lock: that lock is what the keyboard's frame writer and
    /// the API handlers wait on. [`DeviceAdapter::rescan`] is the same call for a caller that has
    /// the adapter to itself.
    ///
    /// Packet 140 is client-only and gets no reply: it goes out through [`send`](Self::send),
    /// never through `request`, which would wait five seconds for a reply that never comes and
    /// break the link. It may also be dropped without a word, when a detection is already running
    /// (`DetectionManager.cpp:1753-1756`); the wait does not care which, because it watches
    /// OpenRGB's list and not the packet. Four ends:
    ///
    /// - the list is settled and some list change came after packet 140 (or the list was already
    ///   changing and has settled): `Ok`. The adapter holds the settled list;
    /// - the list was not changing and no change came within `first`: [`RESCAN_NOT_REDETECTED_MESSAGE`],
    ///   and the list was never touched;
    /// - the list was already changing when packet 140 went out and no 100 came within `first`: the
    ///   hold ends there (OpenRGB sends its 100 inside its packet-140 handler, so none means it
    ///   started no detection), the settle read reads the list again, and the answer is
    ///   [`RESCAN_NOT_REDETECTED_MESSAGE`]; `try_settle` refuses to settle while the hold is on, so
    ///   the wait never spins (#509, second review);
    /// - the list is still changing, or empty, when `cap` runs out: [`RESCAN_DETECT_TIMEOUT_MESSAGE`].
    ///   Detection is still running. The adapter goes on holding the list it has, and answers a read
    ///   with it, until a read finds the new one settled: no wait here decides that, so a detection
    ///   of any length wipes nothing. The link stays up in every case.
    ///
    /// A protocol 4 server has no such packet and is refused as unsupported.
    ///
    /// **Protocol 6 and later** is event-driven and uses none of the above (see
    /// [`rescan_by_events`](Self::rescan_by_events)): the wait ends on OpenRGB's own 103, no timer decides
    /// what the adapter holds, and the list is read again by the next [`enumerate`](Self::enumerate).
    pub async fn rescan_shared(&self) -> Result<Redetected, AdapterError> {
        use tokio::time::{sleep_until, Instant};
        self.require_connected()?;
        let version = self.protocol_version();
        if version < 5 {
            return Err(AdapterError::Unsupported("OpenRGB protocol 4 cannot re-detect devices"));
        }
        if version >= PROTOCOL_WITH_EVENTS {
            return self.rescan_by_events().await;
        }
        let timing = self.rescan_timing;
        // Subscribe before writing and mark the current count seen, so only a change that arrives
        // after packet 140 wakes the wait.
        let mut changes = self.link.list_changed.subscribe();
        let _ = changes.borrow_and_update();
        let started = Instant::now();
        let cap = started + timing.cap;
        // Held before packet 140 goes out, not when the 100 comes back: OpenRGB swaps its list in first and
        // tells clients after (#509), and a frame by position sent in between lands on another device.
        let mut hold = self.hold_for_rescan();
        let changes_at_start = hold.changes;
        self.send(0, REQUEST_RESCAN_DEVICES, &[]).await?;
        loop {
            self.require_connected()?;
            let (unsettled, count, last, failed_at) = {
                let session = guard(&self.link.session);
                (session.unsettled, session.list_changes, session.last_list_change, session.settle_failed_at)
            };
            let now = Instant::now();
            // No 100 within `first` of packet 140: detection starts by clearing the list at once, so OpenRGB
            // started none and never swapped its list. The hold ends there, before the settle read below may
            // run: a read is trusted only when no swap can be in flight (see `try_settle`).
            if count == changes_at_start && now >= started + timing.first {
                hold.release();
            }
            let wake = if !unsettled {
                if count != changes_at_start {
                    // A detection ran, or one already running went quiet, and its list is the one held.
                    let seen = u32::try_from(count - changes_at_start).unwrap_or(u32::MAX);
                    return Ok(Redetected { settled_after: started.elapsed(), list_changes: seen, by_events: false });
                }
                // Nothing came after packet 140: detection starts by clearing the list, at once.
                if now >= started + timing.first {
                    return Err(AdapterError::Protocol(RESCAN_NOT_REDETECTED_MESSAGE.into()));
                }
                started + timing.first
            } else {
                if now >= cap {
                    return Err(AdapterError::Protocol(RESCAN_DETECT_TIMEOUT_MESSAGE.into()));
                }
                let quiet_at = last.map_or(now, |at| at + timing.settle);
                if now < quiet_at {
                    quiet_at
                } else if failed_at == Some(count) {
                    // Read since the last change and not settled: only a change moves it.
                    cap
                } else if self.try_settle().await? {
                    continue;
                } else {
                    // Another task is reading the list (a frame that found it quiet): look again
                    // when it is done.
                    now + LINK_CHECK
                }
            };
            tokio::select! {
                changed = changes.changed() => {
                    changed.map_err(|_| AdapterError::NotConnected)?;
                }
                // Also wakes to look at the link: the wait goes on.
                () = sleep_until(wake.min(cap).min(Instant::now() + LINK_CHECK)) => {}
            }
        }
    }

    /// Holds the list from now until a list change (100) or the end of the rescan call, whichever is first
    /// (see `Session::rescan_requests`).
    fn hold_for_rescan(&self) -> RescanHold<'_> {
        let mut session = guard(&self.link.session);
        session.rescan_requests += 1;
        RescanHold { link: &self.link, epoch: session.epoch, changes: session.list_changes, released: false }
    }

    /// A rescan on protocol 6 and later: OpenRGB says when detection starts (101) and ends (103), so
    /// nothing here is a guess (#290).
    ///
    /// Packet 140 goes out and the call waits for the server's ACK of it. The server handles the packet
    /// on one thread, which sends the 101 it causes before the ACK, on the same socket
    /// (`NetworkServer.cpp:1725-1730`, `DetectionManager.cpp:1753-1776`); so when the ACK arrives the
    /// 101 is already counted, and one of three things is true:
    ///
    /// - a detection runs (this packet began it, or it was already running and dropped the packet,
    ///   `DetectionManager.cpp:1753-1756`: the call joins the running one): wait for its 103, then `Ok`;
    /// - a detection ran and ended meanwhile: `Ok` at once;
    /// - none ran: the server refused or has detection off, [`RESCAN_NOT_REDETECTED_MESSAGE`], the list
    ///   was never touched. Except on a connection that may have joined a detection already running (it
    ///   heard list changes, no 103, and has read no final list; see `Session::may_be_mid_detection`): it
    ///   never saw the 101 that detection began with, the packet was dropped, and the ACK looks the same.
    ///   That call waits for the 103 like the first case, and at `cap` answers "still running".
    ///
    /// `cap` only bounds the call: a detection still running at it answers
    /// [`RESCAN_DETECT_TIMEOUT_MESSAGE`], and the held list stays whatever the detection takes. The
    /// adapter's cached list is not touched here: a read ([`enumerate`](Self::enumerate)) answers with
    /// the held list while the detection runs and reads the new one once it is over.
    async fn rescan_by_events(&self) -> Result<Redetected, AdapterError> {
        use tokio::time::{sleep_until, Instant};
        let mut changes = self.link.list_changed.subscribe();
        let _ = changes.borrow_and_update();
        let started = Instant::now();
        let cap = started + self.rescan_timing.cap;
        let before = {
            let session = guard(&self.link.session);
            (session.detections_started, session.detections_completed, session.rescan_acks, session.list_changes)
        };
        self.send(0, REQUEST_RESCAN_DEVICES, &[]).await?;
        loop {
            self.require_connected()?;
            let (detecting, began, ended, acks, list_changes, joined) = {
                let session = guard(&self.link.session);
                (
                    session.detecting,
                    session.detections_started,
                    session.detections_completed,
                    session.rescan_acks,
                    session.list_changes,
                    session.may_be_mid_detection(),
                )
            };
            if acks != before.2 {
                // No 101 came for this packet. A connection that may have joined a running detection (it
                // heard list changes and no 103) was answered by an ACK alone because OpenRGB dropped the
                // packet: it waits for the 103 like one that saw the 101, up to the cap, and is told "still
                // running" there. Any other connection is told nothing started.
                let nothing_began = began == before.0 && ended == before.1;
                if !detecting && nothing_began && !joined {
                    return Err(AdapterError::Protocol(RESCAN_NOT_REDETECTED_MESSAGE.into()));
                }
                if !detecting && !(nothing_began && joined) {
                    let seen = u32::try_from(list_changes - before.3).unwrap_or(u32::MAX);
                    return Ok(Redetected { settled_after: started.elapsed(), list_changes: seen, by_events: true });
                }
            }
            tokio::select! {
                changed = changes.changed() => {
                    changed.map_err(|_| AdapterError::NotConnected)?;
                }
                () = sleep_until(cap) => {
                    // A detection that runs past the bound is still running. A server that never
                    // acknowledged the packet is not answering at all: that is a failure, not a detection.
                    let acknowledged = guard(&self.link.session).rescan_acks != before.2;
                    return Err(AdapterError::Protocol(
                        if acknowledged { RESCAN_DETECT_TIMEOUT_MESSAGE } else { RESCAN_NOT_ACKNOWLEDGED_MESSAGE }.into(),
                    ));
                }
            }
        }
    }

    /// Reads the list now if it is unsettled and has been quiet for the settle window, once: the
    /// read replaces the cached list when it finds the list settled (see [`RescanTiming`]) and
    /// changes nothing otherwise. Reads nothing (and returns `false`) while another settle read
    /// runs, or after one that found the list not settled and no list change since.
    async fn try_settle(&self) -> Result<bool, AdapterError> {
        {
            let mut session = guard(&self.link.session);
            // Not while a rescan was requested and OpenRGB has said nothing: its list may be swapped already, a
            // read by position can mix the old list and the new, and no 100 would say so (#509). `enumerate()`
            // answers the held list then, which is not a settled one: claiming `true` here sent the rescan's
            // wait round its loop again with no await in it.
            if !session.unsettled
                || session.settling
                || session.rescan_requests > 0
                || session.settle_failed_at == Some(session.list_changes)
            {
                return Ok(false);
            }
            if !list_is_quiet(&session, self.rescan_timing.settle) {
                return Ok(false);
            }
            session.settling = true;
        }
        // Declared after the flag is set: however the read ends, even if this future is dropped,
        // the flag is cleared.
        let _settling = Settling { link: &self.link };
        self.enumerate().await.map(|_| true)
    }

    /// Fails while OpenRGB's list is changing and a list is cached: an index-addressed packet
    /// (a frame, a resize, an identify) could reach another device than the one it names. Tries
    /// to settle the list first, so the lights resume as soon as the list has been quiet.
    async fn ensure_settled(&self) -> Result<(), AdapterError> {
        if !self.list_is_unsettled() {
            return Ok(());
        }
        self.try_settle().await?;
        if self.list_is_unsettled() {
            return Err(AdapterError::Protocol(LIST_UNSETTLED_MESSAGE.into()));
        }
        Ok(())
    }

    /// Whether what an index in OpenRGB's list names is not to be trusted: the list is changing while
    /// this adapter holds one, or a rescan was requested and OpenRGB has not said anything yet. The hold
    /// counts with an empty list too (#520): OpenRGB may have swapped its list already, and a read by
    /// position, or a frame built from one, would mix the old list and the new. An empty list that is
    /// changing with no hold is a start-up that has found nothing yet, and a read of it is the list.
    fn list_is_unsettled(&self) -> bool {
        let session = guard(&self.link.session);
        session.rescan_requests > 0 || (session.unsettled && !session.controllers.is_empty())
    }

    fn require_connected(&self) -> Result<(), AdapterError> {
        if guard(&self.link.session).connected { Ok(()) } else { Err(AdapterError::NotConnected) }
    }

    /// Whether the controller list of this connection is empty.
    fn list_is_empty(&self) -> bool {
        guard(&self.link.session).controllers.is_empty()
    }

    /// Whether a discovery is running on this link.
    fn discovery_running(&self) -> bool {
        guard(&self.link.session).discovering > 0
    }

    /// Looks up a controller by its daemon-facing id. Enumerates once when this connection has
    /// not discovered anything yet, so a caller that skipped `discover()` still works.
    async fn controller(&self, device: &DeviceId) -> Result<Controller, AdapterError> {
        self.ensure_settled().await?;
        let (known, empty) = {
            let session = guard(&self.link.session);
            if !session.connected {
                return Err(AdapterError::NotConnected);
            }
            let known = session.controllers.iter()
                .find(|controller| &device_info(controller).id == device)
                .cloned();
            (known, session.controllers.is_empty())
        };
        if let Some(controller) = known {
            return Ok(controller);
        }
        if empty {
            self.enumerate().await?;
            return self.controller_cached(device);
        }
        Err(AdapterError::UnknownDevice(device.clone()))
    }

    fn controller_cached(&self, device: &DeviceId) -> Result<Controller, AdapterError> {
        guard(&self.link.session).controllers.iter()
            .find(|controller| &device_info(controller).id == device)
            .cloned()
            .ok_or_else(|| AdapterError::UnknownDevice(device.clone()))
    }

    /// Sends a request packet and waits for nothing.
    async fn send(&self, device: u32, packet_id: u32, payload: &[u8]) -> Result<(), AdapterError> {
        let bytes = encode_packet(device, packet_id, payload)?;
        self.write_packet(device, &bytes, false, false).await
    }

    /// Sends a packet that names a controller by its position (protocol 5): a zone resize, or the request of
    /// a controller's colours.
    /// Refused while the list is unsettled, checked under the socket lock like a frame (see
    /// [`write_packet`](Self::write_packet)).
    async fn send_positional(&self, device: u32, packet_id: u32, payload: &[u8]) -> Result<(), AdapterError> {
        let bytes = encode_packet(device, packet_id, payload)?;
        self.write_packet(device, &bytes, false, true).await
    }

    /// Sends a frame packet (`SET_CUSTOM_MODE`, `UPDATE_LEDS`).
    ///
    /// A controller that is being resized is skipped: the frame was built for the zone sizes
    /// the controller had before the resize, and OpenRGB is re-configuring it right now. The
    /// check is under the socket lock, so it cannot slip between the resize and the frame.
    async fn send_frame(&self, device: u32, packet_id: u32, payload: &[u8]) -> Result<(), AdapterError> {
        let bytes = encode_packet(device, packet_id, payload)?;
        self.write_packet(device, &bytes, true, true).await
    }

    /// Writes one packet on the socket, under the socket lock.
    ///
    /// The lock is held across the write, so the packets keep their order. A write that is
    /// given up on (the future is dropped) can leave a part of a packet on the socket: the
    /// [`SendGuard`] then breaks the link, so the next writer does not write into the middle
    /// of it.
    ///
    /// A packet that `positional` marks names a controller by its position on protocol 5, so it may not
    /// go out while OpenRGB's list is unsettled or a rescan was requested and not heard from (#509). That
    /// is checked here, under the socket lock, and not only before the caller picked the controller: the
    /// rescan takes its hold before it writes packet 140 on this lock, so a packet that passes the check
    /// is written before packet 140 and reaches the list as it was, and every packet after is refused.
    async fn write_packet(&self, device: u32, bytes: &[u8], frame: bool, positional: bool) -> Result<(), AdapterError> {
        let mut writer = self.link.writer.lock().await;
        // A write that was cut in the middle left the socket with a part of a packet on it:
        // the socket is dropped instead of carrying on.
        if self.link.broken.swap(false, Ordering::SeqCst) {
            *writer = None;
        }
        if positional && self.list_is_unsettled() {
            return Err(AdapterError::Protocol(LIST_UNSETTLED_MESSAGE.into()));
        }
        if frame && self.resize_in_flight(device) {
            return Err(AdapterError::Protocol(format!(
                "the OpenRGB controller {device} is being resized")));
        }
        if writer.is_none() {
            return Err(AdapterError::NotConnected);
        }
        // The guard owns the socket lock: a write that is given up on breaks the link before
        // the lock is released, so a writer that waits for the lock cannot slip in between
        // and write into the middle of the half-written packet.
        let mut guard = SendGuard { link: &self.link, writer, armed: false, done: false };
        // Armed before the write: a write that is given up on from here on may have put a
        // part of the packet on the socket.
        guard.armed = true;
        let written = async {
            match guard.writer.as_mut() {
                Some(stream) => {
                    stream.write_all(bytes).await?;
                    stream.flush().await
                }
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "the OpenRGB socket is gone",
                )),
            }
        }
        .await;
        guard.done = true;
        match written {
            Ok(()) => Ok(()),
            Err(error) => {
                *guard.writer = None;
                self.link.lose_current();
                Err(AdapterError::Io(error))
            }
        }
    }

    /// Sends a request and waits for the reply with the same device index and packet id.
    ///
    /// Safe to cancel (a caller may give up on it, as the rescan budget does): dropping the
    /// future clears the reply slot, and a request that went out and was not answered loses
    /// the link.
    ///
    /// A request whose reply never comes leaves the link unusable: OpenRGB answers in order
    /// and puts no request id in a reply, so the reply may still come and would answer the
    /// next request. The link is broken and lost, and the supervisor reconnects: a fresh
    /// connection has no stale replies.
    async fn request(&self, device: u32, packet_id: u32, payload: &[u8]) -> Result<Vec<u8>, AdapterError> {
        self.request_as(device, packet_id, payload, false).await
    }

    /// A [`request`](Self::request) that names a controller by its position on protocol 5 and reports what
    /// that controller holds (`read_back`): refused under the socket lock while the list is unsettled or a
    /// rescan was requested, like a frame (see [`write_packet`](Self::write_packet), #509). The list read
    /// of [`enumerate`](Self::enumerate) is not one: it is what settles the list, and it names
    /// every controller in turn on purpose.
    async fn request_positional(&self, device: u32, packet_id: u32, payload: &[u8]) -> Result<Vec<u8>, AdapterError> {
        self.request_as(device, packet_id, payload, true).await
    }

    async fn request_as(&self, device: u32, packet_id: u32, payload: &[u8], positional: bool) -> Result<Vec<u8>, AdapterError> {
        let _gate = self.link.request_gate.lock().await;
        let (reply, received) = oneshot::channel();
        // Declared after the gate, so it drops first: the slot is cleared while the gate is held.
        let mut in_flight = InFlight { link: &self.link, sent: false, owed: None };
        {
            let mut session = guard(&self.link.session);
            session.pending = Some(Pending { device, packet_id, reply });
            // Protocol 6 acknowledges every packet after answering it. Counted before the packet goes
            // out, so the ACK cannot arrive first.
            if session.protocol_version >= PROTOCOL_WITH_EVENTS {
                *session.owed_acks.entry((device, packet_id)).or_insert(0) += 1;
                in_flight.owed = Some((device, packet_id));
            }
        }
        // Checked after the reply slot is set: a reader that ended first has already cleared
        // the session, so the request fails here instead of waiting out the timeout.
        if !guard(&self.link.session).connected {
            return Err(AdapterError::NotConnected);
        }
        if positional {
            self.send_positional(device, packet_id, payload).await?;
        } else {
            self.send(device, packet_id, payload).await?;
        }
        in_flight.sent = true;
        let reply_timeout = Duration::from_millis(
            self.link.reply_timeout_ms.load(std::sync::atomic::Ordering::Relaxed),
        );
        match tokio::time::timeout(reply_timeout, received).await {
            Ok(Ok(Reply::Data(reply))) => Ok(reply),
            // Acknowledged and not answered: nothing is owed, the link is whole.
            Ok(Ok(Reply::Refused)) => Err(AdapterError::Protocol(CONTROLLER_ID_GONE_MESSAGE.into())),
            Ok(Err(_)) => Err(AdapterError::NotConnected),
            Err(_) => {
                // The reply never came: the link is unusable, and the supervisor reconnects.
                self.link.break_link();
                Err(AdapterError::Protocol(format!(
                    "no reply to packet {packet_id} for device {device} within {}s",
                    reply_timeout.as_secs())))
            }
        }
    }

    fn protocol_version(&self) -> u32 {
        guard(&self.link.session).protocol_version
    }

    /// Re-enumerates every controller. OpenRGB reports zone resizes through this same path.
    ///
    /// Returns the list the adapter holds afterwards. Which list that is follows the settle rule
    /// (see [`RescanTiming`], #290): while OpenRGB's list is changing, the list read replaces the
    /// cached one only when it is non-empty, no list change arrived for the settle window before
    /// it began, and none arrived while it ran; every other read leaves the cache as it is and
    /// answers with it. A list read while it is still changing is not even requested.
    async fn enumerate(&self) -> Result<Vec<Controller>, AdapterError> {
        self.require_connected()?;
        if self.protocol_version() >= PROTOCOL_WITH_EVENTS {
            return self.enumerate_by_id().await;
        }
        let settle = self.rescan_timing.settle;
        let changes_before = {
            let session = guard(&self.link.session);
            if session.unsettled && !session.controllers.is_empty() && !list_is_quiet(&session, settle) {
                return Ok(session.controllers.clone());
            }
            // A rescan was requested and OpenRGB has not said anything yet: its list may be swapped already.
            // An empty list is held too: it answers as it is, and nothing is read through the hold (#520).
            if session.rescan_requests > 0 {
                return Ok(session.controllers.clone());
            }
            session.list_changes
        };
        let version = self.protocol_version();
        {
            let mut session = guard(&self.link.session);
            session.discovering += 1;
        }
        let _discovery = Discovery { link: &self.link };
        let count_reply = self.request(0, REQUEST_CONTROLLER_COUNT, &[]).await?;
        let count = read_u32(&count_reply, "controller count")?;
        if count > MAX_CONTROLLERS {
            return Err(AdapterError::Protocol(format!(
                "OpenRGB reports {count} controllers; the limit is {MAX_CONTROLLERS}")));
        }
        let mut controllers = Vec::with_capacity(count as usize);
        for index in 0..count {
            let payload = self.request(index, REQUEST_CONTROLLER_DATA, &version.to_le_bytes()).await?;
            let wire = WireController::decode(&payload, version)?;
            let leds: u64 = wire.zones.iter().map(|zone| u64::from(zone.leds_count)).sum();
            if leds > MAX_LEDS_PER_CONTROLLER {
                // One absurd controller must not hide the others: skip it.
                tracing::warn!(index, leds, "skipping an OpenRGB controller over the LED limit of {MAX_LEDS_PER_CONTROLLER}");
                continue;
            }
            controllers.push(Controller { index, addr: index, wire });
        }
        let mut session = guard(&self.link.session);
        if !session.connected {
            return Ok(controllers);
        }
        let changed_during = session.list_changes != changes_before;
        if session.controllers.is_empty() {
            // Nothing is held, so nothing is at risk: the list read is the list. It is settled when
            // it is not empty and the list was quiet.
            session.controllers = controllers.clone();
            if session.unsettled && !controllers.is_empty() && !changed_during && list_is_quiet(&session, settle) {
                Self::settled(&mut session);
            }
            return Ok(controllers);
        }
        if controllers.is_empty() {
            // An empty list never replaces a held one: it is the list of a detection that has not
            // found its first controller yet, and protocol 5 cannot tell it from every device
            // being gone. The list is changing, whether or not a list change said so.
            session.unsettled = true;
            session.settle_failed_at = Some(session.list_changes);
            tracing::debug!(held = session.controllers.len(), "OpenRGB reports no controllers; keeping the list held");
            return Ok(session.controllers.clone());
        }
        if session.unsettled {
            if changed_during || !list_is_quiet(&session, settle) {
                session.settle_failed_at = Some(session.list_changes);
                tracing::debug!(
                    read = controllers.len(),
                    held = session.controllers.len(),
                    "OpenRGB's list changed while it was read; keeping the list held"
                );
                return Ok(session.controllers.clone());
            }
            if session.controllers.len() != controllers.len() {
                tracing::info!(held = session.controllers.len(), settled = controllers.len(), "OpenRGB's list settled with another count of devices");
            }
            session.controllers = controllers.clone();
            Self::settled(&mut session);
            return Ok(controllers);
        }
        session.controllers = controllers.clone();
        Ok(controllers)
    }

    /// Re-enumerates every controller on protocol 6 and later (#290).
    ///
    /// The count reply carries the id of every controller, and a controller is asked for and written to
    /// by that id (`NetworkServer.cpp:3585-3640`). Ids come from a counter that only rises, so an id never
    /// names another device: a packet for a controller that a detection has since re-registered is dropped
    /// by the server, never delivered somewhere else. That is what makes a read safe at any time, and
    /// what the rules here rest on:
    ///
    /// - while a detection runs (101 seen, 103 not yet) and a list is held, nothing is read and the held
    ///   list is the answer. With no list held (OpenRGB was still starting), the read is the list;
    /// - a read during which a detection began is thrown away and the held list answers: it may hold
    ///   controllers from before the clear and after it;
    /// - a read during which the list changed with no detection (a device plugged in or removed) is read
    ///   again, a few times at most: the ids it holds are valid, so the last read stands if it keeps
    ///   changing;
    /// - a read after 103 replaces the held list, shorter or longer, empty or not: OpenRGB said the
    ///   detection is over, so an empty list is every device gone.
    ///
    /// No timer decides any of it.
    async fn enumerate_by_id(&self) -> Result<Vec<Controller>, AdapterError> {
        let mut gone = None;
        for attempt in 1..=ENUMERATE_ATTEMPTS {
            let (started_before, changes_before, completed_before) = {
                let session = guard(&self.link.session);
                if session.detecting && !session.controllers.is_empty() {
                    return Ok(session.controllers.clone());
                }
                (session.detections_started, session.list_changes, session.detections_completed)
            };
            guard(&self.link.session).discovering += 1;
            let controllers = {
                let _discovery = Discovery { link: &self.link };
                self.read_controllers_by_id().await
            };
            let controllers = match controllers {
                Ok(controllers) => controllers,
                // An id went away between the count and its data: the list changed, read it again.
                Err(AdapterError::Protocol(reason)) if reason == CONTROLLER_ID_GONE_MESSAGE => {
                    gone = Some(AdapterError::Protocol(reason));
                    continue;
                }
                Err(error) => return Err(error),
            };
            let mut session = guard(&self.link.session);
            if !session.connected {
                return Ok(controllers);
            }
            if session.detections_started != started_before || (session.detecting && !session.controllers.is_empty()) {
                tracing::debug!(read = controllers.len(), held = session.controllers.len(), "a detection began while the list was read; keeping the list held");
                return Ok(session.controllers.clone());
            }
            if (session.list_changes != changes_before || session.detections_completed != completed_before)
                && attempt < ENUMERATE_ATTEMPTS
            {
                continue;
            }
            // The list is final when no detection runs, a 103 was heard and no list change since it. A
            // connection that heard nothing may have joined a running detection (release_1.1 tells a new client
            // nothing of it): what it reads may be a part list, held by the adapter for its ids and never called
            // final; the daemon's own rule decides what to apply (#290).
            if !session.detecting {
                session.end_unread = false;
                session.quiet_read |= session.list_changes == 0;
                session.list_final = session.detections_completed > 0 && session.list_changes == session.list_changes_at_103;
            }
            // The controllers that are gone take their per-controller state with them; a controller
            // that was registered again has a new id and no state, so its first frame sets the custom
            // mode (it sits in its built-in mode).
            let live: HashSet<u32> = controllers.iter().map(|controller| controller.addr).collect();
            session.custom_mode.retain(|addr, _| live.contains(addr));
            session.mode_sent.retain(|addr| live.contains(addr));
            session.resizing.retain(|addr, _| live.contains(addr));
            if session.controllers.len() != controllers.len() {
                tracing::info!(held = session.controllers.len(), read = controllers.len(), "OpenRGB's list has another count of devices");
            }
            session.controllers = controllers.clone();
            return Ok(controllers);
        }
        Err(gone.unwrap_or_else(|| AdapterError::Protocol("OpenRGB's controller list kept changing".into())))
    }

    /// One read of the controller list by id: the count with its ids, then every controller's data.
    async fn read_controllers_by_id(&self) -> Result<Vec<Controller>, AdapterError> {
        let version = self.protocol_version();
        let count_reply = self.request(0, REQUEST_CONTROLLER_COUNT, &[]).await?;
        let (count, ids) = decode_controller_count(&count_reply, version)?;
        if count > MAX_CONTROLLERS {
            return Err(AdapterError::Protocol(format!(
                "OpenRGB reports {count} controllers; the limit is {MAX_CONTROLLERS}")));
        }
        let mut controllers = Vec::with_capacity(ids.len());
        for (index, id) in ids.into_iter().enumerate() {
            let payload = self.request(id, REQUEST_CONTROLLER_DATA, &self.data_request_payload()).await?;
            let wire = WireController::decode(&payload, version)?;
            let leds: u64 = wire.zones.iter().map(|zone| u64::from(zone.leds_count)).sum();
            if leds > MAX_LEDS_PER_CONTROLLER {
                tracing::warn!(id, leds, "skipping an OpenRGB controller over the LED limit of {MAX_LEDS_PER_CONTROLLER}");
                continue;
            }
            controllers.push(Controller { index: index as u32, addr: id, wire });
        }
        Ok(controllers)
    }

    /// The payload of a `REQUEST_CONTROLLER_DATA`: the protocol version before 6 (the server answers in
    /// the version it names), nothing after, because the server uses the one negotiated
    /// (`NetworkServer.cpp:1105-1126`).
    fn data_request_payload(&self) -> Vec<u8> {
        let version = self.protocol_version();
        if version >= PROTOCOL_WITH_EVENTS { Vec::new() } else { version.to_le_bytes().to_vec() }
    }

    /// The list read is the settled one: every controller was registered again, so each sits in its
    /// built-in mode and the next frame to each sends `SET_CUSTOM_MODE` first.
    fn settled(session: &mut Session) {
        session.unsettled = false;
        session.settle_failed_at = None;
        session.custom_mode.clear();
        session.resets += 1;
    }

    /// The epoch of a controller's resize: bumped on every resize, so a frame that was built
    /// for the list before the resize can tell it is stale.
    fn resize_epoch(&self, index: u32) -> u64 {
        guard(&self.link.session).resizing.get(&index).map_or(0, |resize| resize.epoch)
    }
    /// Whether a controller is being resized right now.
    fn resize_in_flight(&self, index: u32) -> bool {
        guard(&self.link.session).resizing.get(&index).is_some_and(|resize| resize.in_flight)
    }

    /// Starts a resize of a controller: the frames for it are skipped until it ends.
    fn begin_resize(&self, index: u32) {
        let mut session = guard(&self.link.session);
        let resize = session.resizing.entry(index).or_insert(Resize { epoch: 0, in_flight: false });
        resize.epoch += 1;
        resize.in_flight = true;
    }

    /// Switches a controller to direct/custom mode, then sends a frame. The mode is sent again
    /// after a reset (resume, resize) and whenever the keepalive interval has passed, so a
    /// controller that fell back to its built-in mode heals on its own.
    async fn send_leds(&self, index: u32, colors: &[Rgb8]) -> Result<(), AdapterError> {
        // The controller's resize epoch: a resize that starts while this frame is on its way
        // makes the frame stale, and the frame may not stamp the mode back on.
        let epoch = self.resize_epoch(index);
        let reason = {
            let session = guard(&self.link.session);
            match session.custom_mode.get(&index) {
                // Sent before on this connection, and forgotten since: a resume or a zone resize.
                None if session.mode_sent.contains(&index) => Some(ModeReason::AfterReset),
                None => Some(ModeReason::First),
                Some(stamp) => self.stale_reason(stamp, session.keepalive),
            }
        };
        if let Some(reason) = reason {
            // Stamped before the send. A resume that lands during the await bumps `resets`; the
            // controller is then not recorded, so the next frame sends the mode again. A resize
            // that lands during the await bumps the epoch; the controller is then not recorded
            // either, so the frame after the resize sends the mode again.
            let sent_at = self.clocks.monotonic();
            let sent_wall = self.clocks.wall();
            let resets = guard(&self.link.session).resets;
            self.send_frame(index, SET_CUSTOM_MODE, &[]).await?;
            let first_on_connection = {
                let mut session = guard(&self.link.session);
                let same_epoch = session.resizing.get(&index).is_none_or(|resize| resize.epoch == epoch);
                if session.resets == resets && same_epoch {
                    session.custom_mode.insert(index, ModeStamp { at: sent_at, wall: sent_wall });
                }
                session.mode_sent.insert(index)
            };
            self.log_mode_sent(index, reason, first_on_connection);
        }
        self.send_frame(index, UPDATE_LEDS, &update_leds_payload(colors)?).await
    }

    /// Says that `SET_CUSTOM_MODE` went to controller `index` (#81). OpenRGB turns each one into an
    /// SMBus mode write on the stick and reports nothing back over the SDK, so this line (what was
    /// sent, to which controller, and why) is the only record. The first one per connection is
    /// INFO; the keepalive and the re-sends after a reset are DEBUG, so a stream of frames adds
    /// no INFO line.
    fn log_mode_sent(&self, index: u32, reason: ModeReason, first_on_connection: bool) {
        let why = reason.text();
        if first_on_connection {
            // The level is checked before the name and the location are copied (#86).
            if !tracing::enabled!(tracing::Level::INFO) {
                return;
            }
            let (name, location) = self.controller_wire(index);
            tracing::info!(
                controller = index, name, location, reason = why,
                "SET_CUSTOM_MODE sent to OpenRGB controller {index} ({name}, {location}): {why}"
            );
        } else {
            // The keepalive sends this line every 10 s per controller. The level is off in a
            // normal run, so nothing is copied for a line that goes nowhere (#86).
            if !tracing::enabled!(tracing::Level::DEBUG) {
                return;
            }
            let (name, location) = self.controller_wire(index);
            tracing::debug!(
                controller = index, name, location, reason = why,
                "SET_CUSTOM_MODE sent to OpenRGB controller {index} ({name}, {location}): {why}"
            );
        }
    }

    /// The controller's name and location, as the SDK reports them.
    fn controller_wire(&self, index: u32) -> (String, String) {
        guard(&self.link.session)
            .controllers
            .iter()
            .find(|controller| controller.addr == index)
            .map(|controller| (controller.wire.name.clone(), controller.wire.location.clone()))
            .unwrap_or_default()
    }

    /// Why a controller's mode has to be sent again, or `None` when it does not.
    ///
    /// The keepalive covers a controller that lost power over a sleep: the ENE DRAM sticks fall
    /// back to their built-in rainbow. The monotonic clock may not have counted the sleep (the
    /// standard library does not say), so the wall clock is watched too: it follows the RTC, so
    /// it counts a sleep whatever the monotonic clock did (#5).
    fn stale_reason(&self, stamp: &ModeStamp, keepalive: Duration) -> Option<ModeReason> {
        if self.clocks.monotonic().saturating_duration_since(stamp.at) >= keepalive {
            return Some(ModeReason::Keepalive);
        }
        // A wall clock that went backwards (the clock was set) is not a sleep.
        self.clocks
            .wall()
            .duration_since(stamp.wall)
            .is_ok_and(|gap| gap > keepalive)
            .then_some(ModeReason::WallClockJump)
    }
}

/// Why `SET_CUSTOM_MODE` is sent to a controller (the log line says which).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModeReason {
    /// The first frame to this controller on this connection.
    First,
    /// The mode map was cleared (a resume or a zone resize) after it was sent before.
    AfterReset,
    /// The keepalive interval passed on the monotonic clock.
    Keepalive,
    /// The wall clock moved on by more than the keepalive while the monotonic clock did not: a sleep.
    WallClockJump,
}

impl ModeReason {
    fn text(self) -> &'static str {
        match self {
            Self::First => "the first frame to this controller on this connection",
            Self::AfterReset => "again after a reset (a resume or a zone resize)",
            Self::Keepalive => "again, the keepalive interval passed",
            Self::WallClockJump => "again, the wall clock jumped past the keepalive (a sleep)",
        }
    }
}

fn read_u32(bytes: &[u8], what: &str) -> Result<u32, AdapterError> {
    match bytes.get(..4) {
        Some(four) => Ok(u32::from_le_bytes([four[0], four[1], four[2], four[3]])),
        None => Err(AdapterError::Protocol(format!("short {what} reply"))),
    }
}

#[async_trait]
impl DeviceAdapter for OpenRgbAdapter {
    fn name(&self) -> &'static str {
        "openrgb"
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
        if guard(&self.link.session).connected {
            return Ok(());
        }
        self.disconnect().await?;
        let stream = TcpStream::connect((self.host.as_str(), self.port)).await?;
        stream.set_nodelay(true)?;
        let (read, write) = stream.into_split();
        let epoch = {
            let mut session = guard(&self.link.session);
            session.epoch += 1;
            session.epoch
        };
        *self.link.writer.lock().await = Some(write);
        // A new socket: the link is whole again.
        self.link.broken.store(false, Ordering::SeqCst);
        let reader = tokio::spawn(read_loop(read, Arc::clone(&self.link), epoch));
        {
            let mut session = guard(&self.link.session);
            session.reader = Some(reader);
            session.connected = true;
        }

        let handshake = async {
            let reply = self.request(0, REQUEST_PROTOCOL_VERSION, &CLIENT_MAX_PROTOCOL_VERSION.to_le_bytes()).await?;
            let server = read_u32(&reply, "protocol version")?;
            if server < MIN_PROTOCOL_VERSION {
                return Err(AdapterError::Protocol(format!(
                    "OpenRGB speaks protocol {server}; canvas-rgb needs {MIN_PROTOCOL_VERSION} or newer")));
            }
            // The server answers with the highest version both sides know.
            guard(&self.link.session).protocol_version = server.min(CLIENT_MAX_PROTOCOL_VERSION);
            self.send(0, SET_CLIENT_NAME, &client_name_payload(canvas_proto::consts::OPENRGB_CLIENT_NAME)).await
        };
        if let Err(error) = handshake.await {
            self.disconnect().await?;
            return Err(error);
        }
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        let reader = {
            let mut session = guard(&self.link.session);
            session.epoch += 1;
            session.connected = false;
            session.pending = None;
            session.controllers.clear();
            session.custom_mode.clear();
            session.mode_sent.clear();
            session.resizing.clear();
            session.unsettled = false;
            session.rescan_requests = 0;
            session.list_changes = 0;
            session.last_list_change = None;
            session.settle_failed_at = None;
            session.settling = false;
            session.detecting = false;
            session.detections_started = 0;
            session.detections_completed = 0;
            session.list_final = false;
            session.list_changes_at_103 = 0;
            session.end_unread = false;
            session.quiet_read = false;
            session.detection_signals = 0;
            session.rescan_acks = 0;
            session.owed_acks.clear();
            session.reader.take()
        };
        if let Some(reader) = reader {
            reader.abort();
        }
        *self.link.writer.lock().await = None;
        self.link.broken.store(false, Ordering::SeqCst);
        // A rescan that is waiting looks at the link again.
        self.link.list_changed.send_modify(|n| *n += 1);
        Ok(())
    }

    fn is_connected(&self) -> bool {
        guard(&self.link.session).connected
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        let controllers = self.enumerate().await?;
        Ok(controllers.iter().map(device_info).collect())
    }

    /// Asks OpenRGB to look at the hardware again and waits for its detection to go quiet (#290).
    /// See [`OpenRgbAdapter::rescan_shared`], which does the work with no `&mut` on the adapter.
    async fn rescan(&mut self) -> Result<(), AdapterError> {
        self.rescan_shared().await.map(|_| ())
    }

    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
        self.write_frame_shared(device, colors).await
    }

    async fn set_zone_size(&mut self, device: &DeviceId, zone: u32, leds: u32) -> Result<(), AdapterError> {
        let controller = self.controller(device).await?;
        let info = controller.wire.zones.get(zone as usize).ok_or_else(|| AdapterError::Zone {
            device: device.clone(), zone, reason: "unknown zone".into(),
        })?;
        if !zone_resizable(info) {
            return Err(AdapterError::Zone { device: device.clone(), zone, reason: "zone is not resizable".into() });
        }
        let size = leds.max(info.leds_min).min(info.leds_max);
        // The controller is being re-configured: a frame for it is skipped until the
        // re-enumerate lands, so no frame of the old zone size reaches OpenRGB in between,
        // and no frame stamps the mode back on after the resize cleared it.
        self.begin_resize(controller.addr);
        // The guard ends the resize however it ends: a handler that was cancelled (the client
        // went away in the middle of the PATCH) must not leave the frames of this controller
        // skipped for the rest of the connection.
        let _resize = ResizeGuard { link: &self.link, index: controller.addr };
        let resized = async {
            self.send_positional(controller.addr, RESIZE_ZONE, &resize_zone_payload(zone, size)).await?;
            // The next frame to this controller must re-enter custom mode after OpenRGB reconfigures it.
            guard(&self.link.session).custom_mode.remove(&controller.addr);
            // Ordered after the resize on the same socket, so the reply reflects the new size.
            self.enumerate().await?;
            Ok(())
        }
        .await;
        resized
    }

    async fn identify(&mut self, device: &DeviceId, zone: u32, led: u32) -> Result<(), AdapterError> {
        let controller = self.controller(device).await?;
        let info = controller.wire.zones.get(zone as usize).ok_or_else(|| AdapterError::Zone {
            device: device.clone(), zone, reason: "unknown zone".into(),
        })?;
        if led >= info.leds_count {
            return Err(AdapterError::Zone {
                device: device.clone(), zone,
                reason: format!("LED {led} is outside the zone's {} LEDs", info.leds_count),
            });
        }
        let total = controller.led_count() as usize;
        let mut colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; total];
        let lit = controller.zone_offset(zone as usize) + led as usize;
        colors[lit] = Rgb8 { r: 255, g: 255, b: 255 };
        self.send_leds(controller.addr, &colors).await?;
        // OpenRGB answers nothing to a frame, so what was sent is the whole record (#81).
        tracing::info!(
            controller = controller.addr, device = %device, zone, led, lit, leds = total,
            "identify sent to OpenRGB controller {} ({device}): LED {lit} of {total} white, the other {} black",
            controller.addr, total - 1
        );
        Ok(())
    }

    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        let controller = self.controller(device).await?;
        let payload = self.request_positional(controller.addr, REQUEST_CONTROLLER_DATA, &self.data_request_payload()).await?;
        Ok(WireController::decode(&payload, self.protocol_version())?.colors)
    }
}

impl crate::resume::ResumeBehavior for OpenRgbAdapter {
    fn on_resume(&self) -> crate::resume::OnResume {
        // A controller that lost power over a sleep falls back to its built-in mode, so the
        // next frame to it sends SET_CUSTOM_MODE again. The resume clears the map of the
        // controllers in custom mode (`reset_custom_mode`), and the keepalive covers the case
        // where the resume never reaches the daemon (#5).
        crate::resume::OnResume::SetsModeAgain
    }
}

/// The scratch the reader throws packets it does not need into. Allocated once per connection.
const DISCARD_CHUNK: usize = 4096;

/// Reads and drops `left` bytes of a packet, in `scratch`-sized pieces: nothing is allocated for a
/// packet the client does not use, however large.
async fn discard(read: &mut OwnedReadHalf, mut left: usize, scratch: &mut [u8]) -> std::io::Result<()> {
    while left > 0 {
        let step = left.min(scratch.len());
        read.read_exact(&mut scratch[..step]).await?;
        left -= step;
    }
    Ok(())
}

/// The reader of one session. Every packet the server sends is one of:
///
/// - the reply to the request that is waiting (same controller and packet id): handed over;
/// - 100 (device list updated), which every protocol sends: counted. On protocol 5 it makes the list
///   unsettled (#290); on protocol 6 it only wakes a waiting rescan;
/// - 101, 102 and 103 (protocol 6 and later): detection started, progressed, finished (#290);
/// - an ACK (protocol 6, one after every packet the client sends, `NetworkServer.cpp:3562-3580`): its
///   id is matched against what the client is owed, and an ACK of a request that has no reply says the
///   controller id is gone;
/// - anything else (the server's name, the echo of every frame to every client of protocol 6 and later,
///   the sender too, `RGBController.cpp:2028-2050`): read and dropped through a fixed scratch, because
///   a stream of frames at 60 per second per device would otherwise allocate a buffer for each echo.
///
/// Only the reader of the current session counts anything: a reader that was overtaken by a reconnect
/// must not touch a session that started after it.
async fn read_loop(mut read: OwnedReadHalf, link: Arc<Link>, epoch: u64) {
    let mut scratch = vec![0_u8; DISCARD_CHUNK];
    loop {
        let mut header = [0; HEADER_LEN];
        if read.read_exact(&mut header).await.is_err() {
            break;
        }
        let Ok((device, packet_id, size)) = decode_header(&header) else {
            break;
        };
        // What this packet is for, decided under one lock; the bytes are read after it is released.
        enum Next {
            Overtaken,
            Reply,
            Drop,
            Ack,
            Progress,
        }
        let next = {
            let mut session = guard(&link.session);
            if session.epoch != epoch {
                Next::Overtaken
            } else {
                match packet_id {
                    DEVICE_LIST_UPDATED => {
                        session.list_changes += 1;
                        session.last_list_change = Some(tokio::time::Instant::now());
                        // A zone resize sends a protocol 5 client no list change (`RGBController.cpp:2271`,
                        // `NetworkServer.cpp:4175-4190`), so every one of these is a change of the list.
                        if session.protocol_version < PROTOCOL_WITH_EVENTS {
                            session.unsettled = true;
                            // The list is now held for being unsettled: the hold of a rescan request is over.
                            session.rescan_requests = 0;
                        }
                        Next::Drop
                    }
                    DETECTION_STARTED => {
                        session.detecting = true;
                        session.detections_started += 1;
                        session.detection_signals += 1;
                        session.list_final = false;
                        Next::Drop
                    }
                    // What it says depends on its percent, read below (#534).
                    DETECTION_PROGRESS_CHANGED => Next::Progress,
                    DETECTION_COMPLETE => {
                        session.detecting = false;
                        session.detections_completed += 1;
                        session.list_changes_at_103 = session.list_changes;
                        session.list_final = false;
                        session.end_unread = true;
                        Next::Drop
                    }
                    ACK => Next::Ack,
                    // The reply to the request that is waiting for it. A reply that arrives late
                    // (the request was given up on) never gets here: an unanswered request loses
                    // the link, and the reader is aborted with it.
                    _ if session.pending.as_ref().is_some_and(|p| p.device == device && p.packet_id == packet_id) => Next::Reply,
                    _ => Next::Drop,
                }
            }
        };
        let read_on = match next {
            Next::Overtaken => return,
            Next::Reply => {
                let mut payload = vec![0; size];
                if read.read_exact(&mut payload).await.is_err() {
                    break;
                }
                let waiting = {
                    let mut session = guard(&link.session);
                    if session.epoch != epoch {
                        return;
                    }
                    session.pending.take_if(|p| p.device == device && p.packet_id == packet_id)
                };
                if let Some(pending) = waiting {
                    let _ = pending.reply.send(Reply::Data(payload));
                }
                Ok(())
            }
            Next::Ack => match read_ack(&mut read, size, &mut scratch).await {
                Ok(acked) => {
                    note_ack(&link, epoch, device, acked);
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Next::Progress => match read_progress_percent(&mut read, size, &mut scratch).await {
                Ok(percent) => {
                    note_progress(&link, epoch, percent);
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Next::Drop => discard(&mut read, size, &mut scratch).await,
        };
        if read_on.is_err() {
            break;
        }
        // Everything that changes what a waiting rescan sees ends here, after the state is set.
        if matches!(packet_id, DEVICE_LIST_UPDATED | DETECTION_STARTED | DETECTION_COMPLETE) {
            link.list_changed.send_modify(|n| *n += 1);
        }
    }
    link.lose(epoch);
}

/// Reads an ACK's payload and returns the id of the packet it acknowledges. A payload of another size
/// is dropped and acknowledges nothing.
async fn read_ack(read: &mut OwnedReadHalf, size: usize, scratch: &mut [u8]) -> std::io::Result<Option<u32>> {
    if size != 8 {
        discard(read, size, scratch).await?;
        return Ok(None);
    }
    let mut payload = [0_u8; 8];
    read.read_exact(&mut payload).await?;
    Ok(Some(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]])))
}

/// Reads a progress report's (102) payload and returns its percent: a `u32` size, then the percent
/// (`NetworkServer.cpp:3963-3999`). A payload too short to hold one is dropped and says nothing.
async fn read_progress_percent(read: &mut OwnedReadHalf, size: usize, scratch: &mut [u8]) -> std::io::Result<Option<u32>> {
    if size < 8 {
        discard(read, size, scratch).await?;
        return Ok(None);
    }
    let mut head = [0_u8; 8];
    read.read_exact(&mut head).await?;
    discard(read, size - 8, scratch).await?;
    Ok(Some(u32::from_le_bytes([head[4], head[5], head[6], head[7]])))
}

/// A progress report (102) at `percent` arrived (#534).
///
/// Inside a detection this connection saw start (101) it is one more step of it. Outside one, its percent
/// says what it is: release_1.1 holds the percent at 100 whenever no detection runs (`DetectionManager.cpp:159`,
/// `:532`, `:861`) and sets it to 0 when one begins (`:717`). A 102 below 100 is a detection that began before
/// this connection and never sent it a 101. A 102 at 100 is the hotplug callback running the HID detectors on
/// a device that just arrived, with no detection running (`:2061-2078`, `:1461`, `:1540`, `:1577`): no 103
/// follows it, so taking it for a detection held the list for good and a replugged device's frames went to
/// its dead id. (A detection's own last 102 is at 100 too, `:861-862`, but its 103 follows at once, `:867`.)
fn note_progress(link: &Link, epoch: u64, percent: Option<u32>) {
    let mut session = guard(&link.session);
    if session.epoch != epoch {
        return;
    }
    if session.detecting {
        session.detection_signals += 1;
        return;
    }
    if percent.is_some_and(|percent| percent < 100) {
        session.detection_signals += 1;
        session.detecting = true;
        session.detections_started += 1;
        session.list_final = false;
    }
}

/// An ACK of packet `acked` for controller `device` arrived.
fn note_ack(link: &Link, epoch: u64, device: u32, acked: Option<u32>) {
    let Some(acked) = acked else { return };
    let refused = {
        let mut session = guard(&link.session);
        if session.epoch != epoch {
            return;
        }
        if device == 0 && acked == REQUEST_RESCAN_DEVICES {
            session.rescan_acks += 1;
        }
        let key = (device, acked);
        let mut last = false;
        if let Some(owed) = session.owed_acks.get_mut(&key) {
            *owed = owed.saturating_sub(1);
            if *owed == 0 {
                last = true;
            }
        }
        if last {
            session.owed_acks.remove(&key);
        }
        // A reply comes before its ACK. The last ACK owed for a request that is still waiting is its own:
        // the server took the request and answered nothing.
        if last { session.pending.take_if(|p| p.device == device && p.packet_id == acked) } else { None }
    };
    if let Some(pending) = refused {
        let _ = pending.reply.send(Reply::Refused);
    }
    if device == 0 && acked == REQUEST_RESCAN_DEVICES {
        link.list_changed.send_modify(|n| *n += 1);
    }
}

/// Linear zones with a size range can be resized over the SDK (MSI JARGB headers, for example).
fn zone_resizable(zone: &WireZone) -> bool {
    zone.zone_type == ZONE_TYPE_LINEAR && zone.leds_min != zone.leds_max
}

fn zone_info(index: u32, zone: &WireZone) -> ZoneInfo {
    let kind = match zone.zone_type {
        ZONE_TYPE_SINGLE => ZoneKind::Single,
        ZONE_TYPE_LINEAR => ZoneKind::Linear,
        ZONE_TYPE_MATRIX => {
            let (height, width, values) = zone.matrix.clone().unwrap_or((0, 0, Vec::new()));
            ZoneKind::Matrix {
                width,
                height,
                map: values.into_iter().map(|v| (v != MATRIX_GAP).then_some(v)).collect(),
            }
        }
        _ => ZoneKind::Linear,
    };
    ZoneInfo {
        index,
        name: zone.name.clone(),
        kind,
        led_count: zone.leds_count,
        min_leds: zone.leds_min,
        max_leds: zone.leds_max,
        resizable: zone_resizable(zone),
    }
}

fn device_info(controller: &Controller) -> DeviceInfo {
    let wire = &controller.wire;
    let fingerprint = if wire.serial.trim().is_empty() { &wire.location } else { &wire.serial };
    DeviceInfo {
        id: DeviceId(format!("openrgb:{fingerprint}")),
        name: wire.name.clone(),
        vendor: wire.vendor.clone(),
        kind: device_kind(wire.device_type),
        adapter: "openrgb".into(),
        adapter_index: controller.index,
        serial: wire.serial.clone(),
        location: wire.location.clone(),
        zones: wire.zones.iter().enumerate().map(|(i, zone)| zone_info(i as u32, zone)).collect(),
        connected: true,
    }
}

/// OpenRGB device type to canvas-rgb's kind. Values follow OpenRGB's `DeviceType` enum.
fn device_kind(value: i32) -> DeviceKind {
    match value {
        0 => DeviceKind::Motherboard,
        1 => DeviceKind::Dram,
        2 => DeviceKind::Gpu,
        4 => DeviceKind::LedStrip,
        5 => DeviceKind::Keyboard,
        6 => DeviceKind::Mouse,
        8 => DeviceKind::Headset,
        16 => DeviceKind::Microphone,
        _ => DeviceKind::Other,
    }
}

// ---- hostile OpenRGB listener: wire sizes must not drive allocation or arithmetic

#[cfg(test)]
mod hostile_tests {
    use crate::openrgb::OpenRgbAdapter;
    use crate::protocol::*;
    use canvas_proto::DeviceAdapter;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Answers the handshake, then `controller_count` and `controller_data` as given.
    async fn serve(count: u32, data: Vec<Vec<u8>>) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            loop {
                let mut header = [0u8; HEADER_LEN];
                if stream.read_exact(&mut header).await.is_err() {
                    return;
                }
                let (device, id, size) = decode_header(&header).unwrap();
                let mut payload = vec![0u8; size];
                if stream.read_exact(&mut payload).await.is_err() {
                    return;
                }
                let reply = match id {
                    REQUEST_PROTOCOL_VERSION => 5u32.to_le_bytes().to_vec(),
                    REQUEST_CONTROLLER_COUNT => count.to_le_bytes().to_vec(),
                    REQUEST_CONTROLLER_DATA => data[(device as usize).min(data.len() - 1)].clone(),
                    _ => continue,
                };
                if stream.write_all(&encode_packet(device, id, &reply).unwrap()).await.is_err() {
                    return;
                }
            }
        });
        port
    }

    fn controller(zone_leds: &[u32]) -> Vec<u8> {
        let wire = WireController {
            name: "Evil".into(),
            serial: "x".into(),
            modes: vec![WireMode { name: "Direct".into(), value: 0, color_mode: 1, ..Default::default() }],
            zones: zone_leds
                .iter()
                .map(|&n| WireZone { name: "Z".into(), zone_type: 1, leds_count: n, ..Default::default() })
                .collect(),
            ..Default::default()
        };
        wire.encode().unwrap()
    }

    #[tokio::test]
    async fn a_huge_controller_count_is_refused_not_allocated() {
        let port = serve(u32::MAX, vec![controller(&[3])]).await;
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", port);
        adapter.connect().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), adapter.discover()).await;
        assert!(matches!(result, Ok(Err(_))), "expected an error, got {result:?}");
    }

    #[tokio::test]
    async fn a_controller_over_the_led_cap_is_skipped_not_fatal() {
        let port = serve(2, vec![controller(&[u32::MAX, u32::MAX]), controller(&[3])]).await;
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", port);
        adapter.connect().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), adapter.discover()).await;
        assert!(matches!(&result, Ok(Ok(devices)) if devices.len() == 1), "expected the one good controller, got {result:?}");
    }
}

// ---- the socket-lock re-check of a positional packet (#509)

/// A packet that names a controller by its position passes the settle check in `controller()` and then waits
/// for the socket lock. A rescan that takes its hold in between has to stop it: the check under the lock is the
/// only thing that does. The socket lock is held by the test, so the packet waits at it for as long as the test
/// wants; one poll of the packet's future runs it as far as the lock, and nothing is timed.
#[cfg(test)]
mod rescan_hold_tests {
    use super::*;
    use crate::fake::FakeOpenRgbServer;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };

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
                min_leds: 4,
                max_leds: 16,
                resizable: true,
            }],
            connected: true,
        }
    }

    /// A protocol 5 server with two sticks and an adapter that has read them.
    async fn connected() -> (FakeOpenRgbServer, OpenRgbAdapter) {
        let server = FakeOpenRgbServer::start(vec![stick("a"), stick("b")]).await.expect("the fake server");
        server.server_protocol_version(5);
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
        adapter.connect().await.expect("connect");
        assert_eq!(adapter.discover().await.expect("discover").len(), 2);
        (server, adapter)
    }

    /// One poll: the future has run as far as it can without the socket lock.
    fn waits<F: Future>(future: &mut Pin<Box<F>>) -> bool {
        let mut context = Context::from_waker(std::task::Waker::noop());
        matches!(future.as_mut().poll(&mut context), Poll::Pending)
    }

    fn is_unsettled_refusal<T: std::fmt::Debug>(result: &Result<T, AdapterError>) -> bool {
        matches!(result, Err(AdapterError::Protocol(reason)) if reason == LIST_UNSETTLED_MESSAGE)
    }

    /// Waits until a frame sent after the refusal has landed: the fake reads one connection in order, so a
    /// packet sent before it has been handled by then.
    async fn after_a_later_frame_landed(server: &FakeOpenRgbServer, adapter: &OpenRgbAdapter) {
        adapter.write_frame_shared(&DeviceId("openrgb:a".into()), &[RED; 8]).await.expect("a frame once the hold is over");
        tokio::time::timeout(Duration::from_secs(30), async {
            while server.landings().is_empty() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("the later frame never landed");
    }

    #[tokio::test]
    async fn a_frame_that_passed_the_settle_check_is_refused_when_the_hold_is_taken_before_its_write() {
        let (server, adapter) = connected().await;
        let a = DeviceId("openrgb:a".into());
        let socket = adapter.link.writer.lock().await;
        let mut frame = Box::pin(adapter.write_frame_shared(&a, &[RED; 8]));
        assert!(waits(&mut frame), "the frame did not wait for the socket lock");
        let hold = adapter.hold_for_rescan();
        drop(socket);
        let result = frame.await;
        assert!(is_unsettled_refusal(&result), "a frame went out after the rescan's hold was taken: {result:?}");
        drop(hold);
        after_a_later_frame_landed(&server, &adapter).await;
        assert_eq!(server.landings().len(), 1, "the refused frame reached OpenRGB: {:?}", server.landings());
    }

    #[tokio::test]
    async fn a_resize_that_passed_the_settle_check_is_refused_when_the_hold_is_taken_before_its_write() {
        let (server, adapter) = connected().await;
        let a = DeviceId("openrgb:a".into());
        let socket = adapter.link.writer.lock().await;
        let mut sizing = adapter.clone();
        let mut resize = Box::pin(async move { sizing.set_zone_size(&a, 0, 12).await });
        assert!(waits(&mut resize), "the resize did not wait for the socket lock");
        let hold = adapter.hold_for_rescan();
        drop(socket);
        let result = resize.await;
        assert!(is_unsettled_refusal(&result), "a resize went out after the rescan's hold was taken: {result:?}");
        drop(hold);
        after_a_later_frame_landed(&server, &adapter).await;
        assert_eq!(server.zone_leds(0, 0), Some(8), "the refused resize reached OpenRGB");
        assert_eq!(server.resize_count(0) + server.resize_count(1), 0, "the refused resize reached OpenRGB");
    }

    #[tokio::test]
    async fn a_read_back_that_passed_the_settle_check_is_refused_when_the_hold_is_taken_before_its_write() {
        let (server, adapter) = connected().await;
        let a = DeviceId("openrgb:a".into());
        let socket = adapter.link.writer.lock().await;
        let mut reading = adapter.clone();
        let mut read = Box::pin(async move { reading.read_back(&a).await });
        assert!(waits(&mut read), "the read did not wait for the socket lock");
        let hold = adapter.hold_for_rescan();
        drop(socket);
        let result = read.await;
        assert!(is_unsettled_refusal(&result), "a read went out after the rescan's hold was taken: {result:?}");
        drop(hold);
        // The refused read left the link whole and its reply slot free: the next read answers.
        assert!(adapter.is_connected(), "the link broke over a refused read");
        adapter.clone().read_back(&DeviceId("openrgb:b".into())).await.expect("a read once the hold is over");
        drop(server);
    }
}

// ---- the keepalive over a sleep: the wall clock counts it, the monotonic clock may not

#[cfg(test)]
mod keepalive_tests {
    use super::*;
    use crate::fake::FakeOpenRgbServer;
    use std::sync::Mutex;

    const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };

    /// A DRAM stick, as the fake server reports it.
    fn stick() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId("openrgb:dram".into()),
            name: "DDR5".into(),
            vendor: "fake".into(),
            kind: DeviceKind::Dram,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: "dram".into(),
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

    /// The clocks of an awake machine, which the test moves.
    struct Moved {
        monotonic: Mutex<Instant>,
        wall: Mutex<SystemTime>,
    }

    impl Moved {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                monotonic: Mutex::new(Instant::now()),
                wall: Mutex::new(SystemTime::now()),
            })
        }

        /// The clocks that read this state.
        fn clocks(state: &Arc<Self>) -> Clocks {
            let monotonic = Arc::clone(state);
            let wall = Arc::clone(state);
            Clocks::moved(
                Arc::new(move || *monotonic.monotonic.lock().expect("the clock's lock")),
                Arc::new(move || *wall.wall.lock().expect("the clock's lock")),
            )
        }
    }

    /// The adapter over the fake server, with the clocks the test moves.
    async fn adapter(state: &Arc<Moved>) -> (FakeOpenRgbServer, OpenRgbAdapter) {
        let server = FakeOpenRgbServer::start(vec![stick()]).await.expect("the fake server");
        let mut adapter =
            OpenRgbAdapter::with_clocks("127.0.0.1", server.port(), CUSTOM_MODE_KEEPALIVE, Moved::clocks(state));
        adapter.connect().await.expect("connect");
        (server, adapter)
    }

    /// Writes a frame and waits for the server to have handled it.
    async fn frame(adapter: &mut OpenRgbAdapter) {
        let id = DeviceId("openrgb:dram".into());
        adapter.write_frame(&id, &[RED; 8]).await.expect("frame");
        // The read-back is a request and a reply, so the frame before it has been handled.
        adapter.read_back(&id).await.expect("readback orders the frame");
    }

    /// A second stick, as the fake server reports it (controller 1).
    fn second_stick() -> DeviceInfo {
        let mut info = stick();
        info.id = DeviceId("openrgb:dram2".into());
        info.serial = "dram2".into();
        info.location = "smbus2".into();
        info.adapter_index = 1;
        info
    }

    /// The adapter over a fake server with two sticks, and the log it writes.
    async fn two_sticks(state: &Arc<Moved>) -> (FakeOpenRgbServer, OpenRgbAdapter) {
        let server = FakeOpenRgbServer::start(vec![stick(), second_stick()]).await.expect("the fake server");
        let mut adapter =
            OpenRgbAdapter::with_clocks("127.0.0.1", server.port(), CUSTOM_MODE_KEEPALIVE, Moved::clocks(state));
        adapter.connect().await.expect("connect");
        (server, adapter)
    }

    async fn frame_to(adapter: &mut OpenRgbAdapter, id: &str) {
        let id = DeviceId(id.into());
        adapter.write_frame(&id, &[RED; 8]).await.expect("frame");
        adapter.read_back(&id).await.expect("readback orders the frame");
    }

    /// #81: the first SET_CUSTOM_MODE to each controller on a connection is one INFO line naming the
    /// controller; later frames add none, and the keepalive and a reset re-send at DEBUG.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn the_first_set_custom_mode_per_controller_is_one_info_line_and_the_rest_are_debug() {
        let _serial = crate::resume_serial();
        let logs = crate::test_logs::LogBuf::default();
        let _default = logs.install();
        let state = Moved::new();
        let (server, mut adapter) = two_sticks(&state).await;

        for _ in 0..5 {
            frame_to(&mut adapter, "openrgb:dram").await;
            frame_to(&mut adapter, "openrgb:dram2").await;
        }
        assert_eq!(server.custom_mode_writes(0), 1);
        assert_eq!(server.custom_mode_writes(1), 1);
        let info = logs.lines_with(&["INFO", "SET_CUSTOM_MODE sent to OpenRGB controller"]);
        assert_eq!(info.len(), 2, "one INFO line per controller, none per frame: {info:?}");
        assert!(info[0].contains("controller 0 (DDR5, smbus)") && info[0].contains("first frame"), "{}", info[0]);
        assert!(info[1].contains("controller 1 (DDR5, smbus2)") && info[1].contains("first frame"), "{}", info[1]);

        // The keepalive: DEBUG, not INFO.
        *state.monotonic.lock().expect("the clock's lock") += CUSTOM_MODE_KEEPALIVE;
        frame_to(&mut adapter, "openrgb:dram").await;
        let keepalive = logs.lines_with(&["DEBUG", "controller 0", "keepalive interval passed"]);
        assert_eq!(keepalive.len(), 1, "{}", logs.text());

        // After a reset: DEBUG, and it says so.
        adapter.reset_custom_mode();
        frame_to(&mut adapter, "openrgb:dram2").await;
        let reset = logs.lines_with(&["DEBUG", "controller 1", "after a reset"]);
        assert_eq!(reset.len(), 1, "{}", logs.text());

        let info = logs.lines_with(&["INFO", "SET_CUSTOM_MODE sent to OpenRGB controller"]);
        assert_eq!(info.len(), 2, "the re-sends added no INFO line: {info:?}");

        // A new connection is a new "first".
        adapter.disconnect().await.expect("disconnect");
        adapter.connect().await.expect("connect again");
        frame_to(&mut adapter, "openrgb:dram").await;
        let info = logs.lines_with(&["INFO", "SET_CUSTOM_MODE sent to OpenRGB controller 0"]);
        assert_eq!(info.len(), 2, "the first frame on the new connection is INFO again: {info:?}");
    }

    /// #81: an identify says which controller it went to and what was sent.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn an_identify_is_logged_with_its_controller_and_what_was_sent() {
        let _serial = crate::resume_serial();
        let logs = crate::test_logs::LogBuf::default();
        let _default = logs.install();
        let state = Moved::new();
        let (_server, mut adapter) = two_sticks(&state).await;

        adapter.identify(&DeviceId("openrgb:dram2".into()), 0, 3).await.expect("identify");

        let lines = logs.lines_with(&["INFO", "identify sent to OpenRGB controller 1"]);
        assert_eq!(lines.len(), 1, "{}", logs.text());
        assert!(lines[0].contains("openrgb:dram2"), "{}", lines[0]);
        assert!(lines[0].contains("LED 3 of 8 white, the other 7 black"), "{}", lines[0]);
        // The mode write that went with it is on record too.
        assert_eq!(logs.lines_with(&["SET_CUSTOM_MODE sent to OpenRGB controller 1"]).len(), 1);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn a_sleep_the_monotonic_clock_did_not_count_still_makes_the_mode_be_set_again() {
        let _serial = crate::resume_serial();
        let state = Moved::new();
        let (server, mut adapter) = adapter(&state).await;
        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1, "the first frame sets the mode");

        // The machine sleeps for an hour. The monotonic clock stands still (the standard
        // library does not say that a suspend counts), the wall clock follows the RTC.
        *state.wall.lock().expect("the clock's lock") += Duration::from_secs(3600);

        frame(&mut adapter).await;
        assert_eq!(
            server.custom_mode_writes(0),
            2,
            "the frame after the sleep did not set the mode again"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn the_keepalive_fires_when_the_monotonic_clock_passed_it() {
        let _serial = crate::resume_serial();
        let state = Moved::new();
        let (server, mut adapter) = adapter(&state).await;
        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1);

        *state.monotonic.lock().expect("the clock's lock") += CUSTOM_MODE_KEEPALIVE;

        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 2, "the keepalive set the mode again");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn a_wall_clock_set_backwards_is_not_a_sleep() {
        let _serial = crate::resume_serial();
        let state = Moved::new();
        let (server, mut adapter) = adapter(&state).await;
        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1);

        // The clock was set an hour back: the wall clock moved away from the stamp, so it is
        // not a sleep and the mode stays where it is.
        *state.wall.lock().expect("the clock's lock") -= Duration::from_secs(3600);

        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1, "the mode was set again for no reason");
    }
}

// ---- #290: the reader signals a rescan only for the session it belongs to

#[cfg(test)]
mod rescan_signal_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// A link whose current session is `epoch`, a reader of `reader_epoch` on one end of a socket,
    /// and the other end of that socket.
    async fn reader_on(epoch: u64, reader_epoch: u64) -> (Arc<Link>, tokio::task::JoinHandle<()>, TcpStream) {
        let adapter = OpenRgbAdapter::new("127.0.0.1", 1);
        let link = Arc::clone(&adapter.link);
        guard(&link.session).epoch = epoch;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let connecting = tokio::spawn(TcpStream::connect(("127.0.0.1", port)));
        let (server, _) = listener.accept().await.expect("accept");
        let (read, _write) = connecting.await.expect("task").expect("connect").into_split();
        let reader = tokio::spawn(read_loop(read, Arc::clone(&link), reader_epoch));
        (link, reader, server)
    }

    /// The reader of the current session counts a list change (the control for the test below).
    #[tokio::test]
    async fn the_reader_of_the_current_session_signals_a_list_change() {
        let (link, _reader, mut server) = reader_on(1, 1).await;
        let mut changes = link.list_changed.subscribe();
        let _ = changes.borrow_and_update();
        server.write_all(&encode_packet(0, DEVICE_LIST_UPDATED, &[]).expect("packet")).await.expect("write");
        tokio::time::timeout(Duration::from_secs(5), changes.changed())
            .await
            .expect("the change was never counted")
            .expect("the signal is alive");
    }

    /// A reader that a reconnect overtook must not wake a rescan that started on the new session, nor
    /// make its list unsettled: it counts nothing and ends.
    #[tokio::test]
    async fn an_overtaken_reader_signals_nothing_and_ends() {
        let (link, reader, mut server) = reader_on(2, 1).await;
        let changes = link.list_changed.subscribe();
        server.write_all(&encode_packet(0, DEVICE_LIST_UPDATED, &[]).expect("packet")).await.expect("write");
        tokio::time::timeout(Duration::from_secs(5), reader)
            .await
            .expect("the overtaken reader ends")
            .expect("its task ends");
        assert!(!changes.has_changed().expect("alive"), "an overtaken reader counted a list change");
        assert!(!guard(&link.session).unsettled, "an overtaken reader made the new session's list unsettled");
    }

    /// An overtaken reader takes no 102 for a detection of the new session either (#537).
    #[tokio::test]
    async fn an_overtaken_reader_takes_no_102_for_a_detection() {
        let (link, reader, mut server) = reader_on(2, 1).await;
        let mut payload = 8_u32.to_le_bytes().to_vec();
        payload.extend_from_slice(&50_u32.to_le_bytes());
        server.write_all(&encode_packet(0, DETECTION_PROGRESS_CHANGED, &payload).expect("packet")).await.expect("write");
        tokio::time::timeout(Duration::from_secs(5), reader)
            .await
            .expect("the overtaken reader ends")
            .expect("its task ends");
        let session = guard(&link.session);
        assert!(!session.detecting, "an overtaken reader marked the new session as detecting");
        assert_eq!(session.detection_signals, 0);
    }
}

// ---- the link-break rule: a request or a write that is given up on

/// The rule, in three parts (#308):
///
/// - a request that went out and was given up on breaks the link: OpenRGB answers in order
///   and puts no request id in a reply, so the late reply would answer the next request;
/// - a request that was given up on before it went out breaks nothing: nothing is owed;
/// - a write that was given up on breaks the link while it still holds the socket lock, so
///   a writer that waits for the lock cannot write into the middle of the half-written packet.
///
/// The fakes: the fake OpenRGB server and the real adapter, so the whole link is walked.
#[cfg(test)]
mod link_break_tests {
    use super::*;
    use crate::fake::FakeOpenRgbServer;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// How many LEDs the controller has. A frame of this size is a big packet, so the socket
    /// fills up after a few of them.
    const LEDS: u32 = 8192;

    /// A controller with many LEDs, as the fake server reports it.
    fn big_stick() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId("openrgb:big".into()),
            name: "Big stick".into(),
            vendor: "fake".into(),
            kind: DeviceKind::Dram,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: "big".into(),
            location: "smbus".into(),
            zones: vec![ZoneInfo {
                index: 0,
                name: "RGB".into(),
                kind: ZoneKind::Linear,
                led_count: LEDS,
                min_leds: LEDS,
                max_leds: LEDS,
                resizable: false,
            }],
            connected: true,
        }
    }

    /// Waits until `ready` is true. The wait is on the thing the assert reads; the long limit
    /// only ends a test that would hang.
    async fn until(what: &str, mut ready: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("never happened: {what}"));
    }

    /// Whether the future cannot go on: it is still pending after one poll.
    ///
    /// The poll is the whole check: a write to a socket with no room for it cannot start, and
    /// the future says so. No clock is involved, and nothing is waited for.
    fn is_blocked<F: Future>(future: &mut Pin<Box<F>>) -> bool {
        let mut context = Context::from_waker(std::task::Waker::noop());
        matches!(future.as_mut().poll(&mut context), Poll::Pending)
    }

    /// A write that cannot go out, holding the socket lock.
    ///
    /// The server stops reading, so the socket fills up and a write cannot start. The write
    /// stays alive: it is stuck in the middle of its packet and holds the socket lock, the
    /// way a write that a caller gave up on does.
    fn stuck_write<'a>(
        adapter: &'a OpenRgbAdapter,
        id: &'a DeviceId,
        colors: &'a [Rgb8],
    ) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        for _ in 0..200 {
            let mut write = Box::pin(adapter.write_frame_shared(id, colors));
            if is_blocked(&mut write) {
                return Box::pin(async move {
                    let _ = write.await;
                });
            }
            // The write went out: the socket is filling up.
        }
        panic!("the socket never filled: the write never blocked");
    }

    /// A request that went out and was given up on leaves the link unusable.
    ///
    /// The request goes out and the server keeps the reply. The caller gives up on it (a
    /// client that went away): the reply may still come, and it would answer the next
    /// request, so the link is dropped and the supervisor reconnects.
    #[tokio::test]
    async fn a_request_that_went_out_and_was_given_up_on_leaves_the_link_broken() {
        let server = FakeOpenRgbServer::start(vec![big_stick()]).await.expect("the fake server");
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
        adapter.connect().await.expect("connect");
        adapter.discover().await.expect("discover");

        // The server keeps its replies: the request goes out and waits for an answer.
        server.hold_replies(true);
        let id = DeviceId("openrgb:big".into());
        let mut asking = adapter.clone();
        let asking_id = id.clone();
        let request = tokio::spawn(async move { asking.read_back(&asking_id).await });
        until("the request waits for a reply that never comes", || server.held_replies() == 1).await;

        // The caller gives up on it.
        request.abort();
        let _ = request.await;

        assert!(
            !adapter.is_connected(),
            "the link stayed up after a request that went out and was given up on"
        );
    }

    /// A request that was given up on before it went out leaves the link whole.
    ///
    /// The socket is full, so the request waits for the socket lock and never goes out.
    /// Nothing is owed, so nothing is broken: the link stays up and the next request gets
    /// its own answer.
    #[tokio::test]
    async fn a_request_that_was_given_up_on_before_it_went_out_leaves_the_link_whole() {
        let server = FakeOpenRgbServer::start(vec![big_stick()]).await.expect("the fake server");
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
        adapter.connect().await.expect("connect");
        adapter.discover().await.expect("discover");

        // The socket fills up, so a write cannot go out and holds the socket lock.
        server.stop_reading(true);
        let id = DeviceId("openrgb:big".into());
        let colors = vec![Rgb8::WHITE; LEDS as usize];
        let stuck = stuck_write(&adapter, &id, &colors);

        // A request waits for the socket lock: it has not gone out.
        let mut asking = adapter.clone();
        let asking_id = id.clone();
        let request = tokio::spawn(async move { asking.read_back(&asking_id).await });
        // The request has taken its reply slot, so it is waiting for the socket lock.
        until("the request waits for the socket lock", || {
            guard(&adapter.link.session).pending.is_some()
        })
        .await;

        // The caller gives up on it.
        request.abort();
        let _ = request.await;

        assert!(
            adapter.is_connected(),
            "the link broke over a request that never went out"
        );
        drop(stuck);
    }

    /// A write that was given up on breaks the link before the socket lock is released.
    ///
    /// The write is stuck in the middle of its packet and holds the socket lock. A second
    /// writer waits for that lock. The first write is given up on: the link is broken while
    /// the lock is still held, so the second writer cannot take the lock, see a whole link
    /// and write into the middle of the half-written packet.
    #[tokio::test]
    async fn a_write_that_was_given_up_on_breaks_the_link_before_the_socket_lock_is_released() {
        let server = FakeOpenRgbServer::start(vec![big_stick()]).await.expect("the fake server");
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
        adapter.connect().await.expect("connect");
        adapter.discover().await.expect("discover");

        // The socket fills up, so a write cannot go out and holds the socket lock.
        server.stop_reading(true);
        let id = DeviceId("openrgb:big".into());
        let colors = vec![Rgb8::WHITE; LEDS as usize];
        let stuck = stuck_write(&adapter, &id, &colors);

        // A second writer waits for the socket lock. It says so before it waits, so the test
        // knows it is queued: the lock is held until the first write is given up on.
        let (started, waiting) = oneshot::channel();
        let second = adapter.clone();
        let second_id = id.clone();
        let second_colors = colors.clone();
        let writer = tokio::spawn(async move {
            let _ = started.send(());
            second.write_frame_shared(&second_id, &second_colors).await
        });
        waiting.await.expect("the second writer starts");

        // The first write is given up on: the guard breaks the link while it still holds the
        // socket lock.
        drop(stuck);

        let result = tokio::time::timeout(Duration::from_secs(30), writer)
            .await
            .expect("the second writer ends")
            .expect("its task ends");
        assert!(
            result.is_err(),
            "the second writer wrote into the middle of the half-written packet"
        );
        assert!(
            !adapter.is_connected(),
            "the link stayed up after a write that was given up on"
        );
    }
}

// ---- #534: a 102 marks a detection only below 100 %, and a short one keeps the stream framed

#[cfg(test)]
mod progress_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// A link on session 1, the task reading the client end of a socket for it, and the server end.
    async fn reader() -> (Arc<Link>, tokio::task::JoinHandle<()>, TcpStream) {
        let adapter = OpenRgbAdapter::new("127.0.0.1", 1);
        let link = Arc::clone(&adapter.link);
        guard(&link.session).epoch = 1;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let connecting = tokio::spawn(TcpStream::connect(("127.0.0.1", port)));
        let (server, _) = listener.accept().await.expect("accept");
        let (read, _write) = connecting.await.expect("task").expect("connect").into_split();
        let reader = tokio::spawn(read_loop(read, Arc::clone(&link), 1));
        (link, reader, server)
    }

    /// A 102 as release_1.1 sends it: a size that counts itself, the percent, then a text
    /// (`NetworkServer.cpp:3963-3999`).
    fn progress(percent: u32) -> Vec<u8> {
        let text = b"\0";
        let mut payload = (4 + 4 + 2 + text.len() as u32).to_le_bytes().to_vec();
        payload.extend_from_slice(&percent.to_le_bytes());
        payload.extend_from_slice(&(text.len() as u16).to_le_bytes());
        payload.extend_from_slice(text);
        encode_packet(0, DETECTION_PROGRESS_CHANGED, &payload).expect("packet")
    }

    /// Writes `bytes`, then a list change, and waits until the reader has counted that change: every
    /// packet before it was read by then.
    async fn send_then_sync(link: &Link, server: &mut TcpStream, bytes: &[u8]) {
        let mut changes = link.list_changed.subscribe();
        let _ = changes.borrow_and_update();
        server.write_all(bytes).await.expect("write");
        server.write_all(&encode_packet(0, DEVICE_LIST_UPDATED, &[]).expect("packet")).await.expect("write");
        tokio::time::timeout(Duration::from_secs(5), changes.changed())
            .await
            .expect("the list change after the 102s was never read: the stream lost its framing")
            .expect("the signal is alive");
    }

    #[tokio::test]
    async fn a_102_at_100_or_too_short_is_no_detection_and_one_at_99_is() {
        let (link, _reader, mut server) = reader().await;

        // A hotplug's 102 at 100 %, then two 102s too short to hold a percent (4 and 7 bytes; 7 would
        // eat a byte of the next header if it were read as 8).
        let short = |len: usize| {
            let payload = vec![99_u8; len];
            encode_packet(0, DETECTION_PROGRESS_CHANGED, &payload).expect("packet")
        };
        // The list is final going in, so a 102 that wrongly ends that shows (#537).
        guard(&link.session).list_final = true;
        let mut bytes = progress(100);
        bytes.extend_from_slice(&short(4));
        bytes.extend_from_slice(&short(7));
        send_then_sync(&link, &mut server, &bytes).await;
        {
            let session = guard(&link.session);
            assert!(!session.detecting, "a 102 at 100 % or a short one was taken for a detection");
            assert_eq!(session.detections_started, 0);
            assert_eq!(session.detection_signals, 0);
            assert!(session.list_final, "a 102 at 100 % or a short one ended a final list");
        }

        // 99 % in the shortest payload that holds a percent (8 bytes: the size, then the percent) is a
        // detection that began before this connection, and the list (still final from above) is final no longer.
        let mut eight = 8_u32.to_le_bytes().to_vec();
        eight.extend_from_slice(&99_u32.to_le_bytes());
        let eight = encode_packet(0, DETECTION_PROGRESS_CHANGED, &eight).expect("packet");
        send_then_sync(&link, &mut server, &eight).await;
        let session = guard(&link.session);
        assert!(session.detecting, "a 102 at 99 % was not taken for a running detection");
        assert_eq!(session.detections_started, 1);
        assert_eq!(session.detection_signals, 1);
        assert!(!session.list_final, "a detection that began left the list final");
    }
}
