//! The peripherals: the mouse and the game controller canvas-rgb can read and configure.
//!
//! A peripheral is a device with settings and no lights of its own. The daemon never opens one
//! it was not told to: the vendor app may own it, and both would write the same device. What
//! the daemon may do with a device is saved per device id in `peripherals.json` (spine:
//! `routes::PERIPHERAL_ACCESS`):
//!
//! - `reads`: open the device and read its settings. A read may send the device a request that
//!   changes no setting.
//! - `writes`: send config writes. It needs `reads`: a write patches what the device last
//!   reported.
//!
//! Both are off until the owner turns them on. A device that is not in `peripherals.json` has
//! both off, so it is enumerated and never opened.
//!
//! The enumeration (`GET /peripherals`) never opens anything: the adapters read the HID
//! metadata of the machine and do no I/O on the device itself. The hub asks an adapter who owns
//! a device with `describe`, which is metadata only too, so the device that is opened is always
//! the one the owner asked for.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use canvas_proto::device::DeviceId;
use canvas_proto::peripheral::{PeripheralAccess, PeripheralInfo, PeripheralSchema};
use canvas_proto::props::Props;
use canvas_proto::traits::{AdapterError, PeripheralAdapter};

use crate::state::App;

/// The file the access settings live in, next to `displays.json` and `layouts.json`.
pub const PERIPHERALS_FILE: &str = "peripherals.json";
/// The schema version of `peripherals.json`.
pub const PERIPHERALS_SCHEMA: u32 = 1;

/// The enumeration the routes report is cached this long, so a page that polls does not
/// enumerate on every request.
pub const DISCOVERY_CACHE: Duration = Duration::from_secs(5);

/// A settings read is shared this long. Opening the Peripherals view can ask for the same device
/// more than once (a reload, a second window, a page that refetches); each ask would be another
/// run of feature-report round trips over the receiver's radio link (issue #7), so the asks
/// inside this window share one read. A write or an access change drops it at once.
pub const READ_CACHE: Duration = Duration::from_secs(2);

/// The message the detail carries when the values are missing because reads are off.
pub const READS_OFF: &str = "reads are off for this device";

/// `peripherals.json`: what the daemon may do with each device.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeripheralsFile {
    pub schema_version: u32,
    /// The access per device id. A device that is not in here has both off.
    #[serde(default)]
    pub peripherals: BTreeMap<String, PeripheralAccess>,
}

impl Default for PeripheralsFile {
    fn default() -> Self {
        Self { schema_version: PERIPHERALS_SCHEMA, peripherals: BTreeMap::new() }
    }
}

impl PeripheralsFile {
    /// What the daemon may do with this device. An id that was never saved has both off.
    pub fn access(&self, id: &DeviceId) -> PeripheralAccess {
        self.peripherals.get(&id.0).copied().unwrap_or_default()
    }

    /// Saves the access for one device.
    pub fn set(&mut self, id: &DeviceId, access: PeripheralAccess) {
        self.peripherals.insert(id.0.clone(), access);
    }
}

/// Writes `peripherals.json` in `home`, atomically.
pub fn save(home: &Path, file: &PeripheralsFile) -> std::io::Result<()> {
    crate::storage::write_json(&home.join(PERIPHERALS_FILE), file)
}

/// A peripheral adapter whose config writes can be turned on and off.
///
/// The spine's `PeripheralAdapter` has no gate, but both adapters have one (an inherent
/// `set_writes_enabled`). This trait is the daemon's bridge to it: the daemon turns the gate
/// with the saved access, so nothing ever reaches a device the owner did not allow.
pub trait Peripheral: PeripheralAdapter {
    /// Turns config writes on or off. Nothing is written to the hardware while they are off.
    fn set_config_writes(&mut self, on: bool);
}

impl Peripheral for canvas_adapters::attack_shark_x6::AttackSharkX6 {
    fn set_config_writes(&mut self, on: bool) {
        self.set_writes_enabled(on);
    }
}

impl Peripheral for canvas_adapters::eightbitdo_ultimate2::EightBitDoUltimate2 {
    fn set_config_writes(&mut self, on: bool) {
        self.set_writes_enabled(on);
    }
}

/// The adapters the daemon has, and the last enumeration.
pub struct PeripheralHub {
    /// The adapters. Behind a lock: every call needs `&mut` on the adapter, and a read of a
    /// device can take a while.
    adapters: tokio::sync::Mutex<Vec<Box<dyn Peripheral>>>,
    /// The last enumeration, and when it was taken.
    discovery: tokio::sync::Mutex<Option<Discovery>>,
    /// The last good read of each device, and when it was taken. Only touched while the
    /// `adapters` lock is held, so concurrent reads of one device queue behind the first and
    /// find its answer here (single-flight).
    reads: std::sync::Mutex<BTreeMap<DeviceId, (Instant, Props)>>,
    /// How long a read is shared. [`READ_CACHE`] in the daemon; the tests set it to zero to
    /// count the reads the hub made before it shared any.
    read_cache: Duration,
}

