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
/// is 0.0.
pub fn db_mapped(linear: f32) -> f32 {
    if linear <= 0.0 {
        return 0.0;
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

    let bin_hz = SAMPLE_RATE as f32 / size as f32;
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
    /// Bits a sample.
    pub bits_per_sample: u16,
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
        "the audio capture opened {name} ({id}): {rate} Hz, {channels} channels, {bits} bits, {kind} (format tag {tag})",
        name = endpoint.name,
        id = endpoint.id,
        rate = format.sample_rate,
        channels = format.channels,
        bits = format.bits_per_sample,
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
    /// Whether the first packet of this capture has been logged.
    pub first_packet: std::sync::atomic::AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                spectrum: std::sync::Mutex::new(None),
                running: std::sync::atomic::AtomicBool::new(false),
                first_packet: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// Whether the capture is running.
    pub fn running(&self) -> bool {
        self.inner.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.inner.running.store(false, std::sync::atomic::Ordering::SeqCst);
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
    /// The name the log uses when the endpoint has no friendly name.
    const DEFAULT_ENDPOINT: &str = "the default render endpoint";

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
        MixFormat {
            sample_rate: format.nSamplesPerSec,
            channels: format.nChannels,
            bits_per_sample: format.wBitsPerSample,
            format_tag: format.wFormatTag,
            sample_format,
        }
    }

    /// Starts a loopback capture on the default render endpoint.
    ///
    /// Returns the shared state; the capture runs until `stop` is called.
    pub fn start_default_render() -> Result<Shared, String> {
        // COM must be initialised on this thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let shared = Shared::new();
        shared.inner.running.store(true, Ordering::SeqCst);

        let capture_shared = shared.clone();
        std::thread::Builder::new()
            .name("canvas-rgb-audio".into())
            .spawn(move || {
                if let Err(e) = capture_loop(&capture_shared) {
                    tracing::warn!(error = %e, "the WASAPI loopback capture stopped");
                }
                capture_shared.stop();
            })
            .map_err(|e| e.to_string())?;

        Ok(shared)
    }

    /// The capture loop: read the render endpoint, FFT it, publish the spectrum.
    fn capture_loop(shared: &Shared) -> Result<(), String> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(
                &windows::Win32::Media::Audio::MMDeviceEnumerator,
                None,
                CLSCTX_ALL,
            )
            .map_err(|e| format!("could not open the audio device enumerator: {e}"))?;
            let device: IMMDevice = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| format!("no default render endpoint: {e}"))?;

            // Which device this is, read before the client is opened: the log has to name
            // the endpoint the audio comes from, or a reader cannot tell a silent mix
            // from the wrong device (#1).
            let name = friendly_name(&device);
            let endpoint = EndpointInfo {
                name: name.unwrap_or_else(|| DEFAULT_ENDPOINT.into()),
                id: device_id(&device),
            };

            let audio_client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| format!("could not open the audio client: {e}"))?;

            // The shared-mode loopback client takes the mix format from the endpoint.
            let mix_format_ptr = audio_client.GetMixFormat().map_err(|e| e.to_string())?;
            if mix_format_ptr.is_null() {
                return Err("the audio endpoint has no mix format".into());
            }
            let mix_format: WAVEFORMATEX = *mix_format_ptr;
            let channels = mix_format.nChannels.max(1) as usize;

            // The mix format, as the endpoint published it. The extension is only there
            // when the format is extensible, and it is read from the endpoint's own
            // pointer: the copy above drops it.
            let extensible = if mix_format.wFormatTag == WAVE_FORMAT_EXTENSIBLE
                && extensible_has_extension(mix_format.cbSize)
            {
                // The endpoint published an extensible format, so the pointer is a
                // WAVEFORMATEXTENSIBLE.
                Some(&*(mix_format_ptr as *const WAVEFORMATEXTENSIBLE))
            } else {
                None
            };
            let mix = mix_format_of(&mix_format, extensible);
            tracing::info!("{}", mix_format_line(&endpoint, &mix));

            audio_client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    0,
                    0,
                    // The pointer GetMixFormat returned, not the copy: the endpoint's format is usually
                    // WAVEFORMATEXTENSIBLE, and the 18-byte copy drops its extension, so Initialize
                    // refuses it (E_INVALIDARG).
                    mix_format_ptr,
                    None,
                )
                .map_err(|e| format!("could not initialize the audio client: {e}"))?;

            audio_client.Start().map_err(|e| format!("could not start the audio client: {e}"))?;

            let capture: IAudioCaptureClient =
                audio_client.GetService().map_err(|e| e.to_string())?;

            let mut samples: Vec<f32> = Vec::with_capacity(FFT_SIZE);
            let mut last_publish = std::time::Instant::now();
            // What the capture has heard. It warns when the effect that needs the audio
            // gets nothing from it (#1).
            let mut watch = SilenceWatch::new(std::time::Instant::now());

            while shared.inner.running.load(Ordering::SeqCst) {
                let available = match capture.GetNextPacketSize() {
                    Ok(n) => n,
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                };
                if available == 0 {
                    // No packet at all. The capture may be hearing nothing, and the
                    // effect that needs the audio is drawing black because of it.
                    if let Some(warning) = watch.warning(std::time::Instant::now()) {
                        tracing::warn!("{warning}");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }

                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(|e| e.to_string())?;

                let mut peak = 0.0f32;
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    samples.clear();
                } else {
                    let float_count = (frames as usize) * (channels as usize);
                    let slice = std::slice::from_raw_parts(data as *const f32, float_count.min(1 << 22));
                    // Mix down to mono.
                    let mut mono = Vec::with_capacity(frames as usize);
                    for i in 0..frames as usize {
                        let mut sum = 0.0f32;
                        for ch in 0..channels as usize {
                            sum += slice[i * channels as usize + ch];
                        }
                        let value = sum / channels as f32;
                        peak = peak.max(value.abs());
                        mono.push(value);
                    }
                    samples.clear();
                    samples.extend_from_slice(&mono);
                }
                let _ = capture.ReleaseBuffer(frames);

                // A packet arrived: say so once, so the log shows that the capture is
                // not just running but hearing something (#1).
                watch.packet(std::time::Instant::now(), peak);
                if !shared.inner.first_packet.swap(true, Ordering::SeqCst) {
                    tracing::info!("the first audio packet arrived");
                }
                if let Some(warning) = watch.warning(std::time::Instant::now()) {
                    tracing::warn!("{warning}");
                }

                // Publish at most 30 times a second.
                if last_publish.elapsed() >= std::time::Duration::from_millis(33) && !samples.is_empty() {
                    last_publish = std::time::Instant::now();
                    let window: Vec<f32> =
                        samples.iter().rev().take(FFT_SIZE).rev().copied().collect();
                    let spectrum = Arc::new(super::spectrum_from_samples(&window));
                    if let Ok(mut s) = shared.inner.spectrum.lock() {
                        *s = Some(spectrum);
                    }
                }
            }

            let _ = audio_client.Stop();
            CoUninitialize();
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
            format_tag: 3,
            sample_format: SampleFormat::Float,
        };
        assert_eq!(
            mix_format_line(&endpoint, &format),
            "the audio capture opened Speakers (Realtek Audio) ({0.0.0.00000000}.{4d1e55b2}): \
             48000 Hz, 2 channels, 32 bits, IEEE float (format tag 3)"
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
            format_tag: 65534,
            sample_format: SampleFormat::Float,
        };
        assert_eq!(
            mix_format_line(&endpoint, &format),
            "the audio capture opened the default render endpoint (): 48000 Hz, \
             2 channels, 32 bits, IEEE float (format tag 65534)"
        );
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
