//! Screen Ambience input: Desktop Duplication on the primary output.
//!
//! The daemon runs in the user session, so DXGI's Desktop Duplication works. The frames
//! are downscaled on the CPU to at most 320x200 (the logical canvas size) and published
//! at no more than 30 fps. The capture only runs while an effect that needs the screen is
//! active.

use canvas_proto::traits::ScreenFrame;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The largest frame the daemon publishes.
pub const MAX_WIDTH: u32 = 320;
pub const MAX_HEIGHT: u32 = 200;
/// The capture rate cap.
pub const MAX_FPS: u32 = canvas_proto::consts::SCREEN_CAPTURE_MAX_FPS;

/// The state the capture thread publishes into.
///
/// The capture thread and the daemon share it, so it is behind an `Arc`.
#[derive(Clone)]
pub struct Shared {
    inner: Arc<Inner>,
}

/// The shared state.
pub struct Inner {
    pub frame: Mutex<Option<Arc<ScreenFrame>>>,
    pub running: AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                frame: Mutex::new(None),
                running: AtomicBool::new(false),
            }),
        }
    }

    /// Whether the capture is running.
    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        if let Ok(mut f) = self.inner.frame.lock() {
            *f = None;
        }
    }

    /// The most recent frame, if any.
    pub fn latest(&self) -> Option<Arc<ScreenFrame>> {
        self.inner.frame.lock().ok().and_then(|f| f.clone())
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

/// Scales a frame to at most 320x200, keeping the aspect ratio.
pub fn downscale(frame: &ScreenFrame) -> ScreenFrame {
    if frame.width <= MAX_WIDTH && frame.height <= MAX_HEIGHT {
        return frame.clone();
    }
    let scale = (MAX_WIDTH as f32 / frame.width as f32).min(MAX_HEIGHT as f32 / frame.height as f32);
    let width = ((frame.width as f32 * scale).round() as u32).max(1);
    let height = ((frame.height as f32 * scale).round() as u32).max(1);
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let sy = ((y as f32 / scale) as u32).min(frame.height.saturating_sub(1));
        for x in 0..width {
            let sx = ((x as f32 / scale) as u32).min(frame.width.saturating_sub(1));
            let index = (sy * frame.width + sx) as usize;
            pixels.push(frame.pixels.get(index).copied().unwrap_or_default());
        }
    }
    ScreenFrame { width, height, pixels }
}

