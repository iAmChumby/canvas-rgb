//! The writer task of one device: every frame goes to the adapter, and a frame that went out is
//! recorded (#81).
//!
//! The task lives here, in the library, and not in the daemon's binary, so a test can run it.
//! The time of the last frame is only worth logging if the writer really records it, and that is
//! what the test proves.

use std::sync::Arc;

use canvas_proto::traits::DeviceAdapter;
use canvas_proto::{DeviceId, Rgb8};

use crate::last_frame::LastFrame;

/// Writes the frames of one device to its adapter and records the time of every frame that went
/// out (#81). Returns when the channel closes.
///
/// A frame that failed to be written is not recorded: the last frame is the last one that reached
/// the device, not the last one that was tried.
///
/// The frames come from a watch channel: the writer takes the newest frame there is, so a device
/// that is behind shows the newest colours and not the oldest ones it has not written yet
/// (#308).
pub async fn writer_loop(
    adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    device: DeviceId,
    frames: tokio::sync::watch::Receiver<Vec<Rgb8>>,
    last_frame: Arc<LastFrame>,
) {
    writer_loop_with_lane(adapter, None, device, frames, last_frame).await;
}

/// [`writer_loop`], with the frame lane of the OpenRGB link (see [`App::frame_lane`](crate::state::App)).
///
/// A frame for an OpenRGB device goes through the lane: it shares the link with the adapter and
/// does not wait on the adapter lock, which the supervisor holds while it waits on OpenRGB for a
/// reply (a rescan, a refresh). A device the composite serves natively (the keyboard), and every
/// device when there is no lane, goes through the adapter as before.
pub async fn writer_loop_with_lane(
    adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    lane: Option<canvas_adapters::openrgb::OpenRgbAdapter>,
    device: DeviceId,
    mut frames: tokio::sync::watch::Receiver<Vec<Rgb8>>,
    last_frame: Arc<LastFrame>,
) {
    let lane = lane.filter(|_| crate::composite::CompositeAdapter::is_openrgb_device(&device));
    let mut failures = 0u32;
    // The newest frame wins: a frame that arrives while the device is behind replaces the
    // older one, and the writer takes it when it is ready (#308).
    while frames.changed().await.is_ok() {
        let frame = {
            // The borrow ends here: the frame is copied out, so the sender is never held up
            // while the write runs.
            let current = frames.borrow_and_update();
            current.clone()
        };
        let result = match &lane {
            Some(lane) => lane.write_frame_shared(&device, &frame).await,
            None => adapter.lock().await.write_frame(&device, &frame).await,
        };
        match result {
            Ok(()) => {
                failures = 0;
                // Stores a time; logs nothing (#81).
                last_frame.record();
            }
            Err(e) => {
                failures += 1;
                // Log the first failure and then every 300th, not every frame.
                // Keep the daemon's own target (#87): this loop moved out of the binary into the
                // library, and without an explicit target the line's target became
                // `canvas_daemon::writer`, which a `RUST_LOG=canvas_rgbd=...` filter drops.
                if failures == 1 || failures.is_multiple_of(300) {
                    tracing::warn!(
                        target: crate::logging::LOG_TARGET,
                        device = %device,
                        error = %e,
                        failures,
                        "frame write failed"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::{AdapterCapabilities, DeviceInfo};
    use canvas_proto::traits::AdapterError;
    use std::sync::Mutex;

    /// An adapter that takes every frame and remembers it.
    struct FakeAdapter {
        frames: Mutex<Vec<Vec<Rgb8>>>,
    }

    #[async_trait::async_trait]
    impl DeviceAdapter for FakeAdapter {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn capabilities(&self) -> AdapterCapabilities {
            AdapterCapabilities::default()
        }

        async fn connect(&mut self) -> Result<(), AdapterError> {
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), AdapterError> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            true
        }

        async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
            Ok(Vec::new())
        }

        async fn write_frame(&mut self, _device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).push(colors.to_vec());
            Ok(())
        }

        async fn set_zone_size(&mut self, _device: &DeviceId, _zone: u32, _leds: u32) -> Result<(), AdapterError> {
            Ok(())
        }

        async fn identify(&mut self, _device: &DeviceId, _zone: u32, _led: u32) -> Result<(), AdapterError> {
            Ok(())
        }

        async fn read_back(&mut self, _device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
            Ok(Vec::new())
        }
    }

    /// #81: the frame that reached the device is the frame the quit path logs. The writer task
    /// must record it, or the log says "no frame was sent" while the lights were on.
    #[tokio::test]
    async fn a_frame_that_reached_the_device_is_recorded_as_the_last_frame() {
        let adapter = Arc::new(tokio::sync::Mutex::new(
            Box::new(FakeAdapter { frames: Mutex::new(Vec::new()) }) as Box<dyn DeviceAdapter>,
        ));
        let last_frame = Arc::new(LastFrame::default());
        let (tx, rx) = tokio::sync::watch::channel(Vec::new());
        let writer = tokio::spawn(writer_loop(
            adapter,
            DeviceId("fake:ram".into()),
            rx,
            Arc::clone(&last_frame),
        ));

        tx.send_replace(vec![Rgb8::WHITE; 8]);
        // Closing the channel ends the writer, after the frame it already has.
        drop(tx);
        writer.await.expect("the writer ends");

        let (_, text) = crate::test_logs::captured(|| last_frame.log("the test"));
        assert!(text.contains("1 frames since the daemon started"), "{text}");
    }

    /// An adapter whose writes always fail.
    struct FailingAdapter;

    #[async_trait::async_trait]
    impl DeviceAdapter for FailingAdapter {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn capabilities(&self) -> AdapterCapabilities {
            AdapterCapabilities::default()
        }

        async fn connect(&mut self) -> Result<(), AdapterError> {
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), AdapterError> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            true
        }

        async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
            Ok(Vec::new())
        }

        async fn write_frame(
            &mut self,
            _device: &DeviceId,
            _colors: &[Rgb8],
        ) -> Result<(), AdapterError> {
            Err(AdapterError::Io(std::io::Error::other("the write failed")))
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

        async fn read_back(&mut self, _device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
            Ok(Vec::new())
        }
    }

    /// A frame that failed to be written is not recorded (#87): the last frame is the last one
    /// that reached the device, not the last one that was tried. The failure is logged.
    #[test]
    fn a_failed_write_is_logged_and_the_frame_is_not_recorded() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let failing: Box<dyn DeviceAdapter> = Box::new(FailingAdapter);
        let adapter = Arc::new(tokio::sync::Mutex::new(failing));
        let last_frame = Arc::new(LastFrame::default());
        let (tx, rx) = tokio::sync::watch::channel(Vec::new());
        let device = DeviceId("fake:ram".into());
        // The writer runs on this thread, so its warning goes into this capture.
        let (_, text) = crate::test_logs::captured(|| {
            runtime.block_on(async {
                let frame = vec![Rgb8::WHITE; 8];
                tx.send_replace(frame);
                // Closing the channel ends the writer.
                drop(tx);
                writer_loop(adapter, device, rx, Arc::clone(&last_frame)).await;
            });
        });
        let warned = text.contains("WARN") && text.contains("frame write failed");
        assert!(warned, "{text}");

        let (_, text) = crate::test_logs::captured(|| last_frame.log("the test"));
        assert!(
            text.contains("no frame was sent to any device"),
            "the failed frame is not the last frame: {text}"
        );
    }

    /// The frame-write warning keeps the daemon's own target after the writer moved out of the
    /// binary into the library (#87): a `RUST_LOG=canvas_rgbd=...` filter still catches it. On the
    /// old target (`canvas_daemon::writer`) that filter drops every line from this module.
    ///
    /// `captured` prints the target, so the line reads `WARN canvas_rgbd: frame write failed`
    /// (class `cause:timing-test`: a capture reads lines through `test_logs::captured`, never
    /// through a subscriber a test installs itself, #45).
    #[test]
    fn the_frame_write_warning_keeps_the_daemon_target() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let failing: Box<dyn DeviceAdapter> = Box::new(FailingAdapter);
        let adapter = Arc::new(tokio::sync::Mutex::new(failing));
        let last_frame = Arc::new(LastFrame::default());
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        // The writer runs on this thread, so its warning goes into this capture.
        let (_, text) = crate::test_logs::captured(|| {
            runtime.block_on(async {
                tx.send(vec![Rgb8::WHITE; 8]).await.expect("the writer takes the frame");
                drop(tx);
                writer_loop(adapter, DeviceId("fake:ram".into()), rx, last_frame).await;
            });
        });
        let target = crate::logging::LOG_TARGET;
        assert!(
            text.lines().any(|line| line.contains(&format!("WARN {target}:"))),
            "the frame-write warning must use the {target} target: {text}"
        );
    }
}
