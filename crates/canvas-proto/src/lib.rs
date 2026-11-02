//! canvas-proto: every name and shape that crosses an area boundary in canvas-rgb.
//! Areas build against this crate and never edit it; changes go through the spine owner.

pub mod api;
pub mod cli;
pub mod color;
pub mod config;
pub mod consts;
pub mod device;
pub mod display;
pub mod fans;
pub mod layout;
pub mod peripheral;
pub mod pipe;
pub mod props;
pub mod stack;
pub mod tauri;
pub mod traits;

pub use color::Rgb8;
pub use consts::EFFECT_IDS;
pub use device::{AdapterCapabilities, DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
pub use display::{DisplayConfig, DisplayInfo, DisplayMode, DisplaySource, DisplayUnit, DisplayValue};
pub use peripheral::{
    PeripheralAccess, PeripheralDetail, PeripheralEvent, PeripheralInfo, PeripheralKind, PeripheralSchema,
    PeripheralSection, PeripheralsResponse,
};
pub use layout::{Layout, PlacedZone, RenderScale, SamplingMode, CANVAS_HEIGHT, CANVAS_WIDTH};
pub use props::{InputNeeds, PropKind, PropValue, PropertyDef, PropertySchema, Props};
pub use traits::*;
