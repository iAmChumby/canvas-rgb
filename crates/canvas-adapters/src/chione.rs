//! Native adapter for the GAMDIAS CHIONE Digital Display (USB HID 1B80:B53C), the pump-head
//! display that shows one four-digit number. The protocol is the one decoded from ZEUS CAST in
//! `.build/chione/recon.md`; section numbers below refer to that file.
//!
//! Structure follows `royuan.rs`: a private `Transport` trait over hidapi, an `Opener` that finds
//! the device by VID/PID, and a fake transport for the tests. The tests never touch the real
//! device; only `ChioneDisplay::new()` uses the hidapi opener, and nothing in this crate calls
//! `discover` or `connect` on it except the app when it is asked to.

use async_trait::async_trait;
use canvas_proto::device::DeviceId;
use canvas_proto::display::{DisplayInfo, DisplayMode, DisplayUnit, DisplayValue};
use canvas_proto::traits::{AdapterError, NumericDisplayAdapter};
use std::time::Duration;

pub const VENDOR_ID: u16 = 0x1B80;
pub const PRODUCT_ID: u16 = 0xB53C;
const DISPLAY_NAME: &str = "GAMDIAS CHIONE Digital Display";
const REPORT_LEN: usize = 65;
/// Report ID 0, then the `3A B5` header (recon section 2).
const HEADER: [u8; 3] = [0x00, 0x3A, 0xB5];
const CMD_DISPLAY: u8 = 0x01;
const CMD_QUERY: u8 = 0xFF;
/// A blank digit (recon section 2, bytes 4..7).
const BLANK: u8 = 0x20;
const DIGITS: usize = 4;
const MAX_DECIMALS: u8 = 1;
/// Recon section 2: the I/O loop waits 250 ms between writes.
const WRITE_INTERVAL_MS: u32 = 250;
/// Recon section 2: ZEUS CAST waits 500 ms after the 0xFF query before it reads the reply.
const QUERY_WAIT: Duration = Duration::from_millis(500);
/// ASSUMED. Recon gives no timeout for the query reply (ZEUS CAST reads untimed). One second
/// is long enough for a live device and keeps a silent device from hanging `connect`.
const QUERY_REPLY_TIMEOUT_MS: i32 = 1000;
/// ASSUMED. Recon says ZEUS CAST reads one 65-byte report after every display write, and does not
/// inspect it. The read here is timed so a silent device cannot stall the 250 ms loop. A timeout
/// on this read is not an error: the packet was already written.
const ACK_TIMEOUT_MS: i32 = 100;

fn device_id() -> DeviceId {
    DeviceId(format!("chione:{VENDOR_ID:04x}:{PRODUCT_ID:04x}"))
}

fn refuse(msg: impl Into<String>) -> AdapterError {
    AdapterError::Protocol(msg.into())
}

/// Recon section 2, byte 13: sum of bytes 0..=12, masked to 0xFF (ZEUS CAST `GetCSValue`).
fn checksum(report: &[u8; REPORT_LEN]) -> u8 {
    report[..13].iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

/// The 0xFF startup query (recon section 2): `00 3A B5 FF 20 20 20 20 00 01 01 00 00 <cs>`.
pub fn query_report() -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[..3].copy_from_slice(&HEADER);
    r[3] = CMD_QUERY;
    r[4..8].fill(BLANK);
    r[9] = 1;
    r[10] = 1;
    let cs = checksum(&r);
    r[13] = cs;
    r
}

/// Four digit positions, most significant first, right-aligned. A leading zero becomes a blank,
/// but the `min_digits` rightmost positions always show a digit: with one decimal place the
/// units digit is kept, so 0.5 shows `0.5`, not `.5`.
fn digits_for(n: u32, min_digits: usize) -> [u8; DIGITS] {
    let mut out = [BLANK; DIGITS];
    let mut rest = n;
    for (shown, slot) in out.iter_mut().rev().enumerate() {
        *slot = (rest % 10) as u8;
        rest /= 10;
        if rest == 0 && shown + 1 >= min_digits {
            break;
        }
    }
    out
}

