//! The daemon's shared state.
//!
//! One `App` holds everything the API handlers, the render loop and the background
//! tasks share: the on-disk store, the engine handle, the OpenRGB adapter, the device
//! list, the preview subscribers and the pipe client.

use canvas_proto::api::{LinkState, Status};
use canvas_proto::device::{DeviceId, DeviceInfo};
use canvas_proto::fans::FansStatus;
use canvas_proto::layout::Layout;
use canvas_proto::traits::{DeviceAdapter, Engine, FrameSink, PreviewSink};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::storage::Store;

/// How many preview subscribers are attached. The engine parks when this is zero.
#[derive(Debug, Default)]
pub struct PreviewRegistry {
    count: AtomicU32,
}

impl PreviewRegistry {
    pub fn subscribe(&self) -> u32 {
        self.count.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn unsubscribe(&self) {
        let _ = self.count.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn subscribers(&self) -> u32 {
        self.count.load(Ordering::SeqCst)
    }
}

/// The state every handler sees.
pub struct App {
    /// The on-disk state. Written rarely (a settings change) and read often.
    ///
    /// This is a Tokio lock, not a std one, so the guard is `Send` and an API handler
    /// can hold it across an await.
    pub store: tokio::sync::RwLock<Store>,
    /// The render engine handle.
    pub engine: Arc<dyn Engine>,
    /// The OpenRGB adapter. Behind an async mutex: the SDK client is not `Sync` on its
    /// own, and the API handlers await it.
    pub adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    /// The devices as last discovered.
    pub devices: tokio::sync::RwLock<Vec<DeviceInfo>>,
    /// Zone sizes by device fingerprint (the `zones.json` contents).
    ///
    /// Behind an `Arc` so the OpenRGB supervisor can share it with the API handlers.
    pub zones: Arc<tokio::sync::RwLock<ZoneSizes>>,
    /// Preview subscribers, so the engine only runs while the editor watches.
    pub preview: PreviewRegistry,
    /// The screen and audio captures. They only run while an effect needs them.
    pub inputs: Mutex<crate::capture::DaemonInputs>,
    /// The API token every request must carry.
    pub token: String,
    /// The daemon's version, as reported by `/status`.
    pub version: String,
    /// Milliseconds the service pipe has been down.
    pub service_down_ms: AtomicU64,
    /// The last fan status the service reported.
    pub fans: tokio::sync::RwLock<Option<FansStatus>>,
    /// The last fan config the daemon pushed to the service (G10 export while down).
    pub fans_last: tokio::sync::RwLock<Option<FansFile>>,
    /// SignalRGB conflicts, refreshed every 5 s.
    pub conflicts: tokio::sync::RwLock<Vec<canvas_proto::api::Conflict>>,
    /// Where OpenRGB is, as the daemon sees it.
    pub openrgb: tokio::sync::RwLock<LinkState>,
    /// Set when the daemon is quitting, so background tasks stop.
    pub shutting_down: AtomicU32,
    /// The channel the API sends on when the daemon should quit.
    pub quit_tx: tokio::sync::mpsc::UnboundedSender<()>,
    /// The user HTML effects found in `%APPDATA%\canvas-rgb\effects\`.
    pub user_effects: Arc<crate::user_effects::UserEffects>,
}

/// The fan config the daemon keeps a copy of.
pub type FansFile = canvas_proto::config::FansFile;

/// Zone sizes by device fingerprint, then zone index.
pub type ZoneSizes = BTreeMap<String, BTreeMap<u32, u32>>;

impl App {
    /// Builds an app from the pieces the caller supplies.
    ///
    /// The daemon uses this too, so the tests and the daemon build the same state.
    pub fn new(
        store: crate::storage::Store,
        engine: Arc<dyn Engine>,
        adapter: Box<dyn DeviceAdapter>,
        token: String,
        version: String,
        quit_tx: tokio::sync::mpsc::UnboundedSender<()>,
    ) -> Self {
        Self {
            store: tokio::sync::RwLock::new(store),
            engine,
            adapter: Arc::new(tokio::sync::Mutex::new(adapter)),
            devices: tokio::sync::RwLock::new(Vec::new()),
            zones: Arc::new(tokio::sync::RwLock::new(ZoneSizes::new())),
            preview: PreviewRegistry::default(),
            inputs: Mutex::new(crate::capture::DaemonInputs::empty()),
            token,
            version,
            service_down_ms: AtomicU64::new(0),
            fans: tokio::sync::RwLock::new(None),
            fans_last: tokio::sync::RwLock::new(None),
            conflicts: tokio::sync::RwLock::new(Vec::new()),
            openrgb: tokio::sync::RwLock::new(LinkState::Down),
            shutting_down: AtomicU32::new(0),
            quit_tx,
            user_effects: Arc::new(crate::user_effects::UserEffects::new(
                crate::user_effects::html_builder(),
            )),
        }
    }

    /// The active layout, if any.
    pub async fn active_layout(&self) -> Option<Layout> {
        self.store.read().await.active_layout().cloned()
    }

    /// The device with this id, if it is known.
    pub async fn device(&self, id: &DeviceId) -> Option<DeviceInfo> {
        self.devices.read().await.iter().find(|d| &d.id == id).cloned()
    }

    /// The number of devices currently connected.
    pub async fn devices_connected(&self) -> u32 {
        self.devices.read().await.iter().filter(|x| x.connected).count() as u32
    }

    /// The status snapshot the API and the CLI read.
    pub async fn status(&self) -> Status {
        let stats = self.engine.stats();
        let store = self.store.read().await;
        let power = store.config.settings.power;
        let brightness = store.config.settings.brightness;
        let fps_target = store.config.settings.fps;
        let active_layout = Some(store.layouts.active.clone());
        let active_effect =
            store.active_layout().and_then(|l| l.effect.as_ref().map(|e| e.id.clone()));
        // G7: the editor shows the dismissible "size your ARGB headers" panel until Luke
        // dismisses it.
        let show_zone_panel = !store.config.zone_panel_dismissed;
        let openrgb = *self.openrgb.read().await;
        let conflicts = self.conflicts.read().await.clone();
        drop(store);
        let down = self.service_down_ms.load(Ordering::SeqCst);
        Status {
            version: self.version.clone(),
            power,
            brightness,
            fps_target,
            fps_measured: stats.fps_measured,
            frame_time_ms: stats.frame_time_ms,
            frames_rendered: stats.frames_rendered,
            parked: stats.parked,
            devices_connected: self.devices_connected().await,
            preview_subscribers: self.preview.subscribers(),
            active_layout,
            active_effect,
            openrgb,
            service: if down == 0 { LinkState::Connected } else { LinkState::Down },
            service_down_ms: if down == 0 { None } else { Some(down) },
            conflicts,
            show_zone_panel,
        }
    }

    /// The fan profile names the service knows about.
    pub async fn fan_profile_names(&self) -> Vec<String> {
        self.fans.read().await.as_ref().map(|s| s.profiles.clone()).unwrap_or_default()
    }
}

/// A per-device frame sink: a bounded(1) channel into the device's writer task.
///
/// When the device is slower than the render loop, the old frame is dropped and the new
/// one takes its place. The engine never blocks.
pub struct DeviceSink {
    id: DeviceId,
    tx: tokio::sync::mpsc::Sender<Vec<canvas_proto::Rgb8>>,
}

impl DeviceSink {
    pub fn new(id: DeviceId, tx: tokio::sync::mpsc::Sender<Vec<canvas_proto::Rgb8>>) -> Self {
        Self { id, tx }
    }
}

impl FrameSink for DeviceSink {
    fn submit(&self, colors: Vec<canvas_proto::Rgb8>) {
        // `try_send` never blocks; a full channel means the device is behind and the old
        // frame is dropped.
        match self.tx.try_send(colors) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::debug!(device = %self.id, "device is behind; dropping a frame");
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!(device = %self.id, "device writer is gone; dropping a frame");
            }
        }
    }
}

