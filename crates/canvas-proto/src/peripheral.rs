//! Peripherals: devices canvas-rgb configures rather than lights, such as a mouse or a game
//! controller. Their settings reuse the effect property model (`crate::props`), so the editor
//! renders a peripheral's DPI, polling rate or dead zones with the same controls it uses for an
//! effect's properties. The adapter trait is `PeripheralAdapter` in `traits`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::device::DeviceId;
use crate::props::{PropValue, PropertyDef, Props};

/// What a peripheral is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PeripheralKind {
    /// A mouse.
    Mouse,
    /// A game controller.
    GameController,
    /// A keyboard that holds settings but has no LEDs of its own.
    Keyboard,
}

/// One group of a peripheral's settings: a panel in the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PeripheralSection {
    /// Stable id, e.g. `"dpi"`, `"lighting"`.
    pub id: String,
    /// The title the editor shows.
    pub title: String,
    /// The properties of this section, in the order the editor shows them.
    pub properties: Vec<PropertyDef>,
}

/// What a peripheral has: its sections, its properties, and what cannot be written yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PeripheralSchema {
    /// The peripheral this schema describes.
    pub device: DeviceId,
    /// What kind of peripheral it is.
    pub kind: PeripheralKind,
    /// The sections, in the order the editor shows them.
    pub sections: Vec<PeripheralSection>,
    /// The property keys the adapter cannot write yet.
    pub read_only: Vec<String>,
    /// The ids of the settings the device holds that this adapter does not report as values: the
    /// fields the device has that this build cannot name, and the settings the device's firmware
    /// does not carry at all.
    pub unreported: Vec<String>,
}

impl PeripheralSchema {
    /// Every property of every section, in order.
    pub fn properties(&self) -> impl Iterator<Item = &PropertyDef> {
        self.sections.iter().flat_map(|s| s.properties.iter())
    }

    /// The property with this key, when the device has one.
    pub fn property(&self, key: &str) -> Option<&PropertyDef> {
        self.properties().find(|p| p.key == key)
    }

    /// True when the adapter may write this property.
    pub fn is_writable(&self, key: &str) -> bool {
        !self.read_only.iter().any(|k| k == key)
    }

    /// The default value of this property, from its kind.
    pub fn default_value(&self, key: &str) -> Option<PropValue> {
        self.property(key).map(|p| p.kind.default_value())
    }
}

/// Something the peripheral reported about itself outside a settings read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PeripheralEvent {
    /// The battery: a level when the device reports one that can be trusted, and whether it is
    /// charging.
    Battery {
        /// The battery level in percent, when the device reports one this adapter can trust.
        level: Option<u8>,
        /// The device is charging.
        charging: bool,
    },
    /// The device changed something by itself. The message names it, e.g. `"active-dpi-stage=4"`.
    Notice {
        /// What changed.
        message: String,
    },
}

/// The peripheral as the hub sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PeripheralInfo {
    /// Stable device fingerprint.
    pub id: DeviceId,
    /// The product name.
    pub name: String,
    /// The manufacturer.
    pub vendor: String,
    /// What kind of peripheral it is.
    pub kind: PeripheralKind,
    /// Adapter name, e.g. `"attack-shark-x6"`.
    pub adapter: String,
    /// The serial number, empty when the device reports none.
    pub serial: String,
    /// Where it is, e.g. `"hid:1d57:fa60:usage0b"`.
    pub location: String,
    /// True when the adapter has it open.
    pub connected: bool,
}

/// `GET /peripherals`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PeripheralsResponse {
    pub peripherals: Vec<PeripheralInfo>,
}

/// `GET /peripherals/{id}`: what the device is, what it has, and what it holds now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PeripheralDetail {
    pub info: PeripheralInfo,
    pub schema: PeripheralSchema,
    /// The values read from the device, keyed by property key. A key the device did not report
    /// this time is absent; all are absent while reads are off.
    pub values: Props,
    /// What the daemon may do with this device.
    pub access: PeripheralAccess,
    /// Why the values are missing, when they are: reads are off, the device did not answer, the
    /// vendor app holds it. `None` after a good read.
    pub problem: Option<String>,
}

/// What the daemon may do with one peripheral. Both are off until the owner turns them on, so
/// the daemon never opens or writes a device it was not told to (the vendor app may own it).
/// Saved per device id in `%APPDATA%\canvas-rgb\peripherals.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PeripheralAccess {
    /// Open the device and read its settings. A read may send the device a request that changes
    /// no setting.
    pub reads: bool,
    /// Send config writes. Needs `reads`: a write patches what the device last reported.
    pub writes: bool,
}
