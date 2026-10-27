//! Native adapter for the GAMDIAS CHIONE Digital Display (USB HID 1B80:B53C), the pump-head
//! display that shows one four-digit number. The protocol is the one decoded from ZEUS CAST in
//! `.build/chione/recon.md`; section numbers below refer to that file.
//!
//! Structure follows `royuan.rs`: a private `Transport` trait over hidapi, an `Opener` that finds
//! the device by VID/PID, and a fake transport for the tests. The tests never touch the real
//! device; only `ChioneDisplay::new()` uses the hidapi opener, and nothing in this crate calls
//! `discover` or `connect` on it except the app when it is asked to.
//!
//! # Only ZEUS CAST's packets
//!
//! Until the first hardware test proves otherwise, this adapter sends only packets ZEUS CAST itself
//! sends: the 0xFF query once per connection, then 0x01 display packets whose digits, dot, unit, CPU
//! and mode bytes match what ZEUS CAST builds (recon section 2; `SendOutputData` blanks digit 1
//! alone when it is zero, in the temperature path and in the RPM path). Nothing else is sent: no
//! all-blank packet, no minus sign, no whole-number temperature layout without the dot.
//!
//! `DisplayValue { value: None }` means "blank" in the spine. ZEUS CAST never sends a blank, and
//! recon section 4 marks a fully blank display UNKNOWN, so `write_value` returns
//! `AdapterError::Unsupported` for it and writes nothing. The caller holds the last value instead.
//! This stays until the hardware test proves a blank packet is safe (recon test step 3).
//!
//! # Threads
//!
//! hidapi calls block (an open or a write can take up to a second, the ack read 100 ms, and the
//! handshake waits 500 ms). Every one of them runs on tokio's blocking pool through
//! `spawn_blocking`, never on a runtime worker. The device handle moves into the blocking closure
//! and back out; if the awaiting future is dropped mid-call, the handle is dropped with it and the
//! adapter reads as disconnected.
//!
//! # Enumeration (`discover`)
//!
//! `discover` enumerates every HID device on the machine with hidapi (`HidApi::new`, which reads
//! metadata for each device; it opens none and does no I/O). The app calls it only once a display
//! has a saved config, so a machine without a configured display never enumerates.

use async_trait::async_trait;
use canvas_proto::device::DeviceId;
use canvas_proto::display::{DisplayInfo, DisplayMode, DisplayUnit, DisplayValue};
use canvas_proto::traits::{AdapterError, NumericDisplayAdapter};
use std::sync::Arc;
use std::time::Duration;

pub const VENDOR_ID: u16 = 0x1B80;
pub const PRODUCT_ID: u16 = 0xB53C;
const DISPLAY_NAME: &str = "GAMDIAS CHIONE Digital Display";
/// The output report: report ID 0 plus 64 bytes (recon section 1: OutputReportByteLength 65).
const REPORT_LEN: usize = 65;
/// The input report as Windows hidapi returns it: the leading report ID 0x00 is stripped on read
/// (hidapi-2.6.7 `src/windows_native/mod.rs`), so the 65-byte input report arrives as 64 bytes.
const INPUT_LEN: usize = 64;
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
/// ASSUMED. Recon says ZEUS CAST reads one input report after every display write, and does not
/// inspect it. The read here is timed so a silent device cannot stall the 250 ms loop. A timeout
/// on this read is not an error: the packet was already written.
const ACK_TIMEOUT_MS: i32 = 100;

fn device_id() -> DeviceId {
    DeviceId(format!("chione:{VENDOR_ID:04x}:{PRODUCT_ID:04x}"))
}