/// The mean colour of a frame, as 0..1 floats. Used by the tests and the diagnostics.
pub fn mean_rgb(frame: &ScreenFrame) -> [f32; 3] {
    if frame.pixels.is_empty() {
        return [0.0; 3];
    }
    let mut sum = [0.0f32; 3];
    for p in &frame.pixels {
        sum[0] += p.r as f32 / 255.0;
        sum[1] += p.g as f32 / 255.0;
        sum[2] += p.b as f32 / 255.0;
    }
    let n = frame.pixels.len() as f32;
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

/// Starts the Desktop Duplication capture on the primary output.
///
/// Returns the shared state. The capture runs on its own thread until `stop` is called.
#[cfg(windows)]
pub fn start_primary() -> Result<Shared, String> {
    let shared = Shared::new();
    shared.inner.running.store(true, Ordering::SeqCst);
    let capture_shared = shared.clone();
    std::thread::Builder::new()
        .name("canvas-rgb-screen".into())
        .spawn(move || {
            if let Err(e) = capture_loop(&capture_shared) {
                tracing::warn!(error = %e, "the Desktop Duplication capture stopped");
            }
            capture_shared.stop();
        })
        .map_err(|e| e.to_string())?;
    Ok(shared)
}

#[cfg(not(windows))]
pub fn start_primary() -> Result<Shared, String> {
    Err("screen capture is Windows-only".into())
}

/// Reads BGRA (or RGBA) rows by `pitch` and scales them straight into a frame of at most
/// 320x200, keeping the aspect ratio. Same sampling as `downscale`, without building the
/// full-size pixel buffer first.
pub fn sample_rows(data: &[u8], pitch: usize, width: u32, height: u32, bgra: bool) -> Option<ScreenFrame> {
    if width == 0
        || height == 0
        || pitch < width as usize * 4
        || data.len() < pitch * (height as usize - 1) + width as usize * 4
    {
        return None;
    }
    let scale = (MAX_WIDTH as f32 / width as f32).min(MAX_HEIGHT as f32 / height as f32).min(1.0);
    let out_w = ((width as f32 * scale).round() as u32).max(1);
    let out_h = ((height as f32 * scale).round() as u32).max(1);
    let mut pixels = Vec::with_capacity((out_w * out_h) as usize);
    for y in 0..out_h {
        let sy = ((y as f32 / scale) as u32).min(height - 1) as usize;
        let row = &data[sy * pitch..];
        for x in 0..out_w {
            let sx = ((x as f32 / scale) as u32).min(width - 1) as usize;
            let p = &row[sx * 4..sx * 4 + 4];
            pixels.push(if bgra {
                canvas_proto::Rgb8::new(p[2], p[1], p[0])
            } else {
                canvas_proto::Rgb8::new(p[0], p[1], p[2])
            });
        }
    }
    Some(ScreenFrame { width: out_w, height: out_h, pixels })
}

/// Lets a warning through at most once per interval, so a failing capture is visible in the log
/// without flooding it.
#[cfg(windows)]
struct WarnLimiter {
    last: Option<std::time::Instant>,
}

#[cfg(windows)]
impl WarnLimiter {
    const EVERY: std::time::Duration = std::time::Duration::from_secs(10);

    fn ready(&mut self) -> bool {
        let due = self.last.is_none_or(|t| t.elapsed() >= Self::EVERY);
        if due {
            self.last = Some(std::time::Instant::now());
        }
        due
    }
}

/// The reused staging texture and the size and format it was made for.
#[cfg(windows)]
struct Staging {
    texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    width: u32,
    height: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
}

/// One Desktop Duplication session: the device, the duplication and a reusable staging texture.
#[cfg(windows)]
struct Session {
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    duplication: windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication,
    staging: Option<Staging>,
}

#[cfg(windows)]
fn create_session() -> Result<Session, String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1};

    // SAFETY: plain COM calls; every out-pointer is a local that outlives the call.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(|e| e.to_string())?;

        // The primary output is the first adapter's first output that sits at the desktop's origin.
        let mut chosen: Option<(IDXGIAdapter, IDXGIOutput1)> = None;
        for adapter_index in 0.. {
            let Ok(adapter) = factory.EnumAdapters(adapter_index) else {
                break;
            };
            for output_index in 0.. {
                let Ok(output) = adapter.EnumOutputs(output_index) else {
                    break;
                };
                let desc = output.GetDesc().map_err(|e| e.to_string())?;
                if desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0 {
                    // `DuplicateOutput` lives on IDXGIOutput1.
                    let output1: IDXGIOutput1 = output.cast().map_err(|e| e.to_string())?;
                    chosen = Some((adapter.clone(), output1));
                    break;
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        let Some((adapter, output)) = chosen else {
            return Err("no primary output found".into());
        };

        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        // With an explicit adapter the driver type must be UNKNOWN; HARDWARE and WARP are for a
        // null adapter. The output belongs to this adapter, so no fallback adapter would work.
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|e| format!("could not create a D3D11 device: {e}"))?;
        let device = device.ok_or("no D3D11 device")?;
        let context = context.ok_or("no D3D11 device context")?;

        let duplication = output
            .DuplicateOutput(&device)
            .map_err(|e| format!("could not duplicate the output: {e}"))?;
        Ok(Session { device, context, duplication, staging: None })
    }
}

/// How one pass over the live session ended.
#[cfg(windows)]
enum PassEnd {
    /// `stop` was called.
    Stopped,
    /// The duplication was lost (resolution change, UAC, fullscreen switch): make a new one.
    Lost,
}

