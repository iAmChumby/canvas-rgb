//! Native adapter for the ROYUAN MonsGeek keyboard (USB 3151:4015).
//!
//! The board is one LED: a single colour for the whole keyboard. It is driven over its
//! vendor HID interface (interface 2, usage page 0xFFFF, usage 0x0002) with two 65-byte
//! feature reports. The key-press interfaces (0 and 1) are never opened.

use async_trait::async_trait;
use canvas_proto::color::Rgb8;
use canvas_proto::device::{AdapterCapabilities, DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::traits::{AdapterError, DeviceAdapter};
use std::time::{Duration, Instant};

pub const VENDOR_ID: u16 = 0x3151;
pub const PRODUCT_ID: u16 = 0x4015;
const INTERFACE: i32 = 2;
const USAGE_PAGE: u16 = 0xFFFF;
const USAGE: u16 = 0x0002;
const REPORT_LEN: usize = 65;
/// Re-send the same colour at least this often.
const KEEPALIVE: Duration = Duration::from_secs(1);

/// The report that switches the keyboard to direct (host-driven) lighting.
pub fn direct_mode_report() -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[1..10].copy_from_slice(&[0x07, 0x15, 0x04, 0x04, 0x07, 0x00, 0x00, 0x00, 0xD4]);
    r
}

/// The report that sets the whole keyboard to one colour.
pub fn color_report(c: Rgb8) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[1] = 0x0E;
    r[2] = c.r;
    r[3] = c.g;
    r[4] = c.b;
    r[8] = 255u8.wrapping_sub(0x0E).wrapping_sub(c.r).wrapping_sub(c.g).wrapping_sub(c.b);
    r
}

fn device_id() -> DeviceId {
    DeviceId(format!("royuan:{VENDOR_ID:04x}:{PRODUCT_ID:04x}"))
}

// ---------------------------------------------------------------- transport

/// An open handle that can send one feature report.
///
/// This is the seam the tests fake (the guard test over the daemon's adapters needs to see the
/// reports the keyboard sends), the way the service's `PortIo` and `FanChip` are.
pub trait Transport: Send + Sync {
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String>;
}

/// Finds and opens the vendor interface.
///
/// The seam the tests fake; the product uses [`HidOpener`].
pub trait Opener: Send + Sync {
    /// True when the vendor interface is present.
    fn present(&self) -> bool;
    /// Opens the vendor interface. `Ok(None)` when it is not there.
    fn open(&self) -> Result<Option<Box<dyn Transport>>, String>;
}

/// `HidDevice` is `Send` but not `Sync`; the mutex supplies `Sync` (it is never contended).
struct HidTransport(std::sync::Mutex<hidapi::HidDevice>);

impl Transport for HidTransport {
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        dev.send_feature_report(data).map_err(|e| e.to_string())
    }
}

struct HidOpener;

impl HidOpener {
    fn find_path(api: &hidapi::HidApi) -> Option<std::ffi::CString> {
        api.device_list()
            .find(|d| {
                d.vendor_id() == VENDOR_ID
                    && d.product_id() == PRODUCT_ID
                    && d.interface_number() == INTERFACE
                    && d.usage_page() == USAGE_PAGE
                    && d.usage() == USAGE
            })
            .map(|d| d.path().to_owned())
    }
}

impl Opener for HidOpener {
    fn present(&self) -> bool {
        hidapi::HidApi::new().map(|api| Self::find_path(&api).is_some()).unwrap_or(false)
    }

    fn open(&self) -> Result<Option<Box<dyn Transport>>, String> {
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        match Self::find_path(&api) {
            None => Ok(None),
            Some(path) => {
                let dev = api.open_path(&path).map_err(|e| e.to_string())?;
                Ok(Some(Box::new(HidTransport(std::sync::Mutex::new(dev)))))
            }
        }
    }
}

// ---------------------------------------------------------------- adapter

pub struct RoyuanAdapter {
    opener: Box<dyn Opener>,
    dev: Option<Box<dyn Transport>>,
    /// Colour and time of the last successful send.
    last_sent: Option<(Rgb8, Instant)>,
    /// Last colour handed to `write_frame` that went out (for `read_back`).
    last_color: Rgb8,
    /// The resume count when the keyboard last got its direct-mode report.
    seen_resume: u64,
    /// Set after a resume: the next write sends the direct-mode report again (#5).
    direct_pending: bool,
}

