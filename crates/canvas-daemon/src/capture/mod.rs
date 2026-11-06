//! The daemon's capture inputs: screen (Desktop Duplication) and audio (WASAPI loopback).
//!
//! Both start only while the active effect asks for them and stop when it does not, so
//! an idle daemon does no capture work at all.
//!
//! The capture is one shared value: the daemon hands it to the engines as their input
//! source, so the screen and the audio reach the lights (#1). The captures are behind
//! locks, so the engines can hold the capture while the daemon starts and stops the
//! captures around them.

pub mod audio;
pub mod screen;

use canvas_proto::props::InputNeeds;
use canvas_proto::traits::{AudioSpectrum, InputSource, ScreenFrame};
use std::sync::{Arc, Mutex};

/// The daemon's `InputSource`: it hands the engine the latest screen frame and audio
/// spectrum, if any are being captured.
pub struct DaemonInputs {
    /// The screen capture, while it runs.
    screen: Mutex<Option<screen::Shared>>,
    /// The audio capture, while it runs.
    audio: Mutex<Option<audio::Shared>>,
    /// False in fakes mode (#199): captures never start, so the screen and
    /// audio are never touched.
    enabled: bool,
}

impl DaemonInputs {
    /// An input source with nothing captured.
    pub fn empty() -> Self {
        Self {
            screen: Mutex::new(None),
            audio: Mutex::new(None),
            enabled: true,
        }
    }

    /// An input source that never captures: fakes mode (#199).
    pub fn disabled() -> Self {
        Self {
            screen: Mutex::new(None),
            audio: Mutex::new(None),
            enabled: false,
        }
    }

    /// Whether this input source may start a capture.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Starts the screen capture.
    pub fn start_screen(&self) {
        if !self.enabled {
            return;
        }
        let mut slot = self.screen.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return;
        }
        match screen::start_primary() {
            Ok(shared) => {
                tracing::info!("the screen capture is running");
                *slot = Some(shared);
            }
            Err(e) => tracing::warn!(error = %e, "the screen capture could not start"),
        }
    }

    /// Stops the screen capture.
    pub fn stop_screen(&self) {
        let mut slot = self.screen.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(shared) = slot.take() {
            shared.stop();
            tracing::info!("the screen capture stopped");
        }
    }

    /// Starts the audio capture.
    pub fn start_audio(&self) {
        if !self.enabled {
            return;
        }
        let mut slot = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return;
        }
        #[cfg(windows)]
        match audio::wasapi::start_default_render() {
            Ok(shared) => {
                tracing::info!("the audio capture is running");
                *slot = Some(shared);
            }
            Err(e) => tracing::warn!(error = %e, "the audio capture could not start"),
        }
        #[cfg(not(windows))]
        tracing::warn!("audio capture is Windows-only");
    }

    /// Stops the audio capture.
    pub fn stop_audio(&self) {
        let mut slot = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(shared) = slot.take() {
            shared.stop();
            tracing::info!("the audio capture stopped");
        }
    }

    /// Starts and stops the captures so they match what the active effect needs.
    pub fn apply_needs(&self, needs: InputNeeds) {
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
    pub fn stop_all(&self) {
        self.stop_screen();
        self.stop_audio();
    }

    /// True when nothing is being captured.
    pub fn idle(&self) -> bool {
        let screen = self.screen.lock().unwrap_or_else(|e| e.into_inner());
        let audio = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        screen.is_none() && audio.is_none()
    }
}

impl InputSource for DaemonInputs {
    fn screen(&self) -> Option<Arc<ScreenFrame>> {
        let slot = self.screen.lock().unwrap_or_else(|e| e.into_inner());
        slot.as_ref().and_then(|shared| shared.latest())
    }

    fn audio(&self) -> Option<Arc<AudioSpectrum>> {
        let slot = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        slot.as_ref().and_then(|shared| shared.latest())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_start_and_stop_the_captures() {
        let inputs = DaemonInputs::empty();
        assert!(inputs.idle());
        // Starting the captures on a machine with no session may fail; what matters is
        // that the calls do not panic and the intent is remembered.
        inputs.apply_needs(InputNeeds {
            screen: false,
            audio: false,
        });
        assert!(inputs.idle());
        inputs.stop_all();
        assert!(inputs.idle());
    }

    #[test]
    fn a_disabled_input_source_never_captures() {
        let inputs = DaemonInputs::disabled();
        assert!(!inputs.enabled());
        inputs.apply_needs(InputNeeds { screen: true, audio: true });
        assert!(inputs.idle(), "fakes mode must not start the screen or audio capture");
        assert!(DaemonInputs::empty().enabled());
    }

    /// The capture is the daemon's input source: the engines read the screen and the
    /// audio through it (#1).
    #[test]
    fn the_capture_is_an_input_source() {
        let inputs = DaemonInputs::empty();
        let source: Arc<dyn InputSource> = Arc::new(inputs);
        // Nothing is captured yet, so the source has nothing to give.
        assert!(source.screen().is_none());
        assert!(source.audio().is_none());
    }
}
