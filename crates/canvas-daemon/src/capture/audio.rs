//! Audio Spectrum input: WASAPI loopback on the default render endpoint, an FFT, and
//! ~32 log-spaced bands between 30 Hz and 16 kHz.
//!
//! The band mapping is pure and unit-tested: a synthetic 1 kHz sine must make the band
//! whose centre is nearest 1 kHz the largest.

use canvas_proto::traits::AudioSpectrum;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::sync::Arc;

/// The lowest band centre.
pub const BAND_MIN_HZ: f32 = 30.0;
/// The highest band centre.
pub const BAND_MAX_HZ: f32 = 16_000.0;
/// How many bands the spectrum has.
pub const BAND_COUNT: usize = 32;
/// The capture sample rate. 48 kHz is what every render endpoint uses.
pub const SAMPLE_RATE: u32 = 48_000;
/// The FFT size. 2048 bins at 48 kHz gives ~23 Hz per bin.
pub const FFT_SIZE: usize = 2048;
/// The dB floor a band level is mapped from. Spectrum visualisers work on a
/// logarithmic (dB) scale with a floor around -60 dBFS so that typical music
/// at about -20 dBFS lands at two thirds of the bar instead of a tenth.
pub const DB_FLOOR: f32 = -60.0;
/// The dB ceiling a band level is mapped to: full scale.
pub const DB_CEILING: f32 = 0.0;
/// Bytes of WAVEFORMATEXTENSIBLE beyond WAVEFORMATEX (40 - 18). The mix
/// format's `cbSize` counts these bytes, so an extensible mix reports 22,
/// not the 40 bytes of the whole struct.
pub const EXTENSIBLE_EXTRA_BYTES: u16 = 22;

/// Whether a mix format with this `cbSize` carries the extensible extension.
pub fn extensible_has_extension(cb_size: u16) -> bool {
    cb_size >= EXTENSIBLE_EXTRA_BYTES
}

/// Maps a linear band magnitude (1.0 is full scale) onto 0..1 on a dB scale
/// between [`DB_FLOOR`] and [`DB_CEILING`]: -20 dBFS is 2/3, -30 dBFS is 1/2,
/// a full-scale sine is 1.0, and silence (or anything at or below the floor)
/// is 0.0. A value that is not a finite positive number (NaN, negative) is 0.0 too, so one bad
/// sample can never publish a NaN that freezes a bar.
pub fn db_mapped(linear: f32) -> f32 {
    if linear.is_nan() || linear <= 0.0 {
        return 0.0;
    }
    if linear.is_infinite() {
        return 1.0;
    }
    let db = 20.0 * linear.log10();
    ((db - DB_FLOOR) / (DB_CEILING - DB_FLOOR)).clamp(0.0, 1.0)
}

/// The band centre frequencies, log-spaced from 30 Hz to 16 kHz.
pub fn band_centers() -> Vec<f32> {
    let mut centers = Vec::with_capacity(BAND_COUNT);
    let log_min = BAND_MIN_HZ.ln();
    let log_max = BAND_MAX_HZ.ln();
    for i in 0..BAND_COUNT {
        let t = i as f32 / (BAND_COUNT - 1) as f32;
        centers.push((log_min + t * (log_max - log_min)).exp());
    }
    centers
}

/// Which band a frequency falls in: the nearest centre.
pub fn nearest_band(centers: &[f32], hz: f32) -> usize {
    let mut best = 0usize;
    let mut best_distance = f32::INFINITY;
    for (i, c) in centers.iter().enumerate() {
        let d = (c - hz).abs();
        if d < best_distance {
            best_distance = d;
            best = i;
        }
    }
    best
}

/// Turns a block of mono samples into band levels in 0..1 on a dB scale.
///
/// `samples` is the most recent FFT window. The window is Hann-windowed to cut the
/// leakage that would otherwise smear a tone across neighbouring bands.
///
/// Each band owns the bins between the midpoints to its neighbours, so a tone lands in
/// exactly one band and that band is the largest. The FFT peak is normalised so a
/// full-scale sine reads 1.0 linear, then mapped through [`db_mapped`]: linear
/// magnitudes are unusable as bar heights because typical music peaks around
/// -20 dBFS (0.1 linear), which would fill only a tenth of the canvas and light
/// just the bottom rows. See #188.
pub fn magnitudes(samples: &[f32], centers: &[f32]) -> Vec<f32> {
    magnitudes_with_rate(samples, centers, SAMPLE_RATE)
}

/// Turns a block of mono samples into band levels, at the mix format's own sample rate.
///
/// The band edges move with the rate: the bin width is `sample_rate / size`, so a
/// 44.1 kHz mix and a 48 kHz mix put the same tone in the same band only when the
/// rate is the one the samples actually came in at.
pub fn magnitudes_with_rate(samples: &[f32], centers: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() || centers.is_empty() {
        return vec![0.0; centers.len()];
    }
    let size = samples.len().next_power_of_two().max(64);

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let mut buffer: Vec<Complex<f32>> = samples
        .iter()
        .take(size)
        .map(|s| Complex::new(*s, 0.0))
        .collect();
    buffer.resize(size, Complex::new(0.0, 0.0));

    // Hann window.
    for (i, c) in buffer.iter_mut().enumerate() {
        let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / size as f32).cos());
        c.re *= w;
        c.im *= 0.0;
    }

    fft.process(&mut buffer);

    let bin_hz = sample_rate.max(1) as f32 / size as f32;
    let half = size / 2;
    // A Hann-windowed full-scale sine peaks at `size / 4` in one bin.
    let gain = size as f32 * 0.25;

    // The band edges: the midpoint between two neighbouring centres.
    let mut edges = Vec::with_capacity(centers.len() + 1);
    edges.push(0.0f32);
    for pair in centers.windows(2) {
        edges.push((pair[0] + pair[1]) / 2.0);
    }
    edges.push(f32::INFINITY);

    let mut mags = Vec::with_capacity(centers.len());
    for index in 0..centers.len() {
        let low = edges[index];
        let high = edges[index + 1];
        let mut peak = 0.0f32;
        for (bin, value) in buffer.iter().enumerate().take(half).skip(1) {
            let hz = bin as f32 * bin_hz;
            if hz >= low && hz < high {
                peak = peak.max(value.norm());
            }
        }
        mags.push(db_mapped(peak / gain));
    }
    mags
}

/// Builds a spectrum from raw samples.
pub fn spectrum_from_samples(samples: &[f32]) -> AudioSpectrum {
    let centers = band_centers();
    let magnitudes = magnitudes(samples, &centers);
    AudioSpectrum { centers_hz: centers, magnitudes }
}

/// The endpoint the capture listens to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointInfo {
    /// The endpoint's friendly name, as Windows shows it ("Speakers (Realtek Audio)").
    pub name: String,
    /// The endpoint's id, as Windows names it.
    pub id: String,
}

/// What the samples in the mix are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// Integer samples.
    Pcm,
    /// 32-bit float samples.
    Float,
    /// Something else: the format tag names it.
    Other,
}

impl SampleFormat {
    /// The name the log uses.
    pub fn name(&self) -> &'static str {
        match self {
            SampleFormat::Pcm => "PCM",
            SampleFormat::Float => "IEEE float",
            SampleFormat::Other => "unknown",
        }
    }
}

/// The mix format the endpoint gave the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixFormat {
    /// Frames a second.
    pub sample_rate: u32,
    /// Channels in the mix.
    pub channels: u16,
    /// Bits a sample occupies in the buffer (the container width).
    pub bits_per_sample: u16,
    /// Valid bits within the container (S6): 24 valid bits in a 32-bit
    /// container for 24-in-32 extensible feeds. Equals `bits_per_sample`
    /// for packed formats.
    pub valid_bits_per_sample: u16,
    /// The format tag (`1` is PCM, `3` is IEEE float, `0xFFFE` is extensible).
    pub format_tag: u16,
    /// What the samples are.
    pub sample_format: SampleFormat,
}

/// The line the daemon logs when it opens the audio endpoint.
///
/// The endpoint's name and its mix format are the two things a reader needs to know
/// which device the audio comes from and how its samples are shaped (#1).
pub fn mix_format_line(endpoint: &EndpointInfo, format: &MixFormat) -> String {
    format!(
        "the audio capture opened {name} ({id}): {rate} Hz, {channels} channels, {bits} bits ({valid} valid), {kind} (format tag {tag})",
        name = endpoint.name,
        id = endpoint.id,
        rate = format.sample_rate,
        channels = format.channels,
        bits = format.bits_per_sample,
        valid = format.valid_bits_per_sample,
        kind = format.sample_format.name(),
        tag = format.format_tag,
    )
}

/// How long the capture may hear nothing before it warns.
pub const SILENCE_WARN_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