impl Default for RoyuanAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl RoyuanAdapter {
    pub fn new() -> Self {
        Self::with_opener(Box::new(HidOpener))
    }

    /// The adapter with an opener of the caller's.
    ///
    /// The product uses the HID opener; the tests use a fake one, so they can count the
    /// reports the keyboard sends.
    pub fn with_opener(opener: Box<dyn Opener>) -> Self {
        Self { opener, dev: None, last_sent: None, last_color: Rgb8::BLACK, seen_resume: crate::resume_epoch(), direct_pending: false }
    }

    fn check_id(&self, id: &DeviceId) -> Result<(), AdapterError> {
        if *id == device_id() {
            Ok(())
        } else {
            Err(AdapterError::UnknownDevice(id.clone()))
        }
    }

    fn open_and_init(&mut self) -> Result<(), AdapterError> {
        let mut dev = self
            .opener
            .open()
            .map_err(AdapterError::Protocol)?
            .ok_or(AdapterError::NotConnected)?;
        dev.send_feature(&direct_mode_report()).map_err(AdapterError::Protocol)?;
        self.dev = Some(dev);
        self.last_sent = None;
        self.seen_resume = crate::resume_epoch();
        self.direct_pending = false;
        Ok(())
    }

    fn write_at(&mut self, color: Rgb8, now: Instant) -> Result<(), AdapterError> {
        let Some(dev) = self.dev.as_mut() else {
            return Err(AdapterError::NotConnected);
        };
        if crate::resume_epoch() != self.seen_resume {
            self.seen_resume = crate::resume_epoch();
            self.direct_pending = true;
        }
        if self.direct_pending {
            if let Err(e) = dev.send_feature(&direct_mode_report()) {
                self.dev = None;
                self.last_sent = None;
                return Err(AdapterError::Protocol(e));
            }
            self.direct_pending = false;
            self.last_sent = None;
        }
        if let Some((last, at)) = self.last_sent {
            if last == color && now.saturating_duration_since(at) < KEEPALIVE {
                return Ok(());
            }
        }
        match dev.send_feature(&color_report(color)) {
            Ok(()) => {
                self.last_sent = Some((color, now));
                self.last_color = color;
                Ok(())
            }
            Err(e) => {
                self.dev = None;
                self.last_sent = None;
                Err(AdapterError::Protocol(e))
            }
        }
    }

    fn device_info(&self) -> DeviceInfo {
        DeviceInfo {
            id: device_id(),
            name: "ROYUAN MonsGeek Keyboard".into(),
            vendor: "ROYUAN".into(),
            kind: DeviceKind::Keyboard,
            adapter: "royuan".into(),
            adapter_index: 0,
            serial: String::new(),
            location: format!("hid:{VENDOR_ID:04x}:{PRODUCT_ID:04x}:if{INTERFACE}"),
            zones: vec![ZoneInfo {
                index: 0,
                name: "Keyboard".into(),
                kind: ZoneKind::Single,
                led_count: 1,
                min_leds: 1,
                max_leds: 1,
                resizable: false,
            }],
            connected: true,
        }
    }
}

#[async_trait]
impl DeviceAdapter for RoyuanAdapter {
    fn name(&self) -> &'static str {
        "royuan"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            resizable_zones: false,
            identify: true,
            hardware_brightness: false,
            needs_elevation: false,
        }
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        if self.dev.is_some() {
            return Ok(());
        }
        self.open_and_init()
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        self.dev = None;
        self.last_sent = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.dev.is_some()
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        if self.dev.is_some() || self.opener.present() {
            Ok(vec![self.device_info()])
        } else {
            Ok(Vec::new())
        }
    }

    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
        self.check_id(device)?;
        let color = colors.first().copied().unwrap_or(Rgb8::BLACK);
        self.write_at(color, Instant::now())
    }

    async fn set_zone_size(&mut self, device: &DeviceId, _zone: u32, _leds: u32) -> Result<(), AdapterError> {
        self.check_id(device)?;
        Err(AdapterError::Unsupported("the ROYUAN keyboard has one fixed LED"))
    }

    async fn identify(&mut self, device: &DeviceId, zone: u32, _led: u32) -> Result<(), AdapterError> {
        self.check_id(device)?;
        if zone != 0 {
            return Err(AdapterError::Zone { device: device.clone(), zone, reason: "no such zone".into() });
        }
        self.write_at(Rgb8::WHITE, Instant::now())
    }

    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        self.check_id(device)?;
        Ok(vec![self.last_color])
    }
}