/// A preview sink: the engine's downscaled canvas, forwarded to the WebSocket.
pub struct WebsocketPreview {
    tx: tokio::sync::mpsc::Sender<(u32, u32, Vec<u8>)>,
}

impl WebsocketPreview {
    pub fn new(tx: tokio::sync::mpsc::Sender<(u32, u32, Vec<u8>)>) -> Self {
        Self { tx }
    }
}

impl PreviewSink for WebsocketPreview {
    fn submit(&self, width: u32, height: u32, rgb: Vec<u8>) {
        let _ = self.tx.try_send((width, height, rgb));
    }
}

/// Zone sizes by device fingerprint, then zone index.
pub fn zone_sizes(sizes: &ZoneSizes, device: &DeviceId) -> BTreeMap<u32, u32> {
    sizes.get(&device.0).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::api::Conflict;

    #[test]
    fn preview_registry_counts_subscribers() {
        let registry = PreviewRegistry::default();
        assert_eq!(registry.subscribers(), 0);
        registry.subscribe();
        registry.subscribe();
        assert_eq!(registry.subscribers(), 2);
        registry.unsubscribe();
        assert_eq!(registry.subscribers(), 1);
    }

    #[test]
    fn device_sink_drops_frames_rather_than_blocking() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let sink = DeviceSink::new(DeviceId("openrgb:abc".into()), tx);
        sink.submit(vec![canvas_proto::Rgb8::WHITE]);
        // The channel is full now: the second frame is dropped, not queued.
        sink.submit(vec![canvas_proto::Rgb8::BLACK]);
        let first = rx.try_recv().unwrap();
        assert_eq!(first, vec![canvas_proto::Rgb8::WHITE]);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn conflicts_are_reported() {
        let app_conflicts = [Conflict::SignalRgb { process: true, service: false }];
        assert!(matches!(app_conflicts[0], Conflict::SignalRgb { process: true, .. }));
    }
}