fn refuse(msg: impl Into<String>) -> AdapterError {
    AdapterError::Protocol(msg.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
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

/// The four digit bytes for `n` (0..=9999), ZEUS CAST's way: every position shows its digit, and
/// only digit 1 is blanked when it is zero. So 5.5 (n = 55) is `20 00 05 05`, never `20 20 05 05`,
/// and 0 is `20 00 00 00`. Recon section 2: `SendOutputData` stores 32 in digit 1 alone, in the
/// temperature path and in the RPM path.
fn digits_for(n: u32) -> [u8; DIGITS] {
    let mut out = [(n / 1000 % 10) as u8, (n / 100 % 10) as u8, (n / 10 % 10) as u8, (n % 10) as u8];
    if out[0] == 0 {
        out[0] = BLANK;
    }
    out
}

/// Encodes one display value as the 65-byte output report (recon section 2).
///
/// The value is already in its own unit: a Fahrenheit value is sent as given (the host converts
/// before it gets here). A value that cannot be shown is refused, never truncated.
///
/// Temperature mode always uses ZEUS CAST's `xxx.x` layout with the dot on: a whole-number
/// temperature (0 decimals) is sent as `n.0`, because ZEUS CAST never sends a temperature without
/// the dot and what the device does with dot = 0 in temperature mode is UNKNOWN. RPM mode is the
/// whole-number layout with no dot.
///
/// `value: None` (blank) returns `AdapterError::Unsupported`: ZEUS CAST never sends a blank packet.
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
    let Some(v) = value.value else {
        return Err(AdapterError::Unsupported(
            "a blank display packet is never sent; ZEUS CAST sends none and blanking is unproven on hardware",
        ));
    };
    if !v.is_finite() {
        return Err(refuse(format!("{v} is not a finite number")));
    }
    // UNKNOWN (recon section 4): the device's minus sign. ZEUS CAST has no negative path, so
    // the conservative choice is to refuse rather than wrap the value through a byte.
    if v < 0.0 {
        return Err(refuse(format!("{v} is negative; the CHIONE's minus sign is unknown")));
    }
    // Tenths for a temperature (42.5 is 425; a whole 7 is 70, shown 7.0), whole units for RPM.
    let scaled = if !temperature {
        f64::from(v).round()
    } else if value.decimals == 0 {
        f64::from(v).round() * 10.0
    } else {
        (f64::from(v) * 10.0).round()
    };
    if scaled > 9999.0 {
        return Err(refuse(format!(
            "{v} with {} decimal places does not fit four digits",
            value.decimals
        )));
    }
    let digits = digits_for(scaled as u32);

    let mut r = [0u8; REPORT_LEN];
    r[..3].copy_from_slice(&HEADER);
    r[3] = CMD_DISPLAY;
    r[4..8].copy_from_slice(&digits);
    // Byte 8: the dot before the last digit in temperature mode; none in RPM mode.
    r[8] = u8::from(temperature);
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

/// An open handle: one 65-byte write and one timed read.
trait Transport: Send + Sync {
    /// Writes one output report (byte 0 is the report ID). Returns the bytes written.
    fn write(&mut self, data: &[u8]) -> Result<usize, String>;
    /// Reads one input report into `buf`. `Ok(0)` means nothing arrived within `timeout_ms`.
    /// On Windows hidapi the leading report ID is stripped, so a full report is 64 bytes.
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

/// Offsets of the reply fields in the 64-byte stripped form: recon's 65-byte indices minus the
/// report ID byte. Recon: `[4]` hFW, `[5]` lFW, `[6]` tempFW, `[7]` cpuFW, `[8]` fanFW.
const R_HFW: usize = 3;
const R_LFW: usize = 4;
const R_TEMP_FW: usize = 5;
const R_CPU_FW: usize = 6;
const R_FAN_FW: usize = 7;

/// Reduces a reply to the 64-byte stripped form. Windows hidapi strips the leading report ID, so
/// 64 bytes is what arrives. A platform that keeps it hands over 65 bytes starting with 0x00; that
/// form is accepted too. Anything else is garbled.
fn stripped(reply: &[u8]) -> Option<&[u8]> {
    match reply.len() {
        INPUT_LEN => Some(reply),
        REPORT_LEN if reply[0] == 0x00 => Some(&reply[1..]),
        _ => None,
    }
}

/// The fields of a 0xFF query reply, as the adapter reads them (recon section 2).
///
/// UNKNOWN (recon section 2): the values this unit reports, and whether the real layout matches
/// recon once the report ID is gone. The first hardware test settles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryReply {
    /// The firmware version, high byte.
    pub hfw: u8,
    /// The firmware version, low byte.
    pub lfw: u8,
    /// 1 when the unit starts in Fahrenheit.
    pub temp_fw: u8,
    /// 1 when the unit reports the CPU temperature.
    pub cpu_fw: u8,
    /// 1 when the unit is the fan-RPM variant.
    pub fan_fw: u8,
}

/// Parses the 0xFF reply (recon section 2): `hFW lFW tempFW cpuFW fanFW`, read from the stripped
/// form.
///
/// The recon gives three variants: cpuFW=1 with tempFW=1 (Celsius start), cpuFW=1 with tempFW=0
/// (Fahrenheit start), fanFW=1 (RPM variant). Anything else is refused as garbled, with the raw
/// bytes in the message, so the first hardware test can see them.
pub fn parse_query_reply(reply: &[u8]) -> Result<QueryReply, AdapterError> {
    if reply.is_empty() {
        return Err(refuse("no reply to the CHIONE 0xFF query within 1000 ms"));
    }
    let Some(r) = stripped(reply) else {
        return Err(refuse(format!(
            "garbled CHIONE query reply: {} bytes, expected {INPUT_LEN} (or {REPORT_LEN} starting 00): {}",
            reply.len(),
            hex(reply)
        )));
    };
    let (temp_fw, cpu_fw, fan_fw) = (r[R_TEMP_FW], r[R_CPU_FW], r[R_FAN_FW]);
    let flags_ok = [temp_fw, cpu_fw, fan_fw].iter().all(|f| *f <= 1);
    if !flags_ok || cpu_fw == fan_fw {
        return Err(refuse(format!(
            "garbled CHIONE query reply: tempFW={temp_fw} cpuFW={cpu_fw} fanFW={fan_fw}: {}",
            hex(reply)
        )));
    }
    Ok(QueryReply { hfw: r[R_HFW], lfw: r[R_LFW], temp_fw, cpu_fw, fan_fw })
}

/// Checks the 0xFF reply. Same rules as [`parse_query_reply`], and the fields are logged.
fn check_query_reply(reply: &[u8]) -> Result<(), AdapterError> {
    let parsed = parse_query_reply(reply)?;
    tracing::info!(
        hfw = parsed.hfw,
        lfw = parsed.lfw,
        temp_fw = parsed.temp_fw,
        cpu_fw = parsed.cpu_fw,
        fan_fw = parsed.fan_fw,
        "CHIONE query reply"
    );
    Ok(())
}

/// Opens the device and runs the 0xFF handshake, without checking the reply. Blocking: call it
/// from `spawn_blocking`. Returns the open device and the raw reply bytes.
fn open_and_query(
    opener: &dyn Opener,
    wait: Duration,
) -> Result<(Box<dyn Transport>, Vec<u8>), AdapterError> {
    let mut dev = opener.open().map_err(AdapterError::Protocol)?.ok_or(AdapterError::NotConnected)?;
    let sent = dev
        .write(&query_report())
        .map_err(|e| refuse(format!("CHIONE query write failed: {e}")))?;
    if sent != REPORT_LEN {
        return Err(refuse(format!("CHIONE query: short write of {sent} bytes")));
    }
    std::thread::sleep(wait);
    let mut reply = [0u8; REPORT_LEN];
    let n = dev
        .read(&mut reply, QUERY_REPLY_TIMEOUT_MS)
        .map_err(|e| refuse(format!("CHIONE query read failed: {e}")))?;
    // Every connect logs the raw reply at info, so the first hardware run shows the real layout.
    let raw = hex(&reply[..n]);
    tracing::info!(bytes = n, raw = %raw, "CHIONE raw query reply");
    Ok((dev, reply[..n].to_vec()))
}

/// Opens the device and runs the 0xFF handshake. Blocking: call it from `spawn_blocking`.
/// Any failure is an error and the opened device is dropped.
fn connect_blocking(opener: &dyn Opener, wait: Duration) -> Result<Box<dyn Transport>, AdapterError> {
    let (dev, reply) = open_and_query(opener, wait)?;
    // A garbled reply is logged again, with the reason, before the error goes back.
    if let Err(e) = check_query_reply(&reply) {
        tracing::info!(bytes = reply.len(), raw = %hex(&reply), "CHIONE query reply refused: {e}");
        return Err(e);
    }
    Ok(dev)
}

/// One display write and the ack read. Blocking: call it from `spawn_blocking`.
/// Returns the ack the device sent back, if any.
fn write_blocking(
    dev: &mut dyn Transport,
    report: &[u8; REPORT_LEN],
) -> Result<Option<Vec<u8>>, AdapterError> {
    match dev.write(report) {
        Ok(REPORT_LEN) => {}
        Ok(sent) => return Err(refuse(format!("CHIONE display write: short write of {sent} bytes"))),
        Err(e) => return Err(refuse(format!("CHIONE display write failed: {e}"))),
    }
    // ZEUS CAST reads one input report after every write. Read it here too, so the device's
    // input queue does not fill, and discard it.
    let mut ack = [0u8; REPORT_LEN];
    match dev.read(&mut ack, ACK_TIMEOUT_MS) {
        Ok(0) => {
            tracing::debug!("CHIONE: no ack within {ACK_TIMEOUT_MS} ms");
            Ok(None)
        }
        Ok(n) => {
            tracing::debug!(bytes = n, ack = %hex(&ack[..n]), "CHIONE ack");
            Ok(Some(ack[..n].to_vec()))
        }
        Err(e) => Err(refuse(format!("CHIONE ack read failed: {e}"))),
    }
}

fn join_error(e: tokio::task::JoinError) -> AdapterError {
    refuse(format!("CHIONE blocking task failed: {e}"))
}

// ---------------------------------------------------------------- adapter

pub struct ChioneDisplay {
    opener: Arc<dyn Opener>,
    dev: Option<Box<dyn Transport>>,
    /// Wait between the query and its reply read. `QUERY_WAIT` in production; tests use zero.
    query_wait: Duration,
    /// The ack the device sent back after the last write, if any.
    last_ack: Option<Vec<u8>>,
}

impl Default for ChioneDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl ChioneDisplay {
    pub fn new() -> Self {
        Self::with_opener(Arc::new(HidOpener), QUERY_WAIT)
    }

    fn with_opener(opener: Arc<dyn Opener>, query_wait: Duration) -> Self {
        Self { opener, dev: None, query_wait, last_ack: None }
    }

    /// Opens the device, sends the 0xFF query and reads the reply, and returns the raw reply
    /// bytes.
    ///
    /// The reply is **not** checked: this is the probe's path, and the reply layout on the real
    /// unit is still a guess (recon section 2), so the raw bytes are what the first hardware test
    /// needs. `connect` runs the same handshake and refuses a reply it cannot read.
    ///
    /// The device stays open, so the caller can write to it, and must disconnect it.
    pub async fn query_raw(&mut self) -> Result<Vec<u8>, AdapterError> {
        let opener = self.opener.clone();
        let wait = self.query_wait;
        let (dev, reply) = tokio::task::spawn_blocking(move || open_and_query(opener.as_ref(), wait))
            .await
            .map_err(join_error)??;
        self.dev = Some(dev);
        Ok(reply)
    }

    /// The ack the device sent back after the last write, if it sent one.
    pub fn last_ack(&self) -> Option<&[u8]> {
        self.last_ack.as_deref()
    }

    /// Writes exactly these 65 bytes and reads the ack, as `write_value` does after it encodes.
    ///
    /// This is the probe's path for packets that `encode` would not build (the dot test). The
    /// daemon never calls it: `write_value` is still the only path it uses, and it encodes first.
    /// Refuses with `NotConnected` when the device is not open. A failed write drops the device.
    pub async fn send_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), AdapterError> {
        let Some(mut dev) = self.dev.take() else {
            return Err(AdapterError::NotConnected);
        };
        let report = *report;
        let (dev, result) = tokio::task::spawn_blocking(move || {
            let result = write_blocking(dev.as_mut(), &report);
            (dev, result)
        })
        .await
        .map_err(join_error)?;
        // A failed write or ack read drops the device: `self.dev` stays `None`.
        let ack = result?;
        self.dev = Some(dev);
        self.last_ack = ack;
        Ok(())
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
        let opener = self.opener.clone();
        let wait = self.query_wait;
        // On error the device is dropped inside the closure, so a failed handshake leaves the
        // adapter unconnected.
        let dev = tokio::task::spawn_blocking(move || connect_blocking(opener.as_ref(), wait))
            .await
            .map_err(join_error)??;
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
        let present = if self.dev.is_some() {
            true
        } else {
            // Enumerates every HID device with hidapi (metadata only, no I/O), off the runtime.
            let opener = self.opener.clone();
            tokio::task::spawn_blocking(move || opener.present()).await.map_err(join_error)?
        };
        if present {
            Ok(vec![self.display_info()])
        } else {
            Ok(Vec::new())
        }
    }

    async fn write_value(&mut self, device: &DeviceId, value: &DisplayValue) -> Result<(), AdapterError> {
        self.check_id(device)?;
        // Encode first: a value that cannot be shown (or a blank, which is never sent) is refused
        // before anything is written, and the connection stays up.
        let report = encode(value)?;
        self.send_report(&report).await
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
        /// Blocks the calling thread this long in open, write and read (a slow device).
        delay_ms: u64,
        /// Hand replies over with the report ID byte, as a platform that keeps it would.
        keep_report_id: bool,
    }

    fn slow(s: &Shared) {
        if s.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(s.delay_ms));
        }
    }

    struct FakeOpener(Arc<Mutex<Shared>>);
    struct FakeTransport(Arc<Mutex<Shared>>);

    impl Transport for FakeTransport {
        fn write(&mut self, data: &[u8]) -> Result<usize, String> {
            let mut s = self.0.lock().unwrap();
            slow(&s);
            if s.fail_next_write {
                s.fail_next_write = false;
                return Err("unplugged".into());
            }
            s.writes.push(data.to_vec());
            Ok(data.len())
        }

        /// Like Windows hidapi: a report arrives without its leading report ID byte, so a full
        /// 65-byte input report is 64 bytes. The queue holds what the device sent (65 bytes with
        /// the ID, as recon lays it out); the fake strips the ID byte when it is 0x00 and the
        /// entry is 65 bytes long. An entry of any other length is handed over as it is.
        fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            let mut s = self.0.lock().unwrap();
            slow(&s);
            match s.replies.pop_front() {
                None => Ok(0),
                Some(mut reply) => {
                    if s.keep_report_id {
                        // a platform that keeps the ID: hand the entry over untouched
                    } else if reply.len() == REPORT_LEN && reply[0] == 0 {
                        reply.remove(0);
                    }
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
            slow(&s);
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
            ChioneDisplay::with_opener(Arc::new(FakeOpener(shared.clone())), Duration::ZERO),
            shared,
        )
    }

    /// A reply in recon's 65-byte layout (byte 0 is the report ID), with the given firmware and
    /// variant bytes. The fake transport strips byte 0 on read, as Windows hidapi does.
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
    fn golden_7_with_no_decimals_is_7_point_0() {
        // ZEUS CAST never sends a temperature without the dot, so a whole 7 is 007.0:
        // 20 00 07 00; dot 1; unit C 1; cpu 1; sum 282 -> 0x1A
        let v = value(Some(7.0), 0, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x00, 0x07, 0x00, 0x01, 0x01, 0x01, 0x00, 0x00, 0x1A]);
        assert_eq!(encode(&v).unwrap(), want);
    }

    #[test]
    fn golden_5_5_celsius_keeps_the_zero_in_digit_2() {
        // ZEUS CAST blanks digit 1 only: 5.5 C is 20 00 05 05, not 20 20 05 05. Sum 285 -> 0x1D.
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x00, 0x05, 0x05, 0x01, 0x01, 0x01, 0x00, 0x00, 0x1D]);
        assert_eq!(encode(&temp_c(5.5)).unwrap(), want);
    }

    #[test]
    fn golden_0_0_celsius() {
        // 20 00 00 00; sum 275 -> 0x13
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x00, 0x13]);
        assert_eq!(encode(&temp_c(0.0)).unwrap(), want);
    }

    #[test]
    fn golden_rpm_850_blanks_digit_1_only() {
        // 850 rpm: 0,8,5,0 -> 20 08 05 00; no dot; unit 0; cpu 0; mode 1; sum 286 -> 0x1E
        let v = value(Some(850.0), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, false);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x08, 0x05, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x1E]);
        assert_eq!(encode(&v).unwrap(), want);
    }

    #[test]
    fn golden_rpm_12_keeps_the_zero_in_digit_2() {
        // 12 rpm: 0,0,1,2 -> 20 00 01 02; sum 276 -> 0x14
        let v = value(Some(12.0), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, false);
        let want = expect(&[0x00, 0x3A, 0xB5, 0x01, 0x20, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x14]);
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
    fn a_blank_is_never_encoded() {
        // ZEUS CAST sends no blank packet, so None is Unsupported in either mode.
        let t = value(None, 1, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        assert!(matches!(encode(&t), Err(AdapterError::Unsupported(_))));
        let r = value(None, 0, DisplayUnit::Celsius, DisplayMode::FanRpm, true);
        assert!(matches!(encode(&r), Err(AdapterError::Unsupported(_))));
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
    fn only_digit_1_is_ever_blank() {
        // 0.0 is 20 00 00 00; 0.5 is 20 00 00 05; whole zero is 20 00 00 00 with the dot on
        assert_eq!(&encode(&temp_c(0.0)).unwrap()[4..8], &[0x20, 0x00, 0x00, 0x00]);
        assert_eq!(&encode(&temp_c(0.5)).unwrap()[4..8], &[0x20, 0x00, 0x00, 0x05]);
        let v = value(Some(0.0), 0, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        let r = encode(&v).unwrap();
        assert_eq!(&r[4..8], &[0x20, 0x00, 0x00, 0x00]);
        assert_eq!(r[8], 1, "the temperature layout always carries the dot");
        // digit 1 is a digit once the value reaches 100.0
        assert_eq!(&encode(&temp_c(100.0)).unwrap()[4..8], &[0x01, 0x00, 0x00, 0x00]);
        // no digit but the first is ever 0x20, across a sweep of both layouts
        for n in 0..=9999u32 {
            let t = encode(&value(Some(n as f32 / 10.0), 1, DisplayUnit::Celsius, DisplayMode::Temperature, false)).unwrap();
            let f = encode(&value(Some(n as f32), 0, DisplayUnit::Celsius, DisplayMode::FanRpm, false)).unwrap();
            for r in [t, f] {
                assert!(r[5..8].iter().all(|d| *d <= 9), "n = {n}: {:?}", &r[4..8]);
                assert!(r[4] <= 9 || r[4] == 0x20);
                assert_eq!(r[4] == 0x20, n < 1000, "n = {n}");
            }
        }
    }

    #[test]
    fn rounding_to_decimals() {
        // 42.46 rounds to 42.5, 42.44 rounds to 42.4
        assert_eq!(&encode(&temp_c(42.46)).unwrap()[4..8], &[0x20, 0x04, 0x02, 0x05]);
        assert_eq!(&encode(&temp_c(42.44)).unwrap()[4..8], &[0x20, 0x04, 0x02, 0x04]);
        // whole numbers round to the nearest integer, then show as n.0
        let v = value(Some(7.6), 0, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        assert_eq!(&encode(&v).unwrap()[4..8], &[0x20, 0x00, 0x08, 0x00]);
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
            value(Some(98.6), 1, DisplayUnit::Fahrenheit, DisplayMode::Temperature, true),
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
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0), vec![0xAA; 64]]);
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

    // ------------------------------------------------------------ F1: the stripped reply

    #[tokio::test]
    async fn connect_accepts_the_64_byte_reply_windows_hidapi_returns() {
        // 64 bytes, report ID already gone: hFW at [3], lFW [4], tempFW [5], cpuFW [6], fanFW [7]
        let mut r = vec![0u8; 64];
        r[3] = 0x01;
        r[4] = 0x07;
        r[5] = 1;
        r[6] = 1;
        let (mut a, _) = rig(true, vec![r]);
        a.connect().await.unwrap();
        assert!(a.is_connected());
    }

    #[tokio::test]
    async fn the_fake_strips_the_report_id_like_windows_hidapi() {
        // the 65-byte recon-layout reply reaches the adapter as 64 bytes
        let (_, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        let mut t = FakeTransport(s);
        let mut buf = [0u8; REPORT_LEN];
        assert_eq!(t.read(&mut buf, 0).unwrap(), 64);
        assert_eq!(&buf[3..8], &[0x01, 0x07, 1, 1, 0]);
    }

    #[tokio::test]
    async fn connect_accepts_a_65_byte_reply_that_keeps_the_report_id() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        s.lock().unwrap().keep_report_id = true;
        a.connect().await.unwrap();
        assert!(a.is_connected());
    }

    #[tokio::test]
    async fn connect_refuses_a_65_byte_reply_not_starting_with_00() {
        let mut r = reply(0x01, 0x07, 1, 1, 0);
        r[0] = 0x05;
        let (mut a, s) = rig(true, vec![r]);
        s.lock().unwrap().keep_report_id = true;
        assert!(matches!(a.connect().await, Err(AdapterError::Protocol(ref m)) if m.contains("garbled")));
    }

    #[tokio::test]
    async fn the_old_65_byte_offsets_no_longer_pass() {
        // flags at recon's [6..8] of a 64-byte reply would be byte 6 = cpuFW=1 and byte 7 = fanFW=1
        // read as fanFW: with the fixed offsets that is tempFW=0 cpuFW=1 fanFW=1, two variants
        let mut r = vec![0u8; 64];
        r[6] = 1;
        r[7] = 1;
        let (mut a, _) = rig(true, vec![r]);
        assert!(matches!(a.connect().await, Err(AdapterError::Protocol(_))));
    }

    #[tokio::test]
    async fn a_garbled_reply_error_carries_the_raw_bytes_in_hex() {
        let mut r = vec![0u8; 64];
        r[5] = 7;
        r[6] = 1;
        let (mut a, _) = rig(true, vec![r]);
        let err = a.connect().await.unwrap_err();
        assert!(matches!(&err, AdapterError::Protocol(m) if m.contains("00 00 00 00 00 07 01 00")), "{err:?}");
        let short = check_query_reply(&[0xDE, 0xAD]).unwrap_err();
        assert!(matches!(&short, AdapterError::Protocol(m) if m.contains("de ad")), "{short:?}");
        assert_eq!(hex(&[0x00, 0x3a, 0xff]), "00 3a ff");
    }

    // ------------------------------------------------------------ the probe's path

    #[test]
    fn the_reply_fields_are_read_from_the_stripped_form() {
        let mut r = vec![0u8; 64];
        r[3] = 0x01; // hFW
        r[4] = 0x07; // lFW
        r[5] = 0; // tempFW: the unit starts in Celsius
        r[6] = 1; // cpuFW
        r[7] = 0; // fanFW
        let parsed = parse_query_reply(&r).unwrap();
        assert_eq!(parsed, QueryReply { hfw: 1, lfw: 7, temp_fw: 0, cpu_fw: 1, fan_fw: 0 });
    }

    #[tokio::test]
    async fn query_raw_gives_the_raw_reply_even_when_the_adapter_would_refuse_it() {
        // A reply the adapter refuses (two variants at once): the probe still gets the bytes,
        // because the real layout is still a guess and the first hardware test needs to see it.
        let mut r = vec![0u8; 64];
        r[6] = 1;
        r[7] = 1;
        let (mut a, s) = rig(true, vec![r]);
        let reply = a.query_raw().await.unwrap();
        assert_eq!(reply.len(), 64);
        assert!(parse_query_reply(&reply).is_err(), "the adapter refuses this reply");
        assert!(a.is_connected(), "the device stays open, so the probe can write to it");
        assert_eq!(s.lock().unwrap().opens, 1);
        let _ = a.disconnect().await;
    }

    #[tokio::test]
    async fn the_ack_the_device_sent_is_kept_for_the_probe() {
        let (mut a, _) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0), vec![0xAB; 8]]);
        a.connect().await.unwrap();
        assert!(a.last_ack().is_none(), "no write yet");
        a.write_value(&id(), &temp_c(42.5)).await.unwrap();
        assert_eq!(a.last_ack().map(|bytes| bytes.to_vec()), Some(vec![0xAB; 8]));
    }

    // ------------------------------------------------------------ F2: no blank packet

    #[tokio::test]
    async fn a_blank_value_writes_nothing_and_keeps_the_connection() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        let before = s.lock().unwrap().writes.len();
        let blank = value(None, 1, DisplayUnit::Celsius, DisplayMode::Temperature, false);
        assert!(matches!(a.write_value(&id(), &blank).await, Err(AdapterError::Unsupported(_))));
        assert_eq!(s.lock().unwrap().writes.len(), before, "no packet for a blank");
        assert!(a.is_connected());
    }

    #[tokio::test]
    async fn every_packet_sent_is_the_query_or_a_display_packet_with_a_digit_1_only_blank() {
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        a.connect().await.unwrap();
        for v in [temp_c(5.5), temp_c(0.0), temp_c(123.4)] {
            a.write_value(&id(), &v).await.unwrap();
        }
        let g = s.lock().unwrap();
        for w in &g.writes[1..] {
            assert_eq!(w[3], 0x01, "only the display command");
            assert!(w[5..8].iter().all(|d| *d <= 9), "{:?}", &w[4..8]);
        }
    }

    // ------------------------------------------------------------ F3: off the runtime thread

    #[tokio::test(flavor = "current_thread")]
    async fn device_io_does_not_block_the_runtime_thread() {
        // On a one-thread runtime, a blocking hidapi call on that thread would starve the ticker.
        // Every fake call sleeps 60 ms on its thread: open, write, read, then write and ack read.
        let (mut a, s) = rig(true, vec![reply(0x01, 0x07, 1, 1, 0)]);
        s.lock().unwrap().delay_ms = 60;
        let ticks = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let t = ticks.clone();
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                t.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        a.connect().await.unwrap();
        a.write_value(&id(), &temp_c(42.5)).await.unwrap();
        a.discover().await.unwrap();
        ticker.abort();
        let n = ticks.load(std::sync::atomic::Ordering::SeqCst);
        // 5 blocking calls of 60 ms (>= 300 ms); a blocked runtime thread would tick about 0 times (Windows timers tick at about 15 ms, so 300 ms gives about 20)
        assert!(n >= 10, "the ticker ran {n} times while the device was busy");
    }
}
