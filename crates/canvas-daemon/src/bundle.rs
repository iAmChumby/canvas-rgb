//! Thin daemon wrapper over `canvas-diag`'s one bundle writer (R1).
//!
//! The CLI's `diag bundle` command calls [`write_bundle`]; the API route and the tray call
//! `canvas_diag::bundle::Bundle` directly, so all three scrub and alias the same way.

use std::path::{Path, PathBuf};

pub use canvas_diag::bundle::{output_path, Bundle, BundleError, CONFIG_FILES};

/// Writes a bundle to `out` with the scrubber built from this process's environment.
pub fn write_bundle(out: &Path) -> Result<PathBuf, BundleError> {
    let result = Bundle::new().write(out, &canvas_diag::scrub::Scrubber::from_env())?;
    Ok(PathBuf::from(
        result.path.ok_or_else(|| BundleError::Io(std::io::Error::other("no bundle path")))?,
    ))
}