/// One enumeration and the moment it was taken.
struct Discovery {
    at: Instant,
    found: Vec<PeripheralInfo>,
}

impl PeripheralHub {
    /// The hub with these adapters. The daemon builds it with the real adapters; the tests
    /// build it with fakes.
    pub fn new(adapters: Vec<Box<dyn Peripheral>>) -> Self {
        Self::with_read_cache(adapters, READ_CACHE)
    }

    /// The hub with its own read-sharing window. Zero shares nothing: every read goes to the
    /// device, as it did before the window existed.
    pub fn with_read_cache(adapters: Vec<Box<dyn Peripheral>>, read_cache: Duration) -> Self {
        Self {
            read_cache,
            adapters: tokio::sync::Mutex::new(adapters),
            discovery: tokio::sync::Mutex::new(None),
            reads: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// Every peripheral the adapters see, cached for [`DISCOVERY_CACHE`].
    ///
    /// The enumeration never opens a device (see the module doc), so a listed peripheral is
    /// not a peripheral the daemon may touch: only the ones the owner turned reads on for are
    /// ever opened.
    pub async fn discover(&self) -> Vec<PeripheralInfo> {
        let mut cache = self.discovery.lock().await;
        if let Some(cached) = cache.as_ref() {
            if cached.at.elapsed() < DISCOVERY_CACHE {
                return cached.found.clone();
            }
        }
        let found = self.enumerate().await;
        *cache = Some(Discovery { at: Instant::now(), found: found.clone() });
        found
    }

    /// A fresh enumeration, ignoring the cache. Used when a device was not in the last one.
    pub async fn discover_now(&self) -> Vec<PeripheralInfo> {
        let found = self.enumerate().await;
        let mut cache = self.discovery.lock().await;
        *cache = Some(Discovery { at: Instant::now(), found: found.clone() });
        found
    }

    /// Runs every adapter's enumeration. One that fails does not stop the others.
    async fn enumerate(&self) -> Vec<PeripheralInfo> {
        let mut adapters = self.adapters.lock().await;
        let mut found = Vec::new();
        for adapter in adapters.iter_mut() {
            let name = adapter.name();
            match adapter.discover().await {
                Ok(list) => found.extend(list),
                Err(error) => {
                    tracing::warn!(adapter = name, error = %error, "could not enumerate the peripherals");
                }
            }
        }
        found
    }

    /// What the device has: its sections, its properties and their ranges.
    ///
    /// `describe` is metadata only, so no device is opened here.
    pub async fn describe(&self, device: &DeviceId) -> Result<PeripheralSchema, AdapterError> {
        let mut adapters = self.adapters.lock().await;
        let (_, schema) = owner_of(&mut adapters, device).await?;
        Ok(schema)
    }

    /// The device's settings. The device is opened first if it is not open.
    ///
    /// Asks inside [`READ_CACHE`] of the last good read share its answer: one view-open is one
    /// run of feature reports to the device, however many requests it makes. A write or an
    /// access change drops the shared answer, and a failed read is never shared. A read that
    /// fails because the device is gone closes it, so the next read opens it again.
    pub async fn read(&self, device: &DeviceId) -> Result<Props, AdapterError> {
        // One lock for all adapters. A blocking open inside an adapter holds it, so a slow open
        // of one device queues the reads of the others behind it (the adapters' own HID caches
        // are per adapter and do not share this).
        let mut adapters = self.adapters.lock().await;
        let (index, _) = owner_of(&mut adapters, device).await?;
        let adapter = adapters[index].as_mut();
        // A shared read of a device that has since been closed is not an answer. `is_connected`
        // only says the adapter still holds its handle: an unplug shows on the next I/O, so for
        // up to `read_cache` after an unplug the shared values are served as they were read.
        if adapter.is_connected() {
            if let Some(props) = self.shared_read(device) {
                return Ok(props);
            }
        }
        if !adapter.is_connected() {
            adapter.connect().await?;
        }
        let result = adapter.read_settings(device).await;
        match &result {
            Ok(props) => {
                self.reads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(device.clone(), (Instant::now(), props.clone()));
            }
            Err(error) if device_failure(error) => {
                let _ = adapter.disconnect().await;
            }
            Err(_) => {}
        }
        result
    }

    /// The last good read of this device, when it is younger than [`READ_CACHE`].
    fn shared_read(&self, device: &DeviceId) -> Option<Props> {
        let reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        reads
            .get(device)
            .filter(|(at, _)| at.elapsed() < self.read_cache)
            .map(|(_, props)| props.clone())
    }

    /// Forgets the shared read of this device: the device changed, or is about to.
    fn forget_read(&self, device: &DeviceId) {
        self.reads.lock().unwrap_or_else(|e| e.into_inner()).remove(device);
    }

    /// Writes the given settings to the device. Only the keys present are written.
    ///
    /// Nothing reaches the hardware while the adapter's write gate is off: the gate follows
    /// the saved access.
    pub async fn write(&self, device: &DeviceId, props: &Props) -> Result<(), AdapterError> {
        let mut adapters = self.adapters.lock().await;
        let (index, _) = owner_of(&mut adapters, device).await?;
        let adapter = adapters[index].as_mut();
        self.forget_read(device);
        if !adapter.is_connected() {
            adapter.connect().await?;
        }
        let result = adapter.write_settings(device, props).await;
        if let Err(error) = &result {
            if device_failure(error) {
                let _ = adapter.disconnect().await;
            }
        }
        result
    }

    /// Applies the access to a device: opens it when reads are on, closes it when they are
    /// off, and follows the access with the adapter's write gate.
    ///
    /// Turning reads off closes the device, so the vendor app can take it back. Turning writes
    /// off turns the gate off, so nothing reaches the hardware afterwards.
    pub async fn apply(&self, device: &DeviceId, access: PeripheralAccess) -> Result<(), AdapterError> {
        let mut adapters = self.adapters.lock().await;
        let (index, _) = owner_of(&mut adapters, device).await?;
        let adapter = adapters[index].as_mut();
        self.forget_read(device);
        // The gate follows the access, whatever the access is. Writes need reads, so an
        // access that says otherwise keeps the writes off.
        adapter.set_config_writes(access.writes && access.reads);
        if access.reads {
            if !adapter.is_connected() {
                adapter.connect().await?;
            }
        } else if adapter.is_connected() {
            adapter.disconnect().await?;
        }
        Ok(())
    }
}

/// The adapter that owns this device, and its schema.
///
/// The owner is found with `describe`, which is metadata only: no device is opened while the
/// adapters are asked who owns what. An adapter that does not know the id is skipped.
async fn owner_of(
    adapters: &mut [Box<dyn Peripheral>],
    device: &DeviceId,
) -> Result<(usize, PeripheralSchema), AdapterError> {
    for (index, adapter) in adapters.iter_mut().enumerate() {
        match adapter.describe(device).await {
            Ok(schema) => return Ok((index, schema)),
            Err(AdapterError::UnknownDevice(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(AdapterError::UnknownDevice(device.clone()))
}

/// True when the error means the device is gone or the link failed, not that the request was
/// wrong.
fn device_failure(error: &AdapterError) -> bool {
    matches!(error, AdapterError::Io(_) | AdapterError::NotConnected)
}

/// Applies the saved access to every device that has one.
///
/// The devices the owner turned reads on are opened; the others are left closed. A device that
/// is not here is a warning, not an error: it comes back with the access it was given.
pub async fn apply_saved(app: &App) {
    let saved: Vec<(DeviceId, PeripheralAccess)> = {
        let store = app.store.read().await;
        store
            .peripherals
            .peripherals
            .iter()
            .map(|(id, access)| (DeviceId(id.clone()), *access))
            .collect()
    };
    for (device, access) in saved {
        match app.peripherals.apply(&device, access).await {
            Ok(()) => tracing::info!(device = %device, reads = access.reads, writes = access.writes, "the saved peripheral access was applied"),
            Err(error) => tracing::warn!(device = %device, error = %error, "the saved peripheral access could not be applied"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_device_has_both_off() {
        let file = PeripheralsFile::default();
        let access = file.access(&DeviceId("fake:one".into()));
        assert!(!access.reads);
        assert!(!access.writes);
    }

    #[test]
    fn the_access_is_saved_per_device_id() {
        let mut file = PeripheralsFile::default();
        let device = DeviceId("fake:one".into());
        file.set(&device, PeripheralAccess { reads: true, writes: true });
        assert_eq!(file.access(&device), PeripheralAccess { reads: true, writes: true });
        // Another device is untouched.
        assert_eq!(file.access(&DeviceId("fake:two".into())), PeripheralAccess::default());
    }

    #[test]
    fn the_file_round_trips_through_json() {
        let mut file = PeripheralsFile::default();
        file.set(
            &DeviceId("fake:one".into()),
            PeripheralAccess { reads: true, writes: false },
        );
        let text = serde_json::to_string(&file).expect("serializes");
        let back: PeripheralsFile = serde_json::from_str(&text).expect("parses");
        assert_eq!(back, file);
        let empty: PeripheralsFile = serde_json::from_str(r#"{"schema_version":1}"#)
            .expect("a missing list is empty");
        assert!(empty.peripherals.is_empty());
    }

    #[test]
    fn a_device_failure_is_not_a_refusal() {
        assert!(device_failure(&AdapterError::NotConnected));
        assert!(device_failure(&AdapterError::Io(std::io::Error::other("gone"))));
        assert!(!device_failure(&AdapterError::Protocol("no such setting".into())));
        assert!(!device_failure(&AdapterError::Unsupported("writes are off")));
    }
}