/// The warning the daemon logs when the capture heard no packet at all.
pub const NO_PACKET_WARNING: &str = "the audio capture has heard no packets";

/// The warning the daemon logs when the capture heard packets, but only silence.
pub const SILENT_WARNING: &str = "the audio capture has heard only silence";

/// The loudest sample that still counts as silence (-80 dBFS).
pub const SILENCE_FLOOR: f32 = 0.0001;

/// What the capture has heard, and whether it should warn that it has heard nothing.
///
/// The clock is handed in, so the warning is decided by the samples and not by the wall
/// clock, and a test can move it.
#[derive(Debug, Clone)]
pub struct SilenceWatch {
    /// When the quiet stretch started: the last sound, or the capture's own start.
    quiet_since: std::time::Instant,
    /// How many packets arrived since the quiet stretch started.
    quiet_packets: u64,
    /// When the last warning was logged.
    warned_at: Option<std::time::Instant>,
}

impl SilenceWatch {
    /// A watch for a capture that started at `now`.
    pub fn new(now: std::time::Instant) -> Self {
        Self {
            quiet_since: now,
            quiet_packets: 0,
            warned_at: None,
        }
    }

    /// A packet arrived at `now`; `peak` is the loudest sample in it.
    pub fn packet(&mut self, now: std::time::Instant, peak: f32) {
        if peak > SILENCE_FLOOR {
            // A sound: the quiet stretch starts again here.
            self.quiet_since = now;
            self.quiet_packets = 0;
        } else {
            self.quiet_packets += 1;
        }
    }

    /// The warning to log now, if one is due.
    ///
    /// It is rate limited: one warning per [`SILENCE_WARN_AFTER`], so a capture that
    /// hears nothing says so every ten seconds and not every frame.
    pub fn warning(&mut self, now: std::time::Instant) -> Option<&'static str> {
        let quiet = now.duration_since(self.quiet_since);
        if quiet < SILENCE_WARN_AFTER {
            return None;
        }
        if let Some(at) = self.warned_at {
            if now.duration_since(at) < SILENCE_WARN_AFTER {
                return None;
            }
        }
        let warning = if self.quiet_packets == 0 {
            NO_PACKET_WARNING
        } else {
            SILENT_WARNING
        };
        self.warned_at = Some(now);
        Some(warning)
    }
}

/// The capture's shared state: the latest spectrum and whether the capture runs.
///
/// This lives outside the Windows-only module so the daemon's state can be built on any
/// platform (the tests run there).
#[derive(Clone)]
pub struct Shared {
    inner: Arc<Inner>,
}

/// The shared state.
pub struct Inner {
    pub spectrum: std::sync::Mutex<Option<Arc<AudioSpectrum>>>,
    pub running: std::sync::atomic::AtomicBool,
    /// Whether the capture thread died after `open` returned `Ok` (B3).
    pub failed: std::sync::atomic::AtomicBool,
    /// Whether the first packet of this capture has been logged.
    pub first_packet: std::sync::atomic::AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                spectrum: std::sync::Mutex::new(None),
                running: std::sync::atomic::AtomicBool::new(false),
                failed: std::sync::atomic::AtomicBool::new(false),
                first_packet: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// Whether the capture is running.
    pub fn running(&self) -> bool {
        self.inner.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether the capture thread died after `open` returned `Ok` (B3: the
    /// daemon must not believe a dead capture is running).
    pub fn mark_failed(&self) {
        self.inner.failed.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether the capture thread died after `open` returned `Ok`.
    pub fn failed(&self) -> bool {
        self.inner.failed.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.inner.running.store(false, std::sync::atomic::Ordering::SeqCst);
        self.inner.failed.store(false, std::sync::atomic::Ordering::SeqCst);
        // The next capture is a new run, so it says again that its first packet arrived.
        self.inner.first_packet.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut s) = self.inner.spectrum.lock() {
            *s = None;
        }
    }

    /// The latest spectrum, if any.
    pub fn latest(&self) -> Option<Arc<AudioSpectrum>> {
        self.inner.spectrum.lock().ok().and_then(|s| s.clone())
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a spectrum from raw samples, at the mix format's sample rate.
pub fn spectrum_from_samples_at(samples: &[f32], sample_rate: u32) -> AudioSpectrum {
    let centers = band_centers();
    let magnitudes = magnitudes_with_rate(samples, &centers, sample_rate);
    AudioSpectrum { centers_hz: centers, magnitudes }
}

/// `I16` PCM sample to `f32` in -1..=1.
pub fn pcm16_to_f32(sample: i16) -> f32 {
    sample as f32 / 32768.0
}

/// `I24` PCM sample (sign-extended to 32 bits) to `f32` in -1..=1.
pub fn pcm24_to_f32(sample: i32) -> f32 {
    sample as f32 / 8388608.0
}

/// `I32` PCM sample to `f32` in -1..=1.
pub fn pcm32_to_f32(sample: i32) -> f32 {
    sample as f32 / 2147483648.0
}

/// Decodes one interleaved block of integer-PCM bytes into mono `f32` samples.
///
/// `bits` is the container width (16, 24 or 32) and `valid_bits` the valid bits
/// within it (S6): WASAPI delivers valid bits left-justified in the container,
/// so a 24-in-32 feed is decoded by reading the 32-bit container and shifting
/// back. Anything else is not decoded: the caller reports `UnsupportedFormat`
/// for it. This is the #193 fix: a PCM mix used to be read as float32, so a
/// 16-bit endpoint's samples were garbage.
pub fn decode_pcm_mono(data: &[u8], bits: u16, channels: u16) -> Option<Vec<f32>> {
    decode_pcm_mono_valid(data, bits, bits, channels)
}

/// Decodes one interleaved block of integer-PCM bytes with `valid_bits` valid
/// bits left-justified in a `container_bits`-wide container (S6).
pub fn decode_pcm_mono_valid(
    data: &[u8],
    container_bits: u16,
    valid_bits: u16,
    channels: u16,
) -> Option<Vec<f32>> {
    let channels = channels.max(1) as usize;
    let bytes_per_sample = match container_bits {
        16 => 2,
        24 => 3,
        32 => 4,
        _ => return None,
    };
    if !(8..=32).contains(&valid_bits) || valid_bits > container_bits {
        return None;
    }
    let frame_bytes = bytes_per_sample * channels;
    if frame_bytes == 0 {
        return None;
    }
    let frames = data.len() / frame_bytes;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * frame_bytes;
        let mut sum = 0.0f32;
        for ch in 0..channels {
            let off = base + ch * bytes_per_sample;
            let value = match container_bits {
                16 => pcm16_to_f32(i16::from_le_bytes([data[off], data[off + 1]])),
                24 => {
                    let raw = data[off] as i32 | ((data[off + 1] as i32) << 8) | ((data[off + 2] as i32) << 16);
                    let signed = if raw & 0x0080_0000 != 0 { raw | !0x00FF_FFFF } else { raw };
                    pcm24_to_f32(signed)
                }
                _ => {
                    let raw = i32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
                    if valid_bits < 32 {
                        // Left-justified valid bits: shift the container back
                        // down to the valid width, then scale by that width.
                        let shift = 32 - valid_bits as u32;
                        let adjusted = raw >> shift;
                        adjusted as f32 / (1i64 << (valid_bits - 1)) as f32
                    } else {
                        pcm32_to_f32(raw)
                    }
                }
            };
            sum += value;
        }
        mono.push(sum / channels as f32);
    }
    Some(mono)
}

/// Decodes one interleaved block of bytes by the mix format: the one decode
/// path the capture loop uses (S7), so a test through here guards the wiring.
pub fn decode_bytes_mono(data: &[u8], format: &MixFormat) -> Option<Vec<f32>> {
    match format.sample_format {
        SampleFormat::Float => Some(decode_float_mono(data, format.channels)),
        SampleFormat::Pcm => decode_pcm_mono_valid(
            data,
            format.bits_per_sample,
            format.valid_bits_per_sample,
            format.channels,
        ),
        SampleFormat::Other => None,
    }
}

/// Bytes per sample the endpoint publishes for `format` (the container width).
pub fn container_bytes(format: &MixFormat) -> usize {
    match format.sample_format {
        SampleFormat::Float => 4,
        _ => (format.bits_per_sample as usize).div_ceil(8),
    }
}

/// Decodes one interleaved block of float32 bytes into mono `f32` samples (#166).
pub fn decode_float_mono(data: &[u8], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let frame_bytes = 4 * channels;
    if frame_bytes == 0 {
        return Vec::new();
    }
    let frames = data.len() / frame_bytes;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * frame_bytes;
        let mut sum = 0.0f32;
        for ch in 0..channels {
            let off = base + ch * 4;
            sum += f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        }
        mono.push(sum / channels as f32);
    }
    mono
}

/// The peak in dBFS, floored at `AUDIO_DB_FLOOR`; never NaN (#166).
pub fn peak_db(peak: Option<f32>) -> Option<f32> {
    peak.map(|p| {
        if !p.is_finite() || p <= 0.0 {
            canvas_proto::consts::AUDIO_DB_FLOOR
        } else {
            (20.0 * p.log10()).max(canvas_proto::consts::AUDIO_DB_FLOOR)
        }
    })
}

/// What one capture has heard, as the API reports it. One value, read by both
/// the log line and `GET /audio/test`, so the two cannot disagree (#166).
#[derive(Debug, Clone)]
pub struct CaptureSnapshot {
    pub status: canvas_proto::audio::AudioTestStatus,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub bits_per_sample: Option<u16>,
    pub format_tag: Option<u16>,
    pub peak: Option<f32>,
    pub bands: Vec<f32>,
    pub packets: u64,
    pub silent_packets: u64,
}

impl Default for CaptureSnapshot {
    fn default() -> Self {
        Self {
            status: canvas_proto::audio::AudioTestStatus::Off,
            sample_rate: None,
            channels: None,
            bits_per_sample: None,
            format_tag: None,
            peak: None,
            bands: Vec::new(),
            packets: 0,
            silent_packets: 0,
        }
    }
}

/// Why opening the resolved endpoint failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// `AUDCLNT_E_DEVICE_IN_USE`: another app holds the device exclusively.
    /// A 0.0 peak alone never sets this; only an Initialize/Start refusal does.
    HeldExclusive,
    /// There is no endpoint to open at all.
    NoDevice,
    /// The mix format cannot be decoded; the tag names it.
    UnsupportedFormat { tag: u16 },
    /// Any other open failure; the message carries the HRESULT.
    OpenFailed { message: String },
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::HeldExclusive => f.write_str("held exclusive by another app"),
            OpenError::NoDevice => f.write_str("no audio device"),
            OpenError::UnsupportedFormat { tag } => f.write_str(&format!("format not supported (tag {tag})")),
            OpenError::OpenFailed { message } => f.write_str(message),
        }
    }
}

