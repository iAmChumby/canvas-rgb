//! zones.json: LED counts by device fingerprint, re-applied on every OpenRGB connect.

use canvas_proto::config::{ZonesFile, ZONES_SCHEMA};
use canvas_proto::{AdapterError, DeviceAdapter, DeviceId, DeviceInfo};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct ZoneStore {
    pub path: PathBuf,
    pub file: ZonesFile,
}

impl ZoneStore {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let path = path.as_ref().to_path_buf();
        let file = if path.exists() {
            let bytes = std::fs::read(&path)?;
            let mut file: ZonesFile = serde_json::from_slice(&bytes)?;
            if file.schema_version == 0 { file.schema_version = ZONES_SCHEMA; }
            if file.schema_version > ZONES_SCHEMA {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
                    format!("unsupported zones schema version {}", file.schema_version)));
            }
            file
        } else {
            ZonesFile { schema_version: ZONES_SCHEMA, sizes: Default::default() }
        };
        Ok(Self { path, file })
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let temp = parent.join(format!("{}.tmp", self.path.file_name().and_then(|n| n.to_str()).unwrap_or("zones.json")));
        let bytes = serde_json::to_vec_pretty(&self.file)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, &self.path)
    }

    pub fn get(&self, device_id: &DeviceId, zone: u32) -> Option<u32> {
        self.file.sizes.get(&device_id.0)?.get(&zone).copied()
    }

    pub fn set(&mut self, device_id: &DeviceId, zone: u32, leds: u32) {
        self.file.sizes.entry(device_id.0.clone()).or_default().insert(zone, leds);
    }

    pub async fn apply(&self, adapter: &mut dyn DeviceAdapter, devices: &[DeviceInfo])
        -> Vec<(DeviceId, u32, Result<(), AdapterError>)>
    {
        let mut results = Vec::new();
        for device in devices {
            let Some(zones) = self.file.sizes.get(&device.id.0) else { continue; };
            for (zone, leds) in zones {
                let result = adapter.set_zone_size(&device.id, *zone, *leds).await;
                results.push((device.id.clone(), *zone, result));
            }
        }
        results
    }
}