/// Encodes one display value as the 65-byte output report (recon section 2).
///
/// The value is already in its own unit: a Fahrenheit value is sent as given (the host converts
/// before it gets here). A value that cannot be shown is refused, never truncated.
pub fn encode(value: &DisplayValue) -> Result<[u8; REPORT_LEN], AdapterError> {
    if value.decimals > MAX_DECIMALS {
        return Err(refuse(format!(
            "{} decimal places requested; the CHIONE shows at most {MAX_DECIMALS}",
            value.decimals
        )));
    }
    let temperature = value.mode == DisplayMode::Temperature;
    // UNKNOWN (recon section 2): the RPM layout has no dot (byte 8 = 0 in RPM mode). Refused
    // rather than rounded, so a fan speed never shows a decimal point the device might not place.
    if !temperature && value.decimals != 0 {
        return Err(refuse("fan RPM is a whole number; decimals must be 0"));
    }

    let mut digits = [BLANK; DIGITS];
    let mut dot = false;
    if let Some(v) = value.value {
        if !v.is_finite() {
            return Err(refuse(format!("{v} is not a finite number")));
        }
        // UNKNOWN (recon section 4): the device's minus sign. ZEUS CAST has no negative path, so
        // the conservative choice is to refuse rather than wrap the value through a byte.
        if v < 0.0 {
            return Err(refuse(format!("{v} is negative; the CHIONE's minus sign is unknown")));
        }
        let scaled = (f64::from(v) * 10f64.powi(i32::from(value.decimals))).round();
        if scaled > 9999.0 {
            return Err(refuse(format!(
                "{v} with {} decimal places does not fit four digits",
                value.decimals
            )));
        }
        digits = digits_for(scaled as u32, usize::from(value.decimals) + 1);
        // UNKNOWN (recon section 4): the device places the dot before the last digit; that is what
        // the temperature layout (xxx.x) assumes. Whole numbers carry no dot.
        dot = temperature && value.decimals == 1;
    }

    let mut r = [0u8; REPORT_LEN];
    r[..3].copy_from_slice(&HEADER);
    r[3] = CMD_DISPLAY;
    r[4..8].copy_from_slice(&digits);
    r[8] = u8::from(dot);
    // Byte 9: 1 = Celsius, 0 = Fahrenheit, in temperature mode. Recon says 0 in RPM mode.
    r[9] = u8::from(temperature && value.unit == DisplayUnit::Celsius);
    // Byte 10: the CPU legend, lit in temperature mode and off in RPM mode.
    r[10] = u8::from(temperature);
    // Byte 11: 0 = temperature mode, 1 = fan-RPM mode.
    r[11] = u8::from(!temperature);
    // Byte 12: alarm blink.
    r[12] = u8::from(value.blink);
    let cs = checksum(&r);
    r[13] = cs;
    Ok(r)
}

// ---------------------------------------------------------------- transport

/// An open handle: one 65-byte write and one timed 65-byte read.
trait Transport: Send + Sync {
    /// Writes one output report (byte 0 is the report ID). Returns the bytes written.
    fn write(&mut self, data: &[u8]) -> Result<usize, String>;
    /// Reads one input report into `buf`. `Ok(0)` means nothing arrived within `timeout_ms`.
    fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String>;
}

/// Finds and opens the device.
trait Opener: Send + Sync {
    /// True when the device is enumerated. Does not open it.
    fn present(&self) -> bool;
    /// Opens the device. `Ok(None)` when it is not there.
    fn open(&self) -> Result<Option<Box<dyn Transport>>, String>;
}

/// `HidDevice` is `Send` but not `Sync`; the mutex supplies `Sync` (it is never contended).
struct HidTransport(std::sync::Mutex<hidapi::HidDevice>);