/// Copies the acquired texture into the reused staging texture, maps it and publishes a frame.
#[cfg(windows)]
fn publish_frame(
    session: &mut Session,
    resource: &windows::Win32::Graphics::Dxgi::IDXGIResource,
    shared: &Shared,
) -> Result<(), String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D11::{
        ID3D11Resource, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM};

    // SAFETY: the textures and the mapped memory are used only between Map and Unmap.
    unsafe {
        let texture: ID3D11Texture2D = resource.cast().map_err(|e| format!("not a 2D texture: {e}"))?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        let bgra = if desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM {
            true
        } else if desc.Format == DXGI_FORMAT_R8G8B8A8_UNORM {
            false
        } else {
            return Err(format!("unsupported desktop format {}", desc.Format.0));
        };

        let reuse = session
            .staging
            .as_ref()
            .is_some_and(|s| s.width == desc.Width && s.height == desc.Height && s.format == desc.Format);
        if !reuse {
            let mut staging_desc = desc;
            staging_desc.Usage = D3D11_USAGE_STAGING;
            staging_desc.BindFlags = 0;
            staging_desc.MiscFlags = 0;
            staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            staging_desc.MipLevels = 1;
            staging_desc.ArraySize = 1;
            let mut created: Option<ID3D11Texture2D> = None;
            session
                .device
                .CreateTexture2D(&staging_desc, None, Some(&mut created))
                .map_err(|e| format!("could not create the staging texture: {e}"))?;
            let texture = created.ok_or("no staging texture")?;
            session.staging = Some(Staging { texture, width: desc.Width, height: desc.Height, format: desc.Format });
        }
        let staging = &session.staging.as_ref().ok_or("no staging texture")?.texture;

        let source: ID3D11Resource = texture.cast().map_err(|e| e.to_string())?;
        let target: ID3D11Resource = staging.cast().map_err(|e| e.to_string())?;
        session.context.CopyResource(&target, &source);

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        session
            .context
            .Map(&target, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| format!("could not map the staging texture: {e}"))?;
        let pitch = mapped.RowPitch as usize;
        let frame = if mapped.pData.is_null() {
            None
        } else {
            let data = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * desc.Height as usize);
            sample_rows(data, pitch, desc.Width, desc.Height, bgra)
        };
        session.context.Unmap(&target, 0);

        let frame = frame.ok_or("the mapped surface was empty")?;
        if let Ok(mut slot) = shared.inner.frame.lock() {
            *slot = Some(Arc::new(frame));
        }
        Ok(())
    }
}