impl crate::resume::ResumeBehavior for RoyuanAdapter {
    fn on_resume(&self) -> crate::resume::OnResume {
        // The keyboard may drop out of direct mode over a sleep (seen for the RAM in #5; not
        // yet seen for the keyboard), so the direct-mode report goes out again before the next
        // colour.
        crate::resume::OnResume::SetsModeAgain
    }
}

#[cfg(test)]
mod tests {
    // The serial guard is held across awaits on purpose: it keeps tests that count reports apart.
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Shared {
        sent: Vec<Vec<u8>>,
        opens: u32,
        present: bool,
        fail_next_send: bool,
    }

    struct FakeOpener(Arc<Mutex<Shared>>);
    struct FakeTransport(Arc<Mutex<Shared>>);

    impl Transport for FakeTransport {
        fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
            let mut s = self.0.lock().unwrap();
            if s.fail_next_send {
                s.fail_next_send = false;
                return Err("unplugged".into());
            }
            s.sent.push(data.to_vec());
            Ok(())
        }
    }

    impl Opener for FakeOpener {
        fn present(&self) -> bool {
            self.0.lock().unwrap().present
        }
        fn open(&self) -> Result<Option<Box<dyn Transport>>, String> {
            let mut s = self.0.lock().unwrap();
            if !s.present {
                return Ok(None);
            }
            s.opens += 1;
            Ok(Some(Box::new(FakeTransport(self.0.clone()))))
        }
    }

    /// `note_resume` bumps a process-wide counter, so the tests that count reports run one at a time.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        crate::resume_serial()
    }

    fn rig(present: bool) -> (RoyuanAdapter, Arc<Mutex<Shared>>) {
        let shared = Arc::new(Mutex::new(Shared { present, ..Default::default() }));
        (RoyuanAdapter::with_opener(Box::new(FakeOpener(shared.clone()))), shared)
    }

    fn direct_count(s: &Shared) -> usize {
        s.sent.iter().filter(|r| r.as_slice() == direct_mode_report()).count()
    }

    #[test]
    fn direct_mode_bytes() {
        let r = direct_mode_report();
        assert_eq!(r.len(), 65);
        assert_eq!(&r[..10], &[0x00, 0x07, 0x15, 0x04, 0x04, 0x07, 0x00, 0x00, 0x00, 0xD4]);
        assert!(r[10..].iter().all(|b| *b == 0));
    }

    #[test]
    fn color_bytes_and_checksums() {
        let black = color_report(Rgb8::new(0, 0, 0));
        assert_eq!(&black[..9], &[0x00, 0x0E, 0, 0, 0, 0, 0, 0, 0xF1]);
        assert!(black[9..].iter().all(|b| *b == 0));
        let red = color_report(Rgb8::new(0xFF, 0, 0));
        assert_eq!(&red[..9], &[0x00, 0x0E, 0xFF, 0, 0, 0, 0, 0, 0xF2]);
        let c = color_report(Rgb8::new(1, 2, 3));
        assert_eq!(c[2..5], [1, 2, 3]);
        assert_eq!(c[8], 255u8.wrapping_sub(0x0E + 6));
        assert_eq!(c.len(), 65);
    }

    #[tokio::test]
    async fn discover_empty_when_absent() {
        let _serial = serial();
        let (mut a, _) = rig(false);
        assert!(a.discover().await.unwrap().is_empty());
        assert!(matches!(a.connect().await, Err(AdapterError::NotConnected)));
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn discover_describes_one_led_keyboard() {
        let _serial = serial();
        let (mut a, _) = rig(true);
        let d = a.discover().await.unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].id, DeviceId("royuan:3151:4015".into()));
        assert_eq!(d[0].name, "ROYUAN MonsGeek Keyboard");
        assert_eq!(d[0].kind, DeviceKind::Keyboard);
        assert_eq!(d[0].led_count(), 1);
        assert!(d[0].connected);
        assert_eq!(a.name(), "royuan");
    }

    #[tokio::test]
    async fn direct_mode_sent_once_per_connect() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        a.connect().await.unwrap();
        a.connect().await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 1);
        a.disconnect().await.unwrap();
        assert!(!a.is_connected());
        a.connect().await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 2);
    }

    #[tokio::test]
    async fn after_note_resume_the_next_write_sends_direct_mode_before_the_colour() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        let red = [Rgb8::new(255, 0, 0)];
        a.write_frame(&id, &red).await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 1);
        a.write_frame(&id, &red).await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 1, "no resume: no repeat");

        // The production path: the daemon calls note_resume; the adapter notices on its next write.
        crate::note_resume();
        a.write_frame(&id, &red).await.unwrap();
        let g = s.lock().unwrap();
        assert_eq!(direct_count(&g), 2, "direct mode went out again");
        let n = g.sent.len();
        assert_eq!(g.sent[n - 2].as_slice(), direct_mode_report(), "before the colour");
        assert_eq!(&g.sent[n - 2][1..10], &[0x07, 0x15, 0x04, 0x04, 0x07, 0x00, 0x00, 0x00, 0xD4]);
        assert_eq!(g.sent[n - 1].as_slice(), color_report(red[0]), "the colour follows");
        drop(g);
        a.write_frame(&id, &red).await.unwrap();
        assert_eq!(direct_count(&s.lock().unwrap()), 2, "once per resume");
    }

    #[tokio::test]
    async fn unchanged_colour_not_resent_within_a_second() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        let red = [Rgb8::new(255, 0, 0)];
        a.write_frame(&id, &red).await.unwrap();
        a.write_frame(&id, &red).await.unwrap();
        a.write_frame(&id, &red).await.unwrap();
        // direct mode + one colour
        assert_eq!(s.lock().unwrap().sent.len(), 2);
        // keepalive after a second
        let (c, at) = a.last_sent.unwrap();
        a.write_at(c, at + Duration::from_millis(999)).unwrap();
        assert_eq!(s.lock().unwrap().sent.len(), 2);
        a.write_at(c, at + Duration::from_secs(1)).unwrap();
        assert_eq!(s.lock().unwrap().sent.len(), 3);
    }

    #[tokio::test]
    async fn changed_colour_is_sent() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        a.write_frame(&id, &[Rgb8::new(255, 0, 0)]).await.unwrap();
        a.write_frame(&id, &[Rgb8::new(0, 255, 0)]).await.unwrap();
        {
            let g = s.lock().unwrap();
            assert_eq!(g.sent.len(), 3);
            assert_eq!(g.sent[2], color_report(Rgb8::new(0, 255, 0)).to_vec());
        }
        assert_eq!(a.read_back(&id).await.unwrap(), vec![Rgb8::new(0, 255, 0)]);
    }

    #[tokio::test]
    async fn empty_frame_sends_black() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        a.write_frame(&id, &[Rgb8::new(9, 9, 9)]).await.unwrap();
        a.write_frame(&id, &[]).await.unwrap();
        let g = s.lock().unwrap();
        assert_eq!(g.sent.last().unwrap(), &color_report(Rgb8::BLACK).to_vec());
    }

    #[tokio::test]
    async fn failed_send_closes_and_reconnect_resends_direct_mode() {
        let _serial = serial();
        let (mut a, s) = rig(true);
        let id = device_id();
        a.connect().await.unwrap();
        s.lock().unwrap().fail_next_send = true;
        assert!(a.write_frame(&id, &[Rgb8::new(1, 1, 1)]).await.is_err());
        assert!(!a.is_connected());
        assert!(matches!(
            a.write_frame(&id, &[Rgb8::new(1, 1, 1)]).await,
            Err(AdapterError::NotConnected)
        ));
        a.connect().await.unwrap();
        let g = s.lock().unwrap();
        assert_eq!(g.opens, 2);
        assert_eq!(direct_count(&g), 2);
    }

    #[tokio::test]
    async fn unsupported_and_unknown() {
        let _serial = serial();
        let (mut a, _) = rig(true);
        let id = device_id();
        assert!(matches!(a.set_zone_size(&id, 0, 5).await, Err(AdapterError::Unsupported(_))));
        let other = DeviceId("x".into());
        assert!(matches!(a.write_frame(&other, &[]).await, Err(AdapterError::UnknownDevice(_))));
    }
}