/// A device notification, as the capture thread consumes it. Callbacks only
/// post these; they never reopen the capture themselves (#166).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    /// `OnDefaultDeviceChanged` for `eRender` + `eConsole` only.
    DefaultChanged,
    /// `OnDeviceAdded` / `OnDeviceRemoved` / `OnDeviceStateChanged` for the pinned id.
    PinnedDeviceChanged { id: String },
    /// The endpoint was invalidated mid-run.
    DeviceInvalidated { id: String },
}

/// The seam over Windows audio: endpoint enumeration, the peak meter and the
/// notification source. The WASAPI implementation talks to Windows; the fake
/// lets every state be tested without hardware (#166).
pub trait AudioBackend: Send + Sync {
    /// Every active endpoint, enumerated on demand (never on a timer).
    fn enumerate(&self) -> Vec<canvas_proto::audio::AudioEndpoint>;
    /// Resolves the owner's choice to the endpoint the capture will open.
    fn resolve(&self, requested: &canvas_proto::audio::AudioSource) -> canvas_proto::audio::ResolvedAudioSource;
    /// Opens the resolved endpoint and fills `snapshot` with what it delivers.
    fn open(
        &self,
        resolved: &canvas_proto::audio::ResolvedAudioSource,
        shared: &Shared,
        snapshot: &std::sync::Arc<std::sync::Mutex<CaptureSnapshot>>,
        stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), OpenError>;
    /// The device peak on the opened endpoint (`IAudioMeterInformation`), 0..1.
    /// `None` when there is no meter.
    fn peak(&self, resolved: &canvas_proto::audio::ResolvedAudioSource) -> Option<f32>;
}

/// Resolves the owner's choice to the endpoint the capture will open: one
/// implementation for the fake and for WASAPI (S4), so a pinned endpoint that
/// is enumerated but not `Active` falls back the same way on both.
pub fn resolve_from(
    endpoints: &[canvas_proto::audio::AudioEndpoint],
    requested: &canvas_proto::audio::AudioSource,
) -> canvas_proto::audio::ResolvedAudioSource {
    use canvas_proto::audio::{AudioFallback, AudioFlow, AudioSource, ResolvedAudioSource};
    let default_render = endpoints
        .iter()
        .find(|e| e.flow == AudioFlow::Render && e.is_default && e.state == canvas_proto::audio::AudioEndpointState::Active);
    match requested {
        AudioSource::FollowDefault => match default_render {
            Some(ep) => ResolvedAudioSource { requested: requested.clone(), endpoint_id: Some(ep.id.clone()), name: Some(ep.name.clone()), flow: Some(ep.flow), fallback: None },
            None => ResolvedAudioSource { requested: requested.clone(), endpoint_id: None, name: None, flow: None, fallback: None },
        },
        AudioSource::Endpoint { id, flow, .. } => {
            if let Some(ep) = endpoints.iter().find(|e| &e.id == id && e.state == canvas_proto::audio::AudioEndpointState::Active) {
                ResolvedAudioSource { requested: requested.clone(), endpoint_id: Some(ep.id.clone()), name: Some(ep.name.clone()), flow: Some(ep.flow), fallback: None }
            } else if let Some(ep) = default_render {
                ResolvedAudioSource { requested: requested.clone(), endpoint_id: Some(ep.id.clone()), name: Some(ep.name.clone()), flow: Some(ep.flow), fallback: Some(AudioFallback::PickedMissing) }
            } else {
                ResolvedAudioSource { requested: requested.clone(), endpoint_id: None, name: None, flow: Some(*flow), fallback: Some(AudioFallback::PickedMissing) }
            }
        }
    }
}

/// The fake backend: every `AudioTestStatus` is reachable through it, with no
/// audio device ever opened.
pub struct FakeBackend {
    script: std::sync::Mutex<FakeScript>,
}

#[derive(Debug, Clone)]
struct FakeScript {
    endpoints: Vec<canvas_proto::audio::AudioEndpoint>,
    /// What `open` does: the status it leaves, or an error it returns.
    open: FakeOpen,
    format: MixFormat,
    peak: Option<f32>,
    packets: u64,
    silent_packets: u64,
    samples: Vec<f32>,
    /// When true, `open` leaves `Waiting` and produces nothing.
    pending: bool,
    /// When true, `open` returns `Ok` and the capture is already dead (B3).
    die_after_open: bool,
    /// The event sink the capture posts notifications to.
    events: Vec<BackendEvent>,
}

/// What the fake's `open` does: the status it leaves, or an error it returns.
#[derive(Debug, Clone, PartialEq)]
pub enum FakeOpen {
    Ok,
    HeldExclusive,
    UnsupportedFormat { tag: u16 },
    OpenFailed { message: String },
    NoDevice,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self {
            script: std::sync::Mutex::new(FakeScript {
                endpoints: Vec::new(),
                open: FakeOpen::Ok,
                format: MixFormat { sample_rate: 48000, channels: 2, bits_per_sample: 32, valid_bits_per_sample: 32, format_tag: 3, sample_format: SampleFormat::Float },
                peak: Some(0.5),
                packets: 10,
                silent_packets: 0,
                samples: Vec::new(),
                pending: false,
                die_after_open: false,
                events: Vec::new(),
            }),
        }
    }

    pub fn with_endpoints(endpoints: Vec<canvas_proto::audio::AudioEndpoint>) -> Self {
        let backend = Self::new();
        backend.script.lock().unwrap_or_else(|e| e.into_inner()).endpoints = endpoints;
        backend
    }

    pub fn set_endpoints(&self, endpoints: Vec<canvas_proto::audio::AudioEndpoint>) {
        self.script.lock().unwrap_or_else(|e| e.into_inner()).endpoints = endpoints;
    }
    pub fn set_open(&self, open: FakeOpen) { self.script.lock().unwrap_or_else(|e| e.into_inner()).open = open; }
    /// The open fails with `AUDCLNT_E_DEVICE_IN_USE`.
    pub fn open_held_exclusive() -> FakeOpen { FakeOpen::HeldExclusive }
    /// The mix format cannot be decoded; `tag` names it.
    pub fn open_unsupported(tag: u16) -> FakeOpen { FakeOpen::UnsupportedFormat { tag } }
    pub fn set_format(&self, format: MixFormat) { self.script.lock().unwrap_or_else(|e| e.into_inner()).format = format; }
    pub fn set_peak(&self, peak: Option<f32>) { self.script.lock().unwrap_or_else(|e| e.into_inner()).peak = peak; }
    pub fn set_packets(&self, packets: u64, silent_packets: u64) {
        let mut s = self.script.lock().unwrap_or_else(|e| e.into_inner());
        s.packets = packets;
        s.silent_packets = silent_packets;
    }
    pub fn set_pending(&self, pending: bool) { self.script.lock().unwrap_or_else(|e| e.into_inner()).pending = pending; }
    /// Simulates a capture thread that dies right after `open` returned `Ok` (B3).
    pub fn set_die_after_open(&self, die: bool) { self.script.lock().unwrap_or_else(|e| e.into_inner()).die_after_open = die; }
    pub fn set_samples(&self, samples: Vec<f32>) { self.script.lock().unwrap_or_else(|e| e.into_inner()).samples = samples; }
    pub fn events(&self) -> Vec<BackendEvent> { self.script.lock().unwrap_or_else(|e| e.into_inner()).events.clone() }
    pub fn post_event(&self, event: BackendEvent) { self.script.lock().unwrap_or_else(|e| e.into_inner()).events.push(event); }
}

