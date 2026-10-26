//! zones.json: LED counts by device fingerprint, re-applied on every OpenRGB connect.

use canvas_proto::config::ZonesFile;
use std::path::PathBuf;

pub struct ZoneStore {
    pub path: PathBuf,
    pub file: ZonesFile,
}
