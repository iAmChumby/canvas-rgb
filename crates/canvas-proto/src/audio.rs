//! Issue #166: which audio source the audio-reactive effects read, and the live test that shows
//! what Windows delivers from it. Design: `.build/debate-166/CONSENSUS.md` in the main checkout.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which way the audio flows at an endpoint. An output is captured as loopback (what it plays);
/// an input is captured directly (a microphone, a line-in, a virtual cable's recording side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AudioFlow {
    /// A playback endpoint (`eRender`), captured with `AUDCLNT_STREAMFLAGS_LOOPBACK`.
    Render,
    /// A recording endpoint (`eCapture`).
    Capture,
}

/// The audio source the owner picked. Stored on `ConfigFile`, outside `Settings`, so a
/// whole-object `PUT /settings` can never overwrite it; `PUT /api/v1/audio/source` is its only
/// writer. Old configs without it load as [`AudioSource::FollowDefault`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AudioSource {
    /// The Windows default output (`eRender`, `eConsole`), followed when it changes.
    #[default]
    FollowDefault,
    /// One endpoint, pinned by its opaque `IMMDevice::GetId`. `name_hint` is the friendly name
    /// when it was picked, shown while the endpoint is not connected.
    Endpoint { id: String, flow: AudioFlow, name_hint: String },
}

/// Whether a listed endpoint can be opened now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AudioEndpointState {
    Active,
    /// The saved endpoint, listed so the picker can name it; it is unplugged, disabled or gone.
    NotConnected,
}

/// One row of the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioEndpoint {
    pub id: String,
    pub flow: AudioFlow,
    /// `PKEY_Device_FriendlyName`, else `PKEY_Device_DeviceDesc`, else the id.
    pub name: String,
    /// The Windows default for its flow (console role).
    pub is_default: bool,
    pub state: AudioEndpointState,
}

/// Why the capture is not on the endpoint the owner picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum AudioFallback {
    /// The pinned endpoint is not connected; the default output is captured instead.
    PickedMissing,
    /// The pinned endpoint could not be opened (the message names the HRESULT).
    OpenFailed { message: String },
}

/// What the capture resolved the owner's choice to. The daemon's log lines and the API both read
/// this one value, so they cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResolvedAudioSource {
    pub requested: AudioSource,
    /// The endpoint the capture opened or will open; `None` when there is no endpoint at all.
    pub endpoint_id: Option<String>,
    pub name: Option<String>,
    pub flow: Option<AudioFlow>,
    pub fallback: Option<AudioFallback>,
}

/// `GET /api/v1/audio/sources`. Enumerated on demand (and after a device notification), never on
/// a timer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioSources {
    pub endpoints: Vec<AudioEndpoint>,
    pub current: ResolvedAudioSource,
}

/// The live test's honest state. The editor renders each one as words, never a blank bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AudioTestStatus {
    /// No lease and no effect needs audio: nothing is captured.
    Off,
    /// The capture is opening.
    Waiting,
    /// An input is picked and Listen is off: an input is never opened just by visiting Settings.
    NotListening,
    /// No active endpoint of any kind.
    NoDevice,
    /// The pinned endpoint is not connected (see `fallback`).
    Missing,
    /// `Initialize`/`Start` failed with `AUDCLNT_E_DEVICE_IN_USE`: another app holds it exclusively.
    HeldExclusive,
    /// The client runs but no packets arrive.
    NoPackets,
    /// Packets arrive and the peak is under the silence floor.
    Silent,
    Live,
    /// The Windows default output changed and the capture moved to it.
    DefaultChanged,
    /// The endpoint was invalidated mid-run; the capture is reopening it.
    Reopening,
    /// The mix format is one the capture cannot decode (see `format_tag`).
    UnsupportedFormat,
}

/// `PUT /api/v1/audio/test` answer: a test lease. Every `GET /api/v1/audio/test?lease=<id>`
/// renews it; it ends [`crate::consts::AUDIO_TEST_LEASE`] after its last renewal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioTestLease {
    pub lease_id: String,
}

/// `PUT /api/v1/audio/test` body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioTestRequest {
    /// Listen to a picked input. Outputs are tested without it; an input needs it.
    #[serde(default)]
    pub listen: bool,
}

/// `GET /api/v1/audio/test?lease=<id>`: what Windows is delivering now. A lease that is unknown or
/// expired answers 410, and the editor asks for a new one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioTestState {
    pub status: AudioTestStatus,
    pub source: ResolvedAudioSource,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub bits_per_sample: Option<u16>,
    pub format_tag: Option<u16>,
    /// `IAudioMeterInformation` peak on the same endpoint, 0..1, before endpoint volume.
    /// `None` when there is no meter (no endpoint, or exclusive mode where it reads 0.0).
    pub peak: Option<f32>,
    /// `peak` in dBFS, floored at [`crate::consts::AUDIO_DB_FLOOR`]; never NaN.
    pub peak_db: Option<f32>,
    /// The bands the effects read, 0..1 (dB-mapped). Empty when nothing is captured.
    pub bands: Vec<f32>,
    pub packets: u64,
    pub silent_packets: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_default_is_the_default_and_round_trips() {
        assert_eq!(AudioSource::default(), AudioSource::FollowDefault);
        let json = serde_json::to_string(&AudioSource::FollowDefault).unwrap();
        assert_eq!(json, r#"{"kind":"follow_default"}"#);
        let pinned = AudioSource::Endpoint { id: "{0.0.0}.{abc}".into(), flow: AudioFlow::Render, name_hint: "Speakers".into() };
        let back: AudioSource = serde_json::from_str(&serde_json::to_string(&pinned).unwrap()).unwrap();
        assert_eq!(back, pinned);
    }

    #[test]
    fn a_test_request_without_listen_is_not_listening() {
        let r: AudioTestRequest = serde_json::from_str("{}").unwrap();
        assert!(!r.listen);
    }
}
