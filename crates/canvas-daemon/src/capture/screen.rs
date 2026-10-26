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

/// The capture loop.
#[cfg(windows)]
fn capture_loop(shared: &Shared) -> Result<(), String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter, IDXGIOutput1, IDXGIOutputDuplication, IDXGIFactory1,
    };

    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(|e| e.to_string())?;

        // The primary output is the first adapter's first output that sits at the
        // desktop's origin.
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
        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let created = D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        );
        if created.is_err() {
            // WARP lets the capture work on a machine with no hardware D3D.
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_WARP,
                windows::Win32::Foundation::HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("could not create a D3D11 device: {e}"))?;
        }
        let device = device.ok_or("no D3D11 device")?;

        let duplication: IDXGIOutputDuplication = output
            .DuplicateOutput(&device)
            .map_err(|e| format!("could not duplicate the output: {e}"))?;

        let interval = std::time::Duration::from_millis(1000 / MAX_FPS as u64);
        let mut last_publish = std::time::Instant::now() - interval;

        while shared.inner.running.load(Ordering::SeqCst) {
            let mut frame_info = windows::Win32::Graphics::Dxgi::DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            let acquired = duplication.AcquireNextFrame(100, &mut frame_info, &mut resource);
            if acquired.is_err() {
                // A timeout just means no new frame; the loop keeps going.
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }

            // `MapDesktopSurface` gives the mapped rectangle and the pointer to the
            // pixels. The width and height come from the output's mode.
            let mode = output.GetDesc().map(|d| {
                (
                    (d.DesktopCoordinates.right - d.DesktopCoordinates.left).max(0) as u32,
                    (d.DesktopCoordinates.bottom - d.DesktopCoordinates.top).max(0) as u32,
                )
            });
            let (mode_width, mode_height) = mode.unwrap_or((0, 0));

            match duplication.MapDesktopSurface() {
                Ok(mapped) => {
                    let width = mode_width;
                    let height = mode_height;
                    if width > 0 && height > 0 && !mapped.pBits.is_null() {
                        let pitch = mapped.Pitch.max(0) as usize;
                        let base = mapped.pBits;
                        let mut pixels = Vec::with_capacity((width * height) as usize);
                        for row in 0..height {
                            let start = base.add(row as usize * pitch);
                            for x in 0..width {
                                let b = *start.add(x as usize * 4);
                                let g = *start.add(x as usize * 4 + 1);
                                let r = *start.add(x as usize * 4 + 2);
                                pixels.push(canvas_proto::Rgb8::new(r, g, b));
                            }
                        }
                        let frame = downscale(&ScreenFrame { width, height, pixels });
                        if last_publish.elapsed() >= interval {
                            last_publish = std::time::Instant::now();
                            if let Ok(mut slot) = shared.inner.frame.lock() {
                                *slot = Some(Arc::new(frame));
                            }
                        }
                    }
                    let _ = duplication.UnMapDesktopSurface();
                }
                Err(e) => {
                    tracing::debug!(error = %e, "could not map the desktop surface");
                }
            }
            let _ = duplication.ReleaseFrame();
        }
        Ok(())
    }
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
}