impl Transport for HidTransport {
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        dev.write(data).map_err(|e| e.to_string())
    }

    fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        let dev = self.0.get_mut().map_err(|e| e.to_string())?;
        dev.read_timeout(buf, timeout_ms).map_err(|e| e.to_string())
    }
}

struct HidOpener;

impl HidOpener {
    fn find_path(api: &hidapi::HidApi) -> Option<std::ffi::CString> {
        api.device_list()
            .find(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID)
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

// ---------------------------------------------------------------- handshake

/// Checks the 0xFF reply (recon section 2). The reply layout: `[4]` hFW, `[5]` lFW, `[6]` tempFW,
/// `[7]` cpuFW, `[8]` fanFW. The recon gives three variants:
/// cpuFW=1 with tempFW=1 (Celsius start), cpuFW=1 with tempFW=0 (Fahrenheit start), fanFW=1
/// (RPM variant). Anything else is refused as garbled.
///
/// UNKNOWN (recon section 2): the values this unit reports. Conservative choice: the flag bytes
/// must each be 0 or 1, and exactly one of cpuFW and fanFW must be 1. A unit that reports
/// something else is refused with the bytes in the message, so the first hardware test can see
/// them. Settler: the first hardware test, step 1.
fn check_query_reply(reply: &[u8]) -> Result<(), AdapterError> {
    if reply.is_empty() {
        return Err(refuse("no reply to the CHIONE 0xFF query within 1000 ms"));
    }
    if reply.len() != REPORT_LEN {
        return Err(refuse(format!(
            "garbled CHIONE query reply: {} bytes, expected {REPORT_LEN}",
            reply.len()
        )));
    }
    let (temp_fw, cpu_fw, fan_fw) = (reply[6], reply[7], reply[8]);
    let flags_ok = [temp_fw, cpu_fw, fan_fw].iter().all(|f| *f <= 1);
    if !flags_ok || cpu_fw == fan_fw {
        return Err(refuse(format!(
            "garbled CHIONE query reply: tempFW={temp_fw} cpuFW={cpu_fw} fanFW={fan_fw}"
        )));
    }
    tracing::info!(
        hfw = reply[4],
        lfw = reply[5],
        temp_fw,
        cpu_fw,
        fan_fw,
        "CHIONE query reply"
    );
    Ok(())
}

/// Sends the 0xFF query, waits, and reads and checks the reply. Any failure is an error; the
/// caller drops the device.
async fn query(dev: &mut dyn Transport, wait: Duration) -> Result<(), AdapterError> {
    let sent = dev
        .write(&query_report())
        .map_err(|e| refuse(format!("CHIONE query write failed: {e}")))?;
    if sent != REPORT_LEN {
        return Err(refuse(format!("CHIONE query: short write of {sent} bytes")));
    }
    tokio::time::sleep(wait).await;
    let mut reply = [0u8; REPORT_LEN];
    let n = dev
        .read(&mut reply, QUERY_REPLY_TIMEOUT_MS)
        .map_err(|e| refuse(format!("CHIONE query read failed: {e}")))?;
    check_query_reply(&reply[..n])
}

// ---------------------------------------------------------------- adapter

pub struct ChioneDisplay {
    opener: Box<dyn Opener>,
    dev: Option<Box<dyn Transport>>,
    /// Wait between the query and its reply read. `QUERY_WAIT` in production; tests use zero.
    query_wait: Duration,
}

impl Default for ChioneDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl ChioneDisplay {
    pub fn new() -> Self {
        Self::with_opener(Box::new(HidOpener), QUERY_WAIT)
    }

    fn with_opener(opener: Box<dyn Opener>, query_wait: Duration) -> Self {
        Self { opener, dev: None, query_wait }
    }

    fn check_id(&self, id: &DeviceId) -> Result<(), AdapterError> {
        if *id == device_id() {
            Ok(())
        } else {
            Err(AdapterError::UnknownDevice(id.clone()))
        }
    }

    fn display_info(&self) -> DisplayInfo {
        DisplayInfo {
            id: device_id(),
            name: DISPLAY_NAME.into(),
            digits: DIGITS as u8,
            max_decimals: MAX_DECIMALS,
            modes: vec![DisplayMode::Temperature, DisplayMode::FanRpm],
            connected: self.dev.is_some(),
        }
    }
}

#[async_trait]
impl NumericDisplayAdapter for ChioneDisplay {
    fn name(&self) -> &'static str {
        "chione"
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        if self.dev.is_some() {
            return Ok(());
        }
        let mut dev = self
            .opener
            .open()
            .map_err(AdapterError::Protocol)?
            .ok_or(AdapterError::NotConnected)?;
        // On error `dev` is dropped here, so a failed handshake leaves the adapter unconnected.
        query(dev.as_mut(), self.query_wait).await?;
        self.dev = Some(dev);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        self.dev = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.dev.is_some()
    }

    async fn discover(&mut self) -> Result<Vec<DisplayInfo>, AdapterError> {
        if self.dev.is_some() || self.opener.present() {
            Ok(vec![self.display_info()])
        } else {
            Ok(Vec::new())
        }
    }

    async fn write_value(&mut self, device: &DeviceId, value: &DisplayValue) -> Result<(), AdapterError> {
        self.check_id(device)?;
        // Encode first: a value that cannot be shown is refused before anything is written.
        let report = encode(value)?;
        let Some(dev) = self.dev.as_mut() else {
            return Err(AdapterError::NotConnected);
        };
        match dev.write(&report) {
            Ok(REPORT_LEN) => {}
            Ok(sent) => {
                self.dev = None;
                return Err(refuse(format!("CHIONE display write: short write of {sent} bytes")));
            }
            Err(e) => {
                self.dev = None;
                return Err(refuse(format!("CHIONE display write failed: {e}")));
            }
        }
        // ZEUS CAST reads one input report after every write. Read it here too, so the device's
        // input queue does not fill, and discard it.
        let mut ack = [0u8; REPORT_LEN];
        match dev.read(&mut ack, ACK_TIMEOUT_MS) {
            Ok(0) => tracing::debug!("CHIONE: no ack within {ACK_TIMEOUT_MS} ms"),
            Ok(_) => {}
            Err(e) => {
                self.dev = None;
                return Err(refuse(format!("CHIONE ack read failed: {e}")));
            }
        }
        Ok(())
    }

    fn write_interval_ms(&self) -> u32 {
        WRITE_INTERVAL_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Shared {
        /// Every write, in order (the fake records what the device would have received).
        writes: Vec<Vec<u8>>,
        /// Replies to hand back from `read`, one per call. An empty queue means a timeout (`Ok(0)`).
        replies: VecDeque<Vec<u8>>,
        opens: u32,
        present: bool,
        fail_next_write: bool,
    }

    struct FakeOpener(Arc<Mutex<Shared>>);
    struct FakeTransport(Arc<Mutex<Shared>>);

    impl Transport for FakeTransport {
        fn write(&mut self, data: &[u8]) -> Result<usize, String> {
            let mut s = self.0.lock().unwrap();
            if s.fail_next_write {
                s.fail_next_write = false;
                return Err("unplugged".into());
            }
            s.writes.push(data.to_vec());
            Ok(data.len())
        }

        fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            let mut s = self.0.lock().unwrap();
            match s.replies.pop_front() {
                None => Ok(0),
                Some(reply) => {
                    let n = reply.len().min(buf.len());
                    buf[..n].copy_from_slice(&reply[..n]);
                    Ok(n)
                }
            }
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

    fn rig(present: bool, replies: Vec<Vec<u8>>) -> (ChioneDisplay, Arc<Mutex<Shared>>) {
        let shared = Arc::new(Mutex::new(Shared {
            present,
            replies: replies.into(),
            ..Default::default()
        }));
        (
            ChioneDisplay::with_opener(Box::new(FakeOpener(shared.clone())), Duration::ZERO),
            shared,
        )
    }

    /// A 65-byte reply with the given firmware and variant bytes.
    fn reply(hfw: u8, lfw: u8, temp_fw: u8, cpu_fw: u8, fan_fw: u8) -> Vec<u8> {
        let mut r = vec![0u8; REPORT_LEN];
        r[4] = hfw;
        r[5] = lfw;
        r[6] = temp_fw;
        r[7] = cpu_fw;
        r[8] = fan_fw;
        r
    }

    /// The expected report: the given leading bytes, then zeros (recon: bytes 14..64 are 0x00).
    fn expect(head: &[u8]) -> [u8; REPORT_LEN] {
        let mut r = [0u8; REPORT_LEN];
        r[..head.len()].copy_from_slice(head);
        r
    }

    fn value(v: Option<f32>, decimals: u8, unit: DisplayUnit, mode: DisplayMode, blink: bool) -> DisplayValue {
        DisplayValue { value: v, decimals, unit, mode, blink }
    }

    fn temp_c(v: f32) -> DisplayValue {
        value(Some(v), 1, DisplayUnit::Celsius, DisplayMode::Temperature, false)
    }

    fn id() -> DeviceId {
        device_id()
    }

    // ------------------------------------------------------------ encode: golden bytes

    #[test]
    fn golden_42_5_celsius() {
        // digits: blank,4,2,5; dot 1; unit C 1; cpu 1; mode 0; blink 0; sum 286 -> 0x1E
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x04, 0x02, 0x05, 0x01, 0x01, 0x01, 0x00, 0x00, 0x1E]);
        assert_eq!(encode(&temp_c(42.5)).unwrap(), want);
    }

    #[test]
    fn golden_100_0_fahrenheit_is_not_converted() {
        // digits: 1,0,0,0; dot 1; unit F 0; cpu 1; sum 243 -> 0xF3. A value of 100.0 shown as
        // 100.0 (not 37.8) proves the host's number goes through unchanged.
        let v = value(Some(100.0), 1, DisplayUnit::Fahrenheit, DisplayMode::Temperature, false);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xF3]);
        assert_eq!(encode(&v).unwrap(), want);
    }

    #[test]
    fn golden_7_with_no_decimals() {
        // blank,blank,blank,7; no dot; unit C 1; cpu 1; sum 345 -> 0x59
        let v = value(Some(7.0), 0, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x20, 0x20, 0x07, 0x00, 0x01, 0x01, 0x00, 0x00, 0x59]);
        assert_eq!(encode(&v).unwrap(), want);
    }

    #[test]
    fn golden_1234_rpm() {
        // digits 1,2,3,4; no dot; unit 0; cpu 0; mode 1; sum 251 -> 0xFB
        let v = value(Some(1234.0), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, false);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x01, 0x02, 0x03, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0xFB]);
        assert_eq!(encode(&v).unwrap(), want);
    }

    #[test]
    fn golden_blank() {
        // all four digits blank; no dot; unit C 1; cpu 1; sum 370 -> 0x72
        let v = value(None, 1, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x20, 0x20, 0x20, 0x00, 0x01, 0x01, 0x00, 0x00, 0x72]);
        assert_eq!(encode(&v).unwrap(), want);
    }

    #[test]
    fn golden_blink_sets_byte_12_and_changes_checksum() {
        // as 42.5 C, byte 12 = 1; sum 287 -> 0x1F
        let v = value(Some(42.5), 1, DisplayUnit::Celsius, DisplayMode::Temperature, true);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x04, 0x02, 0x05, 0x01, 0x01, 0x01, 0x00, 0x01, 0x1F]);
        assert_eq!(encode(&v).unwrap(), want);
    }

    #[test]
    fn query_golden() {
        // 00 3A B5 FF 20 20 20 20 00 01 01 00 00, sum 624 -> 0x70
        let want = expect(&[0x00, 0x3A, 0xB5, 0xFF, 0x20, 0x20, 0x20, 0x20, 0x00, 0x01, 0x01, 0x00, 0x00, 0x70]);
        assert_eq!(query_report(), want);
    }

    #[test]
    fn largest_values_fit() {
        let r = encode(&temp_c(999.9)).unwrap();
        assert_eq!(&r[4..8], &[0x09, 0x09, 0x09, 0x09]);
        let v = value(Some(9999.0), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, false);
        assert_eq!(&encode(&v).unwrap()[4..8], &[0x09, 0x09, 0x09, 0x09]);
    }

    #[test]
    fn zero_shows_a_digit_not_a_blank() {
        // 0.0 is "0.0": the units digit stays, the tenths digit stays
        let r = encode(&temp_c(0.0)).unwrap();
        assert_eq!(&r[4..8], &[0x20, 0x20, 0x00, 0x00]);
        // 0.5 is "0.5", not ".5"
        let r = encode(&temp_c(0.5)).unwrap();
        assert_eq!(&r[4..8], &[0x20, 0x20, 0x00, 0x05]);
        // whole zero with no decimals is "0"
        let v = value(Some(0.0), 0, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        assert_eq!(&encode(&v).unwrap()[4..8], &[0x20, 0x20, 0x20, 0x00]);
    }

    #[test]
    fn rounding_to_decimals() {
        // 42.46 rounds to 42.5, 42.44 rounds to 42.4
        assert_eq!(&encode(&temp_c(42.46)).unwrap()[4..8], &[0x20, 0x04, 0x02, 0x05]);
        assert_eq!(&encode(&temp_c(42.44)).unwrap()[4..8], &[0x20, 0x04, 0x02, 0x04]);
        // whole numbers round to the nearest integer
        let v = value(Some(7.6), 0, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        assert_eq!(&encode(&v).unwrap()[4..8], &[0x20, 0x20, 0x20, 0x08]);
    }

    // ------------------------------------------------------------ encode: refusals

    #[test]
    fn refuses_more_than_one_decimal() {
        let v = value(Some(1.0), 2, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        assert!(matches!(encode(&v), Err(AdapterError::Protocol(_))));
    }

    #[test]
    fn refuses_too_big_for_four_digits() {
        let whole = value(Some(10000.0), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, false);
        assert!(matches!(encode(&whole), Err(AdapterError::Protocol(_))));
        // 1000.0 needs five digits with one decimal
        assert!(matches!(encode(&temp_c(1000.0)), Err(AdapterError::Protocol(_))));
    }

    #[test]
    fn refuses_rounding_up_past_four_digits() {
        // 999.96 rounds to 1000.0, which does not fit
        assert!(matches!(encode(&temp_c(999.96)), Err(AdapterError::Protocol(_))));
        // 9999.5 rounds to 10000 with no decimals
        let v = value(Some(9999.5), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, false);
        assert!(matches!(encode(&v), Err(AdapterError::Protocol(_))));
    }

    #[test]
    fn refuses_negative_values() {
        assert!(matches!(encode(&temp_c(-1.0)), Err(AdapterError::Protocol(_))));
        assert!(matches!(encode(&temp_c(-0.04)), Err(AdapterError::Protocol(_))));
    }

    #[test]
    fn refuses_non_finite_values() {
        assert!(matches!(encode(&temp_c(f32::NAN)), Err(AdapterError::Protocol(_))));
        assert!(matches!(encode(&temp_c(f32::INFINITY)), Err(AdapterError::Protocol(_))));
    }

    #[test]
    fn refuses_decimal_places_in_fan_mode() {
        let v = value(Some(1234.0), 1, DisplayUnit::Celsius, DisplayMode::FanRpm, false);
        assert!(matches!(encode(&v), Err(AdapterError::Protocol(_))));
    }

    #[test]
    fn checksum_is_sum_of_first_13_bytes_mod_256() {
        let cases = [
            temp_c(42.5),
            temp_c(0.0),
            temp_c(999.9),
            value(None, 1, DisplayUnit::Fahrenheit, DisplayMode::Temperature, true),
            value(Some(1234.0), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, true),
        ];
        for v in cases {
            let r = encode(&v).unwrap();
            let sum: u32 = r[..13].iter().map(|b| u32::from(*b)).sum();
            assert_eq!(u32::from(r[13]), sum % 256, "checksum for {v:?}");
        }
        // 42.5 C sums to 286, which wraps to 0x1E
        assert_eq!(encode(&temp_c(42.5)).unwrap()[13], 0x1E);
        assert_eq!(query_report()[13], 0x70);
    }

    #[test]
    fn every_report_is_65_bytes_with_zero_padding() {
        let r = encode(&temp_c(42.5)).unwrap();
        assert_eq!(r.len(), 65);
        assert!(r[14..].iter().all(|b| *b == 0));
        assert_eq!(r[0], 0x00);
    }

    // ------------------------------------------------------------ discover

    #[tokio::test]
    async fn discover_empty_when_absent() {
        let (mut a, _) = rig(false, vec![]);
        assert!(a.discover().await.unwrap().is_empty());
        assert!(matches!(a.connect().await, Err(AdapterError::NotConnected)));
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn discover_reports_one_display_when_present() {
        let (mut a, _) = rig(true, vec![]);
        let d = a.discover().await.unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].id, DeviceId("chione:1b80:b53c".into()));
        assert_eq!(d[0].name, "GAMDIAS CHIONE Digital Display");
        assert_eq!(d[0].digits, 4);
        assert_eq!(d[0].max_decimals, 1);
        assert_eq!(d[0].modes, vec![DisplayMode::Temperature, DisplayMode::FanRpm]);
        assert!(!d[0].connected);
        assert_eq!(a.name(), "chione");
    }

    #[tokio::test]
    async fn discover_does_not_open_the_device() {
        let (mut a, s) = rig(true, vec![]);
        a.discover().await.unwrap();
        assert_eq!(s.lock().unwrap().opens, 0);
        assert!(s.lock().unwrap().writes.is_empty());
    }

    // ------------------------------------------------------------ connect

    #[tokio::test]
    async fn connect_sends_the_query_and_accepts_a_good_reply() {
        // cpuFW=1, tempFW=1 (Celsius start), fanFW=0
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        assert!(a.is_connected());
        let g = s.lock().unwrap();
        assert_eq!(g.opens, 1);
        assert_eq!(g.writes.len(), 1);
        assert_eq!(g.writes[0], query_report().to_vec());
        assert_eq!(g.writes[0].len(), 65);
    }

    #[tokio::test]
    async fn connect_accepts_fahrenheit_start_and_rpm_variant() {
        let (mut a, _) = rig(true, vec![reply(0x01, 0x00, 0, 1, 0)]);
        a.connect().await.unwrap();
        let (mut b, _) = rig(true, vec![reply(0x01, 0x00, 0, 0, 1)]);
        b.connect().await.unwrap();
    }

    #[tokio::test]
    async fn connect_times_out_with_no_reply() {
        // no reply queued: the fake's read returns Ok(0), the same as a hidapi timeout
        let (mut a, s) = rig(true, vec![]);
        let err = a.connect().await.unwrap_err();
        assert!(matches!(err, AdapterError::Protocol(ref m) if m.contains("no reply")), "{err:?}");
        assert!(!a.is_connected());
        // the opened device was dropped, and a later connect opens it again
        assert_eq!(s.lock().unwrap().opens, 1);
    }

    #[tokio::test]
    async fn connect_refuses_a_short_reply() {
        let (mut a, _) = rig(true, vec![vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x01, 0x01, 0x00]]);
        let err = a.connect().await.unwrap_err();
        assert!(matches!(err, AdapterError::Protocol(ref m) if m.contains("garbled")), "{err:?}");
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn connect_refuses_a_garbled_flag_byte() {
        // a flag byte of 7 is not 0 or 1
        let (mut a, _) = rig(true, vec![reply(0x01, 0x00, 7, 1, 0)]);
        assert!(matches!(a.connect().await, Err(AdapterError::Protocol(_))));
        assert!(!a.is_connected());
    }

    #[tokio::test]
    async fn connect_refuses_a_reply_naming_no_variant_or_two() {
        let (mut a, _) = rig(true, vec![reply(0x01, 0x00, 1, 0, 0)]);
        assert!(matches!(a.connect().await, Err(AdapterError::Protocol(_))));
        let (mut b, _) = rig(true, vec![reply(0x01, 0x00, 1, 1, 1)]);
        assert!(matches!(b.connect().await, Err(AdapterError::Protocol(_))));
        assert!(!b.is_connected());
    }

    #[tokio::test]
    async fn connect_missing_device_is_not_connected() {
        let (mut a, s) = rig(false, vec![]);
        assert!(matches!(a.connect().await, Err(AdapterError::NotConnected)));
        assert_eq!(s.lock().unwrap().writes.len(), 0);
    }

    #[tokio::test]
    async fn connect_twice_is_a_no_op() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        a.connect().await.unwrap();
        assert_eq!(s.lock().unwrap().opens, 1);
    }

    #[tokio::test]
    async fn disconnect_drops_the_device() {
        let (mut a, _) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        a.disconnect().await.unwrap();
        assert!(!a.is_connected());
    }

    // ------------------------------------------------------------ write_value

    #[tokio::test]
    async fn write_value_sends_exactly_one_65_byte_report() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        let before = s.lock().unwrap().writes.len();
        a.write_value(&id(), &temp_c(42.5)).await.unwrap();
        let g = s.lock().unwrap();
        assert_eq!(g.writes.len() - before, 1, "exactly one write per value");
        let sent = &g.writes[before];
        assert_eq!(sent.len(), 65);
        assert_eq!(sent.as_slice(), encode(&temp_c(42.5)).unwrap().as_slice());
    }

    #[tokio::test]
    async fn write_value_tolerates_a_missing_ack() {
        // no ack queued: the read times out, the packet was still written, and the adapter stays up
        let (mut a, _) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        a.write_value(&id(), &temp_c(42.5)).await.unwrap();
        assert!(a.is_connected());
    }

    #[tokio::test]
    async fn write_value_discards_the_ack() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0), vec![0xAA; 65]]);
        a.connect().await.unwrap();
        a.write_value(&id(), &temp_c(42.5)).await.unwrap();
        assert!(a.is_connected());
        assert_eq!(s.lock().unwrap().replies.len(), 0, "the ack was read");
    }

    #[tokio::test]
    async fn refused_value_writes_nothing() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        let before = s.lock().unwrap().writes.len();
        let r = a.write_value(&id(), &temp_c(1000.0)).await;
        assert!(matches!(r, Err(AdapterError::Protocol(_))));
        assert_eq!(s.lock().unwrap().writes.len(), before);
        assert!(a.is_connected());
    }

    #[tokio::test]
    async fn write_value_refuses_unknown_device_and_unconnected() {
        let (mut a, _) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        let other = DeviceId("x".into());
        assert!(matches!(a.write_value(&other, &temp_c(1.0)).await, Err(AdapterError::UnknownDevice(_))));
        assert!(matches!(a.write_value(&id(), &temp_c(1.0)).await, Err(AdapterError::NotConnected)));
    }

    #[tokio::test]
    async fn failed_write_drops_the_device() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        s.lock().unwrap().fail_next_write = true;
        assert!(matches!(a.write_value(&id(), &temp_c(42.5)).await, Err(AdapterError::Protocol(_))));
        assert!(!a.is_connected());
        assert!(matches!(a.write_value(&id(), &temp_c(42.5)).await, Err(AdapterError::NotConnected)));
    }

    #[test]
    fn write_interval_is_250_ms() {
        assert_eq!(ChioneDisplay::new().write_interval_ms(), 250);
    }

    #[test]
    fn new_does_not_touch_the_device() {
        let a = ChioneDisplay::new();
        assert!(!a.is_connected());
        assert_eq!(a.name(), "chione");
    }
}
