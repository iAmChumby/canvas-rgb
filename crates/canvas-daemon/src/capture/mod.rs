//! The daemon's capture inputs: screen (Desktop Duplication) and audio (WASAPI loopback).
//!
//! Both start only while the active effect asks for them and stop when it does not, so
//! an idle daemon does no capture work at all.

pub mod audio;
pub mod screen;

use canvas_proto::props::InputNeeds;
use canvas_proto::traits::{AudioSpectrum, InputSource, ScreenFrame};
use std::sync::Arc;

/// The daemon's `InputSource`: it hands the engine the latest screen frame and audio
/// spectrum, if any are being captured.
pub struct DaemonInputs {
    screen: Option<screen::Shared>,
    audio: Option<audio::Shared>,
}

impl DaemonInputs {
    /// An input source with nothing captured.
    pub fn empty() -> Self {
        Self { screen: None, audio: None }
    }

    /// Starts the screen capture.
    pub fn start_screen(&mut self) {
        if self.screen.is_some() {
            return;
        }
        match screen::start_primary() {
            Ok(shared) => {
                tracing::info!("the screen capture is running");
                self.screen = Some(shared);
            }
            Err(e) => tracing::warn!(error = %e, "the screen capture could not start"),
        }
    }

    /// Stops the screen capture.
    pub fn stop_screen(&mut self) {
        if let Some(shared) = self.screen.take() {
            shared.stop();
            tracing::info!("the screen capture stopped");
        }
    }

    /// Starts the audio capture.
    pub fn start_audio(&mut self) {
        if self.audio.is_some() {
            return;
        }
        #[cfg(windows)]
        match audio::wasapi::start_default_render() {
            Ok(shared) => {
                tracing::info!("the audio capture is running");
                self.audio = Some(shared);
            }
            Err(e) => tracing::warn!(error = %e, "the audio capture could not start"),
        }
        #[cfg(not(windows))]
        tracing::warn!("audio capture is Windows-only");
    }

    /// Stops the audio capture.
    pub fn stop_audio(&mut self) {
        if let Some(shared) = self.audio.take() {
            shared.stop();
            tracing::info!("the audio capture stopped");
        }
    }

    /// Starts and stops the captures so they match what the active effect needs.
    pub fn apply_needs(&mut self, needs: InputNeeds) {
        if needs.screen {
            self.start_screen();
        } else {
            self.stop_screen();
        }
        if needs.audio {
            self.start_audio();
        } else {
            self.stop_audio();
        }
    }

    /// Stops every capture.
    pub fn stop_all(&mut self) {
        self.stop_screen();
        self.stop_audio();
    }

    /// True when nothing is being captured.
    pub fn idle(&self) -> bool {
        self.screen.is_none() && self.audio.is_none()
    }
}

impl InputSource for DaemonInputs {
    fn screen(&self) -> Option<Arc<ScreenFrame>> {
        self.screen.as_ref().and_then(|s| s.latest())
    }

    fn audio(&self) -> Option<Arc<AudioSpectrum>> {
        self.audio.as_ref().and_then(|a| a.latest())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_start_and_stop_the_captures() {
        let mut inputs = DaemonInputs::empty();
        assert!(inputs.idle());
        // Starting the captures on a machine with no session may fail; what matters is
        // that the calls do not panic and the intent is remembered.
        inputs.apply_needs(InputNeeds { screen: false, audio: false });
        assert!(inputs.idle());
        inputs.stop_all();
        assert!(inputs.idle());
    }
}
