//! `canvas-rgb diag bundle`: a zip of the daemon's logs and config files (G8).
//!
//! The API token is never in the bundle.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The files the bundle contains, relative to the user directory.
const CONFIG_FILES: &[&str] = &[
    "config.toml",
    "layouts.json",
    "zones.json",
    "effects.json",
    "displays.json",
    "fans.last.toml",
];

/// Writes a zip with the logs and the config files.
///
/// Returns the path of the zip.
pub fn write_bundle(out: &Path) -> Result<PathBuf, BundleError> {
    let out = if out.is_dir() {
        out.join(format!("canvas-rgb-diag-{}.zip", crate::storage::timestamp()))
    } else {
        out.to_path_buf()
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(&out)?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options: zip::write::FileOptions<'_, '_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // The logs.
    let log_dir = crate::paths::log_dir();
    add_directory(&mut zip, &log_dir, "logs", options)?;

    // The config files, minus the token.
    let home = crate::paths::home();
    for name in CONFIG_FILES {
        let path = home.join(name);
        if path.is_file() {
            let bytes = std::fs::read(&path)?;
            zip.start_file(format!("config/{name}"), options)?;
            zip.write_all(&bytes)?;
        }
    }

    zip.finish()?;
    Ok(out)
}

/// Adds every file in a directory to the zip.
fn add_directory(
    zip: &mut zip::ZipWriter<std::io::BufWriter<std::fs::File>>,
    dir: &Path,
    prefix: &str,
    options: zip::write::FileOptions<'_, '_, ()>,
) -> Result<(), BundleError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let bytes = std::fs::read(&path)?;
        zip.start_file(format!("{prefix}/{name}"), options)?;
        zip.write_all(&bytes)?;
    }
    Ok(())
}

/// Why a bundle could not be written.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_configs_are_in_the_diagnostic_bundle() {
        assert!(CONFIG_FILES.contains(&"displays.json"));
    }

    #[test]
    fn the_bundle_never_carries_the_token() {
        // The token file is not in the list of files the bundle writes.
        assert!(!CONFIG_FILES.contains(&"api-token"));
    }

    #[test]
    fn a_bundle_is_written() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-bundle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("bundle.zip");
        let path = write_bundle(&out).unwrap();
        assert!(path.exists());
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"PK"), "the bundle is a zip file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