/// Runs one session until it is lost or the capture stops.
#[cfg(windows)]
fn run_session(session: &mut Session, shared: &Shared, warn: &mut WarnLimiter) -> PassEnd {
    use windows::Win32::Graphics::Dxgi::{
        DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    };

    let interval = std::time::Duration::from_millis(1000 / MAX_FPS as u64);
    let mut last_publish = std::time::Instant::now() - interval;
    while shared.inner.running.load(Ordering::SeqCst) {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        // SAFETY: `info` and `resource` outlive the call; ReleaseFrame follows every success.
        let acquired = unsafe { session.duplication.AcquireNextFrame(100, &mut info, &mut resource) };
        match acquired {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                if warn.ready() {
                    tracing::warn!("the desktop duplication was lost; recreating it");
                }
                return PassEnd::Lost;
            }
            Err(e) => {
                if warn.ready() {
                    tracing::warn!(error = %e, "could not acquire a desktop frame");
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
        }
        // A frame with no present time carries only a cursor move, or the empty surface a fresh
        // duplication starts with; publishing it would flash the lights black.
        if info.LastPresentTime != 0 && last_publish.elapsed() >= interval {
            if let Some(resource) = resource.as_ref() {
                match publish_frame(session, resource, shared) {
                    Ok(()) => last_publish = std::time::Instant::now(),
                    Err(e) => {
                        if warn.ready() {
                            tracing::warn!(error = %e, "could not read the desktop frame");
                        }
                    }
                }
            }
        }
        // SAFETY: a frame was acquired above.
        let _ = unsafe { session.duplication.ReleaseFrame() };
    }
    PassEnd::Stopped
}

/// The capture loop. A lost or failed duplication is recreated with backoff; only `stop` ends it.
#[cfg(windows)]
fn capture_loop(shared: &Shared) -> Result<(), String> {
    let mut warn = WarnLimiter { last: None };
    let mut backoff = std::time::Duration::from_millis(250);
    while shared.inner.running.load(Ordering::SeqCst) {
        match create_session() {
            Ok(mut session) => {
                backoff = std::time::Duration::from_millis(250);
                if matches!(run_session(&mut session, shared, &mut warn), PassEnd::Stopped) {
                    break;
                }
            }
            Err(e) => {
                if warn.ready() {
                    tracing::warn!(error = %e, "the screen capture could not start; retrying");
                }
            }
        }
        // Back off in short steps so `stop` is honoured quickly.
        let until = std::time::Instant::now() + backoff;
        while std::time::Instant::now() < until && shared.inner.running.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::Rgb8;

    fn frame(width: u32, height: u32, color: Rgb8) -> ScreenFrame {
        ScreenFrame { width, height, pixels: vec![color; (width * height) as usize] }
    }

    #[test]
    fn a_small_frame_is_kept_as_is() {
        let f = frame(100, 50, Rgb8::WHITE);
        let scaled = downscale(&f);
        assert_eq!(scaled.width, 100);
        assert_eq!(scaled.height, 50);
    }

    #[test]
    fn a_big_frame_is_scaled_into_the_canvas() {
        let f = frame(1920, 1080, Rgb8::WHITE);
        let scaled = downscale(&f);
        assert!(scaled.width <= MAX_WIDTH);
        assert!(scaled.height <= MAX_HEIGHT);
        assert_eq!(scaled.pixels.len(), (scaled.width * scaled.height) as usize);
        // 16:9 stays 16:9.
        let ratio = scaled.width as f32 / scaled.height as f32;
        assert!((ratio - 16.0 / 9.0).abs() < 0.05, "ratio {ratio}");
    }

    #[test]
    fn the_mean_of_a_red_frame_is_red() {
        let f = frame(64, 64, Rgb8::new(255, 0, 0));
        let mean = mean_rgb(&f);
        assert!((mean[0] - 1.0).abs() < 0.001);
        assert!(mean[1].abs() < 0.001);
        assert!(mean[2].abs() < 0.001);
    }

    #[test]
    fn rows_are_read_by_pitch_and_scaled() {
        // 640x400 BGRA with 16 bytes of row padding; every pixel is blue (B=255).
        let (w, h, pitch) = (640u32, 400u32, 640usize * 4 + 16);
        let mut data = vec![0u8; pitch * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                data[y * pitch + x * 4] = 255;
                data[y * pitch + x * 4 + 3] = 255;
            }
        }
        let f = sample_rows(&data, pitch, w, h, true).expect("a frame");
        assert_eq!((f.width, f.height), (320, 200));
        assert!(f.pixels.iter().all(|p| *p == Rgb8::new(0, 0, 255)));
        assert!(sample_rows(&data[..100], pitch, w, h, true).is_none());
    }

    /// Starts the real Desktop Duplication capture for 3 s and checks a frame arrives.
    /// Needs an interactive desktop, so it runs only on request:
    /// `cargo test -p canvas-daemon real_screen_capture -- --ignored --nocapture`.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs an interactive desktop"]
    fn real_screen_capture_publishes_a_frame() {
        let shared = start_primary().expect("start");
        let mut first = None;
        let begin = std::time::Instant::now();
        while begin.elapsed() < std::time::Duration::from_secs(3) {
            if first.is_none() {
                if let Some(f) = shared.latest() {
                    first = Some((begin.elapsed(), f));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let last = shared.latest();
        if let Some(l) = last.as_ref() {
            let nonblack = l.pixels.iter().filter(|p| p.r > 0 || p.g > 0 || p.b > 0).count();
            println!("last frame: mean_rgb={:?} non-black pixels={}/{}", mean_rgb(l), nonblack, l.pixels.len());
        }
        shared.stop();
        let (at, frame) = first.expect("no frame was published in 3 s");
        let mean = mean_rgb(&frame);
        println!(
            "first frame after {:?}: {}x{} px={} mean_rgb={:?}",
            at,
            frame.width,
            frame.height,
            frame.pixels.len(),
            mean
        );
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(frame.pixels.len(), (frame.width * frame.height) as usize);
        assert!(last.is_some(), "frames kept coming");
    }
}