impl AudioBackend for FakeBackend {
    fn enumerate(&self) -> Vec<canvas_proto::audio::AudioEndpoint> {
        self.script.lock().unwrap_or_else(|e| e.into_inner()).endpoints.clone()
    }

    fn resolve(&self, requested: &canvas_proto::audio::AudioSource) -> canvas_proto::audio::ResolvedAudioSource {
        resolve_from(&self.enumerate(), requested)
    }

    fn open(
        &self,
        resolved: &canvas_proto::audio::ResolvedAudioSource,
        shared: &Shared,
        snapshot: &std::sync::Arc<std::sync::Mutex<CaptureSnapshot>>,
        _stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), OpenError> {
        use canvas_proto::audio::AudioTestStatus;
        let script = self.script.lock().unwrap_or_else(|e| e.into_inner()).clone();
        shared.inner.running.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
        snap.sample_rate = Some(script.format.sample_rate);
        snap.channels = Some(script.format.channels);
        snap.bits_per_sample = Some(script.format.bits_per_sample);
        snap.format_tag = Some(script.format.format_tag);
        snap.packets = script.packets;
        snap.silent_packets = script.silent_packets;
        snap.peak = script.peak;
        if script.pending {
            snap.status = AudioTestStatus::Waiting;
            snap.bands = Vec::new();
            return Ok(());
        }
        if script.die_after_open {
            // The capture thread died after `open` returned `Ok`: the daemon
            // must stop believing the capture runs (B3). The thread leaves an
            // honest terminal status and marks the shared state failed.
            snap.status = AudioTestStatus::NoDevice;
            snap.peak = None;
            drop(snap);
            shared.inner.running.store(false, std::sync::atomic::Ordering::SeqCst);
            shared.mark_failed();
            return Ok(());
        }
        match &script.open {
            FakeOpen::HeldExclusive => { snap.status = AudioTestStatus::HeldExclusive; snap.peak = None; Err(OpenError::HeldExclusive) }
            FakeOpen::UnsupportedFormat { tag } => { snap.status = AudioTestStatus::UnsupportedFormat; snap.format_tag = Some(*tag); Err(OpenError::UnsupportedFormat { tag: *tag }) }
            FakeOpen::OpenFailed { message } => { snap.status = AudioTestStatus::NoDevice; Err(OpenError::OpenFailed { message: message.clone() }) }
            FakeOpen::NoDevice => { snap.status = AudioTestStatus::NoDevice; Err(OpenError::NoDevice) }
            FakeOpen::Ok => {
                if resolved.endpoint_id.is_none() {
                    snap.status = AudioTestStatus::NoDevice;
                    return Err(OpenError::NoDevice);
                }
                if resolved.fallback.is_some() {
                    snap.status = AudioTestStatus::Missing;
                } else if script.packets == 0 {
                    snap.status = AudioTestStatus::NoPackets;
                } else if script.peak.map(|p| p <= SILENCE_FLOOR).unwrap_or(true) {
                    snap.status = AudioTestStatus::Silent;
                } else {
                    snap.status = AudioTestStatus::Live;
                }
                let samples = if script.samples.is_empty() { vec![0.5f32; FFT_SIZE] } else { script.samples.clone() };
                let spectrum = spectrum_from_samples_at(&samples, script.format.sample_rate);
                snap.bands = spectrum.magnitudes.clone();
                if let Ok(mut s) = shared.inner.spectrum.lock() {
                    *s = Some(std::sync::Arc::new(spectrum));
                }
                Ok(())
            }
        }
    }

    fn peak(&self, _resolved: &canvas_proto::audio::ResolvedAudioSource) -> Option<f32> {
        self.script.lock().unwrap_or_else(|e| e.into_inner()).peak
    }
}

impl Default for FakeBackend {
    fn default() -> Self { Self::new() }
}

