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

/// Turns a block of mono samples into band magnitudes in 0..1.
///
/// `samples` is the most recent FFT window. The window is Hann-windowed to cut the
/// leakage that would otherwise smear a tone across neighbouring bands.
///
/// Each band owns the bins between the midpoints to its neighbours, so a tone lands in
/// exactly one band and that band is the largest.
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
        mags.push((peak / gain).clamp(0.0, 1.0));
    }
    mags
}

/// Builds a spectrum from raw samples.
pub fn spectrum_from_samples(samples: &[f32]) -> AudioSpectrum {
    let centers = band_centers();
    let magnitudes = magnitudes(samples, &centers);
    AudioSpectrum { centers_hz: centers, magnitudes }
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
}

impl Shared {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                spectrum: std::sync::Mutex::new(None),
                running: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// Whether the capture is running.
    pub fn running(&self) -> bool {
        self.inner.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.inner.running.store(false, std::sync::atomic::Ordering::SeqCst);
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
        WAVEFORMATEX,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

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

            audio_client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    0,
                    0,
                    &mix_format,
                    None,
                )
                .map_err(|e| format!("could not initialize the audio client: {e}"))?;

            audio_client.Start().map_err(|e| format!("could not start the audio client: {e}"))?;

            let capture: IAudioCaptureClient =
                audio_client.GetService().map_err(|e| e.to_string())?;

            let mut samples: Vec<f32> = Vec::with_capacity(FFT_SIZE);
            let mut last_publish = std::time::Instant::now();

            while shared.inner.running.load(Ordering::SeqCst) {
                let available = match capture.GetNextPacketSize() {
                    Ok(n) => n,
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                };
                if available == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }

                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(|e| e.to_string())?;

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
                        mono.push(sum / channels as f32);
                    }
                    samples.clear();
                    samples.extend_from_slice(&mono);
                }
                let _ = capture.ReleaseBuffer(frames);

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
}
