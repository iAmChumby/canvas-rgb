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
pub async fn writer_loop(
    adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    device: DeviceId,
    mut frames: tokio::sync::mpsc::Receiver<Vec<Rgb8>>,
    last_frame: Arc<LastFrame>,
) {
    let mut failures = 0u32;
    while let Some(frame) = frames.recv().await {
        let result = adapter.lock().await.write_frame(&device, &frame).await;
        match result {
            Ok(()) => {
                failures = 0;
                // Stores a time; logs nothing (#81).
                last_frame.record();
            }
            Err(e) => {
                failures += 1;
                // Log the first failure and then every 300th, not every frame.
                if failures == 1 || failures.is_multiple_of(300) {
                    tracing::warn!(device = %device, error = %e, failures, "frame write failed");
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
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let writer = tokio::spawn(writer_loop(
            adapter,
            DeviceId("fake:ram".into()),
            rx,
            Arc::clone(&last_frame),
        ));

        tx.send(vec![Rgb8::WHITE; 8]).await.expect("the writer takes the frame");
        // Closing the channel ends the writer, after the frame it already has.
        drop(tx);
        writer.await.expect("the writer ends");

        let (_, text) = crate::test_logs::captured(|| last_frame.log("the test"));
        assert!(text.contains("1 frames since the daemon started"), "{text}");
    }
}