/// The capture, running on its own task.
///
/// The WASAPI part is Windows-only; the band maths above is portable and tested.
#[cfg(windows)]
pub mod wasapi {
    use super::*;
    use std::sync::atomic::Ordering;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    /// `WAVE_FORMAT_PCM`: integer samples.
    const WAVE_FORMAT_PCM: u16 = 1;
    /// `WAVE_FORMAT_IEEE_FLOAT`: float samples.
    const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
    /// `WAVE_FORMAT_EXTENSIBLE`: the real format is in the extension.
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
    /// `KSDATAFORMAT_SUBTYPE_PCM`.
    const KSDATAFORMAT_SUBTYPE_PCM: windows::core::GUID =
        windows::core::GUID::from_u128(0x0000_0001_0000_0010_8000_00aa_0038_9b71);
    /// `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`.
    const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows::core::GUID =
        windows::core::GUID::from_u128(0x0000_0003_0000_0010_8000_00aa_0038_9b71);
    /// The endpoint's friendly name, as Windows shows it ("Speakers (Realtek Audio)").
    ///
    /// The name comes from the device's property store. An endpoint without one has no
    /// name to give.
    fn friendly_name(device: &IMMDevice) -> Option<String> {
        use windows::Win32::Foundation::PROPERTYKEY;
        use windows::Win32::System::Com::STGM_READ;
        use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
        // `PKEY_Device_FriendlyName`: the device's name as the user sees it.
        let key = PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0xa45c_255e_fd1c_4efd_8020_67d1_46a8_50e0),
            pid: 14,
        };
        // SAFETY: the property store is opened for reading only, and the value it gives
        // is read before it is dropped.
        unsafe {
            let store: IPropertyStore = device.OpenPropertyStore(STGM_READ).ok()?;
            let value = store.GetValue(&key).ok()?;
            let name = value.to_string();
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        }
    }

    /// The endpoint's id, as Windows names it.
    fn device_id(device: &IMMDevice) -> String {
        // SAFETY: the id is a string the endpoint owns; it is copied out here.
        unsafe {
            match device.GetId() {
                Ok(id) => id.to_string().unwrap_or_default(),
                Err(_) => String::new(),
            }
        }
    }

    /// The mix format the endpoint gave, read from the format it published.
    ///
    /// The extension is only there when the format is `WAVEFORMATEXTENSIBLE`, which is
    /// what most endpoints publish.
    fn mix_format_of(
        format: &WAVEFORMATEX,
        extensible: Option<&WAVEFORMATEXTENSIBLE>,
    ) -> MixFormat {
        let sample_format = match format.wFormatTag {
            WAVE_FORMAT_PCM => SampleFormat::Pcm,
            WAVE_FORMAT_IEEE_FLOAT => SampleFormat::Float,
            WAVE_FORMAT_EXTENSIBLE => match extensible.map(|e| e.SubFormat) {
                Some(sub) if sub == KSDATAFORMAT_SUBTYPE_PCM => SampleFormat::Pcm,
                Some(sub) if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT => SampleFormat::Float,
                _ => SampleFormat::Other,
            },
            _ => SampleFormat::Other,
        };
        // S6: an extensible 24-in-32 feed publishes its valid bits in the
        // extension; fall back to the container width when it says none.
        let valid_bits = extensible
            .map(|e| unsafe { e.Samples.wValidBitsPerSample })
            .filter(|bits| *bits != 0 && *bits <= format.wBitsPerSample)
            .unwrap_or(format.wBitsPerSample);
        MixFormat {
            sample_rate: format.nSamplesPerSec,
            channels: format.nChannels,
            bits_per_sample: format.wBitsPerSample,
            valid_bits_per_sample: valid_bits,
            format_tag: format.wFormatTag,
            sample_format,
        }
    }

    /// The WASAPI backend: enumeration, the peak meter and the notification
    /// client over Windows audio (#166). COM is initialised on every thread
    /// that enumerates or captures. The state is shared behind an `Arc` so the
    /// capture thread owns it for its whole life.
    #[derive(Clone)]
    pub struct WasapiBackend {
        inner: std::sync::Arc<WasapiInner>,
    }

    /// The shared WASAPI state: the notification client registration, the
    /// posted events and the pinned endpoint id.
    pub struct WasapiInner {
        /// Events the notification callbacks posted, drained by the capture thread.
        events: std::sync::Mutex<Vec<BackendEvent>>,
        /// The pinned endpoint id the callbacks watch for.
        pinned: std::sync::Mutex<Option<String>>,
        /// The last successful enumeration (S5): resolve reads this, so the
        /// lease/effect path does no COM work. `GET /audio/sources` refreshes it.
        cached: std::sync::Mutex<Vec<canvas_proto::audio::AudioEndpoint>>,
    }

    impl WasapiInner {
        fn post(&self, event: BackendEvent) {
            self.events.lock().unwrap_or_else(|e| e.into_inner()).push(event);
        }

        fn drain_events(&self) -> Vec<BackendEvent> {
            std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()))
        }
    }

    impl Default for WasapiBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl WasapiBackend {
        pub fn new() -> Self {
            Self {
                inner: std::sync::Arc::new(WasapiInner {
                    events: std::sync::Mutex::new(Vec::new()),
                    pinned: std::sync::Mutex::new(None),
                    cached: std::sync::Mutex::new(Vec::new()),
                }),
            }
        }

        fn device_desc(device: &IMMDevice) -> Option<String> {
            use windows::Win32::Foundation::PROPERTYKEY;
            use windows::Win32::System::Com::STGM_READ;
            use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
            let key = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xa45c_255e_fd1c_4efd_8020_67d1_46a8_50e0), pid: 2 };
            unsafe {
                let store: IPropertyStore = device.OpenPropertyStore(STGM_READ).ok()?;
                let value = store.GetValue(&key).ok()?;
                if value.vt() == windows::Win32::System::Variant::VT_EMPTY { return None; }
                let name = value.to_string();
                if name.is_empty() { None } else { Some(name) }
            }
        }

        fn endpoint_name(device: &IMMDevice) -> String {
            let id = device_id(device);
            friendly_name(device).or_else(|| Self::device_desc(device)).unwrap_or_else(|| id.clone())
        }

        fn enumerator() -> Result<IMMDeviceEnumerator, String> {
            let _hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            unsafe {
                CoCreateInstance(
                    &windows::Win32::Media::Audio::MMDeviceEnumerator,
                    None,
                    CLSCTX_ALL,
                )
                .map_err(|e| format!("could not open the audio device enumerator: {e}"))
            }
        }
    }

    impl AudioBackend for WasapiBackend {
        fn enumerate(&self) -> Vec<canvas_proto::audio::AudioEndpoint> {
            use canvas_proto::audio::{AudioEndpoint, AudioEndpointState, AudioFlow};
            use windows::Win32::Media::Audio::{eCapture, DEVICE_STATE_ACTIVE};
            let enumerator = match Self::enumerator() { Ok(e) => e, Err(_) => return Vec::new() };
            let mut endpoints = Vec::new();
            for (flow_enum, flow) in [(eRender, AudioFlow::Render), (eCapture, AudioFlow::Capture)] {
                let collection = unsafe { enumerator.EnumAudioEndpoints(flow_enum, DEVICE_STATE_ACTIVE).ok() };
                let Some(collection) = collection else { continue };
                let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(flow_enum, eConsole).ok() }.map(|d| device_id(&d));
                let count = unsafe { collection.GetCount().unwrap_or(0) };
                for i in 0..count {
                    let Ok(device) = (unsafe { collection.Item(i) }) else { continue };
                    let id = device_id(&device);
                    endpoints.push(AudioEndpoint { id: id.clone(), flow, name: Self::endpoint_name(&device), is_default: default_id.as_deref() == Some(id.as_str()), state: AudioEndpointState::Active });
                }
            }
            *self.inner.cached.lock().unwrap_or_else(|e| e.into_inner()) = endpoints.clone();
            endpoints
        }

        fn resolve(&self, requested: &canvas_proto::audio::AudioSource) -> canvas_proto::audio::ResolvedAudioSource {
            // S5: resolve from the last successful enumeration, so the
            // lease/effect path does no COM work. Only the first resolve
            // before any enumeration touches COM.
            let cached = self.inner.cached.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if cached.is_empty() {
                let endpoints = self.enumerate();
                resolve_from(&endpoints, requested)
            } else {
                resolve_from(&cached, requested)
            }
        }

        fn open(
            &self,
            resolved: &canvas_proto::audio::ResolvedAudioSource,
            shared: &Shared,
            snapshot: &std::sync::Arc<std::sync::Mutex<CaptureSnapshot>>,
            stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        ) -> Result<(), OpenError> {
            let Some(id) = resolved.endpoint_id.clone() else { return Err(OpenError::NoDevice) };
            let flow = resolved.flow.unwrap_or(canvas_proto::audio::AudioFlow::Render);
            // The capture runs on its own thread; COM is initialised there.
            // The inner state is Arc-shared, so it outlives the thread.
            let inner = std::sync::Arc::clone(&self.inner);
            let backend = self.clone();
            let shared = shared.clone();
            let snapshot = std::sync::Arc::clone(snapshot);
            let stop = std::sync::Arc::clone(stop);
            let resolved = resolved.clone();
            std::thread::Builder::new()
                .name("canvas-rgb-audio".into())
                .spawn(move || {
                    // B3: a capture thread that dies clears the daemon's
                    // "running" state, so the next need reopens the capture
                    // instead of believing a dead thread is live.
                    if let Err(error) = backend.run_capture(&inner, &resolved, &id, flow, &shared, &snapshot, &stop) {
                        shared.mark_failed();
                        let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                        snap.status = match &error {
                            OpenError::HeldExclusive => canvas_proto::audio::AudioTestStatus::HeldExclusive,
                            OpenError::UnsupportedFormat { .. } => canvas_proto::audio::AudioTestStatus::UnsupportedFormat,
                            OpenError::NoDevice | OpenError::OpenFailed { .. } => canvas_proto::audio::AudioTestStatus::NoDevice,
                        };
                        if snap.status == canvas_proto::audio::AudioTestStatus::HeldExclusive {
                            snap.peak = None;
                        }
                        tracing::warn!(error = %error, "the audio capture thread died");
                    }
                })
                .map_err(|e| OpenError::OpenFailed { message: e.to_string() })?;
            Ok(())
        }

        fn peak(&self, _resolved: &canvas_proto::audio::ResolvedAudioSource) -> Option<f32> { None }
    }

    impl WasapiBackend {
        /// Runs one capture until `stop` is set, looping — never recursing —
        /// when the device is invalidated or the default changes (B2): one
        /// notification registration, one client at a time, re-resolved.
        #[allow(clippy::too_many_arguments)]
        fn run_capture(
            &self,
            inner: &std::sync::Arc<WasapiInner>,
            resolved: &canvas_proto::audio::ResolvedAudioSource,
            id: &str,
            flow: canvas_proto::audio::AudioFlow,
            shared: &Shared,
            snapshot: &std::sync::Arc<std::sync::Mutex<CaptureSnapshot>>,
            stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        ) -> Result<(), OpenError> {
            use canvas_proto::audio::AudioFlow;
            use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
            let _hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let enumerator = WasapiBackend::enumerator().map_err(|m| OpenError::OpenFailed { message: m })?;
            // Register the notification client once for this capture's life:
            // never re-registered inside a callback or a reopen, and the client
            // object (held in `_client`) lives as long as the capture thread.
            let _client: windows::Win32::Media::Audio::IMMNotificationClient = NotifyClient {
                shared: std::sync::Arc::clone(inner),
            }
            .into();
            if let Err(e) = unsafe { enumerator.RegisterEndpointNotificationCallback(&_client) } {
                tracing::warn!(error = %e, "the audio notification client could not be registered");
            }
            *inner.pinned.lock().unwrap_or_else(|e| e.into_inner()) = match &resolved.requested {
                canvas_proto::audio::AudioSource::Endpoint { id, .. } => Some(id.clone()),
                _ => None,
            };
            let mut current_id = id.to_string();
            let mut current_resolved = resolved.clone();
            let mut current_flow = flow;
            'open: loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let id_hstring = windows::core::HSTRING::from(current_id.as_str());
                let device: IMMDevice = unsafe {
                    enumerator
                        .GetDevice(&id_hstring)
                        .map_err(|e| OpenError::OpenFailed { message: format!("could not open the endpoint: {e}") })?
                };
                let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None).map_err(|e| OpenError::OpenFailed { message: e.to_string() })? };
                let meter: Option<IAudioMeterInformation> = unsafe { device.Activate(CLSCTX_ALL, None).ok() };
                let mix_format_ptr = unsafe { audio_client.GetMixFormat().map_err(|e| OpenError::OpenFailed { message: e.to_string() })? };
                if mix_format_ptr.is_null() { return Err(OpenError::NoDevice); }
                let mix_format: WAVEFORMATEX = unsafe { *mix_format_ptr };
                let extensible = if mix_format.wFormatTag == WAVE_FORMAT_EXTENSIBLE && extensible_has_extension(mix_format.cbSize) {
                    Some(unsafe { &*(mix_format_ptr as *const WAVEFORMATEXTENSIBLE) })
                } else { None };
                let mix = mix_format_of(&mix_format, extensible);
                let channels = mix.channels.max(1) as usize;
                match mix.sample_format {
                    SampleFormat::Float => {}
                    SampleFormat::Pcm if matches!(mix.bits_per_sample, 16 | 24 | 32) => {}
                    _ => {
                        let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                        snap.status = canvas_proto::audio::AudioTestStatus::UnsupportedFormat;
                        snap.format_tag = Some(mix.format_tag);
                        return Err(OpenError::UnsupportedFormat { tag: mix.format_tag });
                    }
                }
                // Render captures loopback; input captures use no flags (nit: a
                // plain 0, not a convoluted flag expression).
                let flags = if current_flow == AudioFlow::Render { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
                let init = unsafe {
                    audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, mix_format_ptr, None)
                };
                if let Err(e) = init {
                    // AUDCLNT_E_DEVICE_IN_USE is 0x8888000A: another app holds it exclusively.
                    if e.code().0 as u32 == 0x8888_000A {
                        let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                        snap.status = canvas_proto::audio::AudioTestStatus::HeldExclusive;
                        snap.peak = None;
                        return Err(OpenError::HeldExclusive);
                    }
                    return Err(OpenError::OpenFailed { message: format!("could not initialize the audio client: {e}") });
                }
                if let Err(e) = unsafe { audio_client.Start() } {
                    if e.code().0 as u32 == 0x8888_000A {
                        let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                        snap.status = canvas_proto::audio::AudioTestStatus::HeldExclusive;
                        snap.peak = None;
                        return Err(OpenError::HeldExclusive);
                    }
                    return Err(OpenError::OpenFailed { message: format!("could not start the audio client: {e}") });
                }
                shared.inner.running.store(true, Ordering::SeqCst);
                {
                    let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                    snap.sample_rate = Some(mix.sample_rate);
                    snap.channels = Some(mix.channels);
                    snap.bits_per_sample = Some(mix.bits_per_sample);
                    snap.format_tag = Some(mix.format_tag);
                    snap.status = canvas_proto::audio::AudioTestStatus::Waiting;
                }
                tracing::info!("{}", mix_format_line(&EndpointInfo { name: current_resolved.name.clone().unwrap_or_default(), id: current_id.clone() }, &mix));
                let capture: IAudioCaptureClient = unsafe { audio_client.GetService().map_err(|e| OpenError::OpenFailed { message: e.to_string() })? };
                let mut samples: Vec<f32> = Vec::with_capacity(FFT_SIZE);
                let mut last_publish = std::time::Instant::now();
                let mut watch = SilenceWatch::new(std::time::Instant::now());
                loop {
                    if stop.load(Ordering::SeqCst) || !shared.inner.running.load(Ordering::SeqCst) {
                        break 'open;
                    }
                    if let Some(event) = inner.drain_events().into_iter().next() {
                        // Stop the current client before re-opening (B2): the
                        // old capture is finished before the new one starts.
                        let _ = unsafe { audio_client.Stop() };
                        let next = self.resolve(&resolved.requested);
                        let (status, message) = match event {
                            BackendEvent::DefaultChanged => (canvas_proto::audio::AudioTestStatus::DefaultChanged, "the Windows default output changed; reopening the audio capture"),
                            BackendEvent::DeviceInvalidated { .. } | BackendEvent::PinnedDeviceChanged { .. } => (canvas_proto::audio::AudioTestStatus::Reopening, "the audio endpoint was invalidated; reopening the audio capture"),
                        };
                        {
                            let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                            snap.status = status;
                        }
                        tracing::info!("{message}");
                        // B2-C: a re-resolve with no endpoint is an honest
                        // terminal failure, never a stuck `Reopening`.
                        let Some(next_id) = next.endpoint_id.clone() else {
                            return Err(match next.fallback {
                                Some(canvas_proto::audio::AudioFallback::PickedMissing) => OpenError::NoDevice,
                                _ => OpenError::NoDevice,
                            });
                        };
                        current_resolved = next;
                        current_id = next_id;
                        current_flow = current_resolved.flow.unwrap_or(current_flow);
                        continue 'open;
                    }
                    let available = match unsafe { capture.GetNextPacketSize() } {
                        Ok(n) => n,
                        Err(_) => { std::thread::sleep(std::time::Duration::from_millis(10)); continue; }
                    };
                    if available == 0 {
                        if let Some(warning) = watch.warning(std::time::Instant::now()) { tracing::warn!("{warning}"); }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    let mut data: *mut u8 = std::ptr::null_mut();
                    let mut frames: u32 = 0;
                    let mut flags: u32 = 0;
                    if unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }.is_err() { continue; }
                    let mut peak = 0.0f32;
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                        samples.clear();
                    } else {
                        // Decode through the one shared path (S7) the tests guard.
                        let bytes_per_sample = container_bytes(&mix);
                        let byte_count = (frames as usize) * channels * bytes_per_sample;
                        let bytes = unsafe { std::slice::from_raw_parts(data, byte_count.min(1 << 24)) };
                        let mono = decode_bytes_mono(bytes, &mix).unwrap_or_default();
                        for value in &mono { peak = peak.max(value.abs()); }
                        samples.clear();
                        samples.extend_from_slice(&mono);
                    }
                    let _ = unsafe { capture.ReleaseBuffer(frames) };
                    watch.packet(std::time::Instant::now(), peak);
                    {
                        let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                        snap.packets += 1;
                        if peak <= SILENCE_FLOOR { snap.silent_packets += 1; }
                        if let Some(m) = &meter {
                            if let Ok(p) = unsafe { m.GetPeakValue() } { snap.peak = Some(p); }
                        } else { snap.peak = Some(peak); }
                        snap.status = if snap.packets == 0 { canvas_proto::audio::AudioTestStatus::NoPackets } else if peak <= SILENCE_FLOOR { canvas_proto::audio::AudioTestStatus::Silent } else { canvas_proto::audio::AudioTestStatus::Live };
                    }
                    if last_publish.elapsed() >= std::time::Duration::from_millis(33) && !samples.is_empty() {
                        last_publish = std::time::Instant::now();
                        let window: Vec<f32> = samples.iter().rev().take(FFT_SIZE).rev().copied().collect();
                        let spectrum = std::sync::Arc::new(super::spectrum_from_samples_at(&window, mix.sample_rate));
                        if let Ok(mut s) = shared.inner.spectrum.lock() { *s = Some(spectrum.clone()); }
                        let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                        snap.bands = spectrum.magnitudes.clone();
                    }
                }
            }
            unsafe { CoUninitialize() };
            Ok(())
        }
    }

    /// The `IMMNotificationClient`: callbacks only post to the capture thread.
    #[windows::core::implement(windows::Win32::Media::Audio::IMMNotificationClient)]
    struct NotifyClient {
        shared: std::sync::Arc<WasapiInner>,
    }

    impl NotifyClient {
        fn pinned_matches(&self, id: &str) -> bool {
            self.shared.pinned.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(id)
        }
    }

    impl windows::Win32::Media::Audio::IMMNotificationClient_Impl for NotifyClient_Impl {
        fn OnDefaultDeviceChanged(
            &self,
            flow: windows::Win32::Media::Audio::EDataFlow,
            role: windows::Win32::Media::Audio::ERole,
            _newdefaultdevice: &windows::core::PCWSTR,
        ) -> windows::core::Result<()> {
            // Only the default output (eRender + eConsole) moves the capture.
            if flow == windows::Win32::Media::Audio::eRender && role == windows::Win32::Media::Audio::eConsole {
                self.this.shared.post(BackendEvent::DefaultChanged);
            }
            Ok(())
        }
        fn OnDeviceAdded(&self, pwstrdeviceid: &windows::core::PCWSTR) -> windows::core::Result<()> {
            let id = unsafe { pwstrdeviceid.to_string().unwrap_or_default() };
            if self.this.pinned_matches(&id) {
                self.this.shared.post(BackendEvent::PinnedDeviceChanged { id });
            }
            Ok(())
        }
        fn OnDeviceRemoved(&self, pwstrdeviceid: &windows::core::PCWSTR) -> windows::core::Result<()> {
            let id = unsafe { pwstrdeviceid.to_string().unwrap_or_default() };
            if self.this.pinned_matches(&id) {
                self.this.shared.post(BackendEvent::PinnedDeviceChanged { id });
            }
            Ok(())
        }
        fn OnDeviceStateChanged(&self, pwstrdeviceid: &windows::core::PCWSTR, _dwnewstate: windows::Win32::Media::Audio::DEVICE_STATE) -> windows::core::Result<()> {
            let id = unsafe { pwstrdeviceid.to_string().unwrap_or_default() };
            if self.this.pinned_matches(&id) {
                self.this.shared.post(BackendEvent::PinnedDeviceChanged { id });
            }
            Ok(())
        }
        fn OnPropertyValueChanged(&self, _pwstrdeviceid: &windows::core::PCWSTR, _key: &windows::Win32::Foundation::PROPERTYKEY) -> windows::core::Result<()> {
            Ok(())
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_log_spaced_between_30hz_and_16khz() {
        let centers = band_centers();
        assert_eq!(centers.len(), BAND_COUNT);
        assert!((centers[0] - BAND_MIN_HZ).abs() < 0.01);
        assert!((centers[BAND_COUNT - 1] - BAND_MAX_HZ).abs() < 1.0);
        // Log spacing means the ratios between neighbours are constant.
        let ratio = centers[1] / centers[0];
        for i in 1..BAND_COUNT - 1 {
            let r = centers[i + 1] / centers[i];
            assert!((r - ratio).abs() < 0.02, "band {i} ratio {r} vs {ratio}");
        }
    }

    #[test]
    fn a_1khz_sine_makes_the_nearest_band_the_largest() {
        let centers = band_centers();
        let n = FFT_SIZE;
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / SAMPLE_RATE as f32;
            samples.push((2.0 * std::f32::consts::PI * 1000.0 * t).sin());
        }
        let mags = magnitudes(&samples, &centers);
        assert_eq!(mags.len(), centers.len());
        let expected = nearest_band(&centers, 1000.0);
        let largest = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(largest, expected, "band {expected} should be the largest, got {largest}");
        assert!(mags[expected] > 0.5, "the band should be strong, got {}", mags[expected]);
    }

    #[test]
    fn silence_is_silent() {
        let centers = band_centers();
        let mags = magnitudes(&vec![0.0f32; FFT_SIZE], &centers);
        assert!(mags.iter().all(|m| *m < 0.001));
    }

    /// Typical music peaks around -20 dBFS (per-sample amplitude 0.1). A
    /// visualiser must show that as a clearly visible bar, not as a sliver at
    /// the bottom of the canvas: -20 dBFS is 2/3 of the way from a -60 dBFS
    /// floor to 0 dBFS, so the mapped level is at least half of full scale.
    #[test]
    fn a_minus_20dbfs_sine_is_a_clearly_visible_band_level() {
        let centers = band_centers();
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                0.1 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin()
            })
            .collect();
        let mags = magnitudes(&samples, &centers);
        let expected = nearest_band(&centers, 1000.0);
        assert!(
            mags[expected] >= 0.5,
            "a -20 dBFS sine should map to at least half of full scale, got {}",
            mags[expected]
        );
    }

    #[test]
    fn a_full_scale_sine_reaches_the_top() {
        let centers = band_centers();
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (2.0 * std::f32::consts::PI * 1000.0 * t).sin()
            })
            .collect();
        let mags = magnitudes(&samples, &centers);
        let expected = nearest_band(&centers, 1000.0);
        assert!(
            mags[expected] >= 0.8,
            "a full-scale sine should reach the top, got {}",
            mags[expected]
        );
    }

    #[test]
    fn a_440hz_tone_picks_the_440hz_band() {
        let centers = band_centers();
        let n = FFT_SIZE;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect();
        let mags = magnitudes(&samples, &centers);
        let expected = nearest_band(&centers, 440.0);
        let largest = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(largest, expected);
    }

    #[test]
    fn the_spectrum_carries_its_centres() {
        let spectrum = spectrum_from_samples(&vec![0.0; 128]);
        assert_eq!(spectrum.centers_hz.len(), BAND_COUNT);
        assert_eq!(spectrum.magnitudes.len(), BAND_COUNT);
    }

    #[test]
    fn the_mix_format_line_names_the_endpoint_and_its_format() {
        let endpoint = EndpointInfo {
            name: "Speakers (Realtek Audio)".into(),
            id: "{0.0.0.00000000}.{4d1e55b2}".into(),
        };
        let format = MixFormat {
            sample_rate: 48000,
            channels: 2,
            bits_per_sample: 32,
            valid_bits_per_sample: 32,
            format_tag: 3,
            sample_format: SampleFormat::Float,
        };
        assert_eq!(
            mix_format_line(&endpoint, &format),
            "the audio capture opened Speakers (Realtek Audio) ({0.0.0.00000000}.{4d1e55b2}): \
             48000 Hz, 2 channels, 32 bits (32 valid), IEEE float (format tag 3)"
        );
    }

    #[test]
    fn the_sample_formats_have_names() {
        assert_eq!(SampleFormat::Pcm.name(), "PCM");
        assert_eq!(SampleFormat::Float.name(), "IEEE float");
        assert_eq!(SampleFormat::Other.name(), "unknown");
    }

    #[test]
    fn an_extensible_mix_is_recognised_by_its_extra_bytes() {
        // WAVEFORMATEXTENSIBLE is 40 bytes total and WAVEFORMATEX is 18, so an
        // extensible mix reports cbSize 22. Asking for 40 (the whole struct)
        // rejected every extensible mix, so the log said "unknown" for them.
        assert!(extensible_has_extension(22));
        assert!(extensible_has_extension(40));
        assert!(!extensible_has_extension(0));
        assert!(!extensible_has_extension(21));
    }

    #[test]
    fn an_extensible_float_mix_is_named_by_its_sub_format() {
        let endpoint = EndpointInfo {
            name: "the default render endpoint".into(),
            id: String::new(),
        };
        let format = MixFormat {
            sample_rate: 48000,
            channels: 2,
            bits_per_sample: 32,
            valid_bits_per_sample: 32,
            format_tag: 65534,
            sample_format: SampleFormat::Float,
        };
        assert_eq!(
            mix_format_line(&endpoint, &format),
            "the audio capture opened the default render endpoint (): 48000 Hz, \
             2 channels, 32 bits (32 valid), IEEE float (format tag 65534)"
        );
    }

    #[test]
    fn db_mapping_is_never_nan() {
        assert_eq!(db_mapped(f32::NAN), 0.0);
        assert_eq!(db_mapped(-1.0), 0.0);
        assert_eq!(db_mapped(0.0), 0.0);
        assert_eq!(db_mapped(f32::INFINITY), 1.0);
    }

    #[test]
    fn db_mapping_puts_music_level_at_two_thirds() {
        assert_eq!(db_mapped(0.0), 0.0);
        assert_eq!(db_mapped(1.0), 1.0);
        // -20 dBFS.
        let at_music = db_mapped(0.1);
        assert!((at_music - 2.0 / 3.0).abs() < 0.02, "got {at_music}");
        // -30 dBFS is half of the -60..0 range.
        let at_half = db_mapped(10f32.powf(-30.0 / 20.0));
        assert!((at_half - 0.5).abs() < 0.02, "got {at_half}");
        // At and below the floor is dark.
        assert_eq!(db_mapped(10f32.powf(-70.0 / 20.0)), 0.0);
    }

    /// The time `seconds` after `start`.
    fn at(start: std::time::Instant, seconds: u64) -> std::time::Instant {
        start + std::time::Duration::from_secs(seconds)
    }

    /// #193: integer PCM 16/24/32 decodes to the right level, not as float32.
    #[test]
    fn integer_pcm_decodes_to_the_right_level() {
        assert!((pcm16_to_f32(16384) - 0.5).abs() < 0.001);
        assert!((pcm16_to_f32(-32768) + 1.0).abs() < 0.001);
        assert!((pcm24_to_f32(4194304) - 0.5).abs() < 0.001);
        assert!((pcm32_to_f32(1073741824) - 0.5).abs() < 0.001);
        // A 16-bit stereo frame: left full-scale, right silent -> mono half.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&32767i16.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        let mono = decode_pcm_mono(&bytes, 16, 2).unwrap();
        assert_eq!(mono.len(), 1);
        assert!((mono[0] - 0.5).abs() < 0.01, "got {}", mono[0]);
        // A 24-bit mono sample at half scale.
        let raw: i32 = 4194304;
        let bytes = raw.to_le_bytes()[..3].to_vec();
        let mono = decode_pcm_mono(&bytes, 24, 1).unwrap();
        assert!((mono[0] - 0.5).abs() < 0.001);
        // An unsupported width decodes to nothing (UnsupportedFormat upstream).
        assert!(decode_pcm_mono(&[0u8; 8], 8, 1).is_none());
        assert!(decode_pcm_mono(&[0u8; 8], 64, 1).is_none());
    }

    /// S7 (#193): the decode path the capture loop uses — bytes plus mix format
    /// into mono samples — decodes PCM16/24/32 and float to the right level.
    /// A wiring regression back to reading PCM as float fails here, because
    /// the fixtures are integer bytes, not floats.
    #[test]
    fn the_shared_decode_path_reads_pcm_as_pcm_not_as_float() {
        let format = |bits: u16, valid: u16, channels: u16, sample_format: SampleFormat| MixFormat {
            sample_rate: 48000,
            channels,
            bits_per_sample: bits,
            valid_bits_per_sample: valid,
            format_tag: 1,
            sample_format,
        };
        // PCM16 stereo: left full-scale, right silent -> mono ~1/2.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&32767i16.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        let mono = decode_bytes_mono(&bytes, &format(16, 16, 2, SampleFormat::Pcm)).unwrap();
        assert!((mono[0] - 0.5).abs() < 0.01, "PCM16 got {}", mono[0]);
        // PCM24 mono at half scale.
        let raw: i32 = 4194304;
        let mono = decode_bytes_mono(&raw.to_le_bytes()[..3], &format(24, 24, 1, SampleFormat::Pcm)).unwrap();
        assert!((mono[0] - 0.5).abs() < 0.001, "PCM24 got {}", mono[0]);
        // PCM32 mono at half scale.
        let mono = decode_bytes_mono(&1073741824i32.to_le_bytes(), &format(32, 32, 1, SampleFormat::Pcm)).unwrap();
        assert!((mono[0] - 0.5).abs() < 0.001, "PCM32 got {}", mono[0]);
        // Float stereo at half scale either way.
        let mono = decode_bytes_mono(&[0.5f32.to_le_bytes(), 0.5f32.to_le_bytes()].concat(), &format(32, 32, 2, SampleFormat::Float)).unwrap();
        assert!((mono[0] - 0.5).abs() < 0.001, "float got {}", mono[0]);
        // Unknown format decodes to nothing (UnsupportedFormat upstream).
        assert!(decode_bytes_mono(&[0u8; 8], &format(32, 32, 1, SampleFormat::Other)).is_none());
    }

    /// S6: a 24-in-32 container delivers its valid bits left-justified; the
    /// decode reads exactly that level, without the padding noise.
    #[test]
    fn valid_bits_inside_the_container_are_read_at_their_alignment() {
        // 24 valid bits, value at +0.5 of full scale, left-justified in a
        // 32-bit container, low padding byte set: exact decode, not the
        // container read as a plain 32-bit sample.
        let container: i32 = (4194304i32 << 8) | 0xFF;
        let format = MixFormat { sample_rate: 48000, channels: 1, bits_per_sample: 32, valid_bits_per_sample: 24, format_tag: 65534, sample_format: SampleFormat::Pcm };
        let mono = decode_bytes_mono(&container.to_le_bytes(), &format).unwrap();
        assert!((mono[0] - 0.5).abs() < 1e-9, "24-in-32 got {}", mono[0]);
        // A zero valid-bits field reads as the container width.
        let packed = decode_pcm_mono_valid(&container.to_le_bytes(), 32, 32, 1).unwrap();
        assert!((packed[0] - 0.5).abs() < 0.001, "packed 32 got {}", packed[0]);
    }

    /// S4: resolve_from treats a non-Active pinned endpoint as missing, the
    /// same way the fake and WASAPI resolve now.
    #[test]
    fn a_pinned_endpoint_that_is_not_active_falls_back_to_the_default() {
        use canvas_proto::audio::{AudioEndpoint, AudioEndpointState, AudioFlow, AudioSource};
        let endpoints = vec![
            AudioEndpoint { id: "speakers-id".into(), flow: AudioFlow::Render, name: "Speakers".into(), is_default: true, state: AudioEndpointState::Active },
            AudioEndpoint { id: "mic-id".into(), flow: AudioFlow::Capture, name: "Microphone".into(), is_default: false, state: AudioEndpointState::NotConnected },
        ];
        let requested = AudioSource::Endpoint { id: "mic-id".into(), flow: AudioFlow::Capture, name_hint: "Microphone".into() };
        let resolved = resolve_from(&endpoints, &requested);
        assert_eq!(resolved.fallback, Some(canvas_proto::audio::AudioFallback::PickedMissing));
        assert_eq!(resolved.endpoint_id.as_deref(), Some("speakers-id"));
    }

    /// The sample rate from the mix format moves the band edges.
    #[test]
    fn the_sample_rate_moves_the_band_edges() {
        let centers = band_centers();
        // A 440 Hz tone at 22.05 kHz must land in the 440 Hz band, and at
        // 48 kHz in the same band: the rate is what the samples came in at.
        for rate in [22050u32, 44100, 48000] {
            let n = FFT_SIZE;
            let samples: Vec<f32> = (0..n)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                })
                .collect();
            let mags = magnitudes_with_rate(&samples, &centers, rate);
            let expected = nearest_band(&centers, 440.0);
            let largest = mags
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap();
            assert_eq!(largest, expected, "at {rate} Hz band {expected} should be largest, got {largest}");
            assert!(mags[expected] > 0.5, "at {rate} Hz the band should be strong, got {}", mags[expected]);
        }
        // The edges themselves move: the same samples read at the wrong rate
        // put the tone in a different band (440 Hz at 22.05 kHz reads as
        // about 957 Hz at 48 kHz), so the rate has to be the mix format's own.
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / 22050.0;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect();
        let at_own_rate = magnitudes_with_rate(&samples, &centers, 22050);
        let at_wrong_rate = magnitudes_with_rate(&samples, &centers, 48000);
        let own = at_own_rate.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(i, _)| i).unwrap();
        let wrong = at_wrong_rate.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(i, _)| i).unwrap();
        assert_eq!(own, nearest_band(&centers, 440.0));
        assert_ne!(wrong, own, "reading 22.05 kHz samples at 48 kHz must move the tone's band");
    }

    #[test]
    fn peak_db_is_floored_and_never_nan() {
        assert_eq!(peak_db(None), None);
        assert_eq!(peak_db(Some(0.0)), Some(canvas_proto::consts::AUDIO_DB_FLOOR));
        assert_eq!(peak_db(Some(f32::NAN)), Some(canvas_proto::consts::AUDIO_DB_FLOOR));
        assert_eq!(peak_db(Some(f32::INFINITY)), Some(canvas_proto::consts::AUDIO_DB_FLOOR));
        let db = peak_db(Some(0.5)).unwrap();
        assert!((db - (-6.02)).abs() < 0.1, "got {db}");
        assert_eq!(peak_db(Some(1e-9)), Some(canvas_proto::consts::AUDIO_DB_FLOOR));
    }


    #[test]
    fn the_watch_warns_when_no_packet_arrives() {
        let start = std::time::Instant::now();
        let mut watch = SilenceWatch::new(start);
        assert!(
            watch.warning(at(start, 9)).is_none(),
            "the warning waits for the full ten seconds"
        );
        assert_eq!(watch.warning(at(start, 10)), Some(NO_PACKET_WARNING));
    }

    #[test]
    fn the_watch_warns_when_only_silent_packets_arrive() {
        let start = std::time::Instant::now();
        let mut watch = SilenceWatch::new(start);
        // A silent packet every second: the capture hears something, but no sound.
        for second in 1..=5 {
            watch.packet(at(start, second), 0.0);
        }
        assert!(
            watch.warning(at(start, 5)).is_none(),
            "the packets are still arriving"
        );
        assert_eq!(watch.warning(at(start, 10)), Some(SILENT_WARNING));
    }

    #[test]
    fn the_watch_is_rate_limited() {
        let start = std::time::Instant::now();
        let mut watch = SilenceWatch::new(start);
        assert_eq!(watch.warning(at(start, 10)), Some(NO_PACKET_WARNING));
        assert!(
            watch.warning(at(start, 15)).is_none(),
            "the warning waits for the next window"
        );
        assert_eq!(watch.warning(at(start, 20)), Some(NO_PACKET_WARNING));
    }

    #[test]
    fn a_sound_starts_the_quiet_window_again() {
        let start = std::time::Instant::now();
        let mut watch = SilenceWatch::new(start);
        // Nine seconds of nothing: the warning is not due yet.
        assert!(watch.warning(at(start, 9)).is_none());
        // A sound at the ninth second: the quiet window starts again there.
        watch.packet(at(start, 9), 0.5);
        assert!(
            watch.warning(at(start, 18)).is_none(),
            "the sound reset the quiet window"
        );
        // Ten seconds after the sound the window is over again.
        assert_eq!(watch.warning(at(start, 19)), Some(NO_PACKET_WARNING));
    }
}
