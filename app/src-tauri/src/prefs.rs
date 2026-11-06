//! `%LOCALAPPDATA%\canvas-rgb\tray\prefs.json`: what the tray remembers between runs about
//! which editor to open and which update feed to read (#202, #203). The file sits beside
//! `update.json`; the tray is its only writer.
//!
//! A missing file is the defaults. An unreadable one is kept aside (`prefs.json.bad`) and the
//! defaults are used, so a damaged file never stops the tray and never hides the file from the
//! person who has to look at it.

use std::path::Path;

use canvas_proto::tauri::TrayPrefs;

/// Reads the prefs. A missing file is the defaults; an unreadable one is logged and the defaults
/// are used.
pub fn load(path: &Path) -> TrayPrefs {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            // Keep what was there (it may hold the channel the person chose) instead of
            // overwriting it later.
            let bad = path.with_extension("json.bad");
            let kept = match std::fs::rename(path, &bad) {
                Ok(()) => format!("kept as {}", bad.display()),
                Err(re) => format!("could not be kept ({re})"),
            };
            crate::log::append(&format!(
                "prefs: {} is unreadable ({e}), {kept}; using the defaults",
                path.display()
            ));
            TrayPrefs::default()
        }),
        Err(_) => TrayPrefs::default(),
    }
}

/// Writes next to the file, then renames over it, so a crash never leaves half a file.
pub fn save(path: &Path, prefs: &TrayPrefs) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(prefs).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_the_defaults() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-prefs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("prefs.json");
        assert_eq!(load(&path), TrayPrefs::default());
        assert_eq!(
            load(&path),
            TrayPrefs { editor: canvas_proto::tauri::EditorChoice::React, channel: canvas_proto::tauri::UpdateChannel::Stable }
        );
    }

    #[test]
    fn a_damaged_file_is_the_defaults_and_is_kept_aside() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-prefs-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prefs.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), TrayPrefs::default());
        assert_eq!(std::fs::read_to_string(dir.join("prefs.json.bad")).unwrap(), "{ not json");
        assert!(!path.exists(), "the damaged file is not left in place to be overwritten unseen");
    }

    #[test]
    fn a_file_without_a_field_takes_the_default_of_that_field() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-prefs-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prefs.json");
        std::fs::write(&path, r#"{"channel":"experimental"}"#).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.channel, canvas_proto::tauri::UpdateChannel::Experimental);
        assert_eq!(loaded.editor, canvas_proto::tauri::EditorChoice::React);
    }

    #[test]
    fn saving_writes_next_to_the_file_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-prefs-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tray").join("prefs.json");
        let prefs = TrayPrefs {
            editor: canvas_proto::tauri::EditorChoice::React,
            channel: canvas_proto::tauri::UpdateChannel::Experimental,
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        assert!(!path.with_extension("json.tmp").exists(), "no temp file left behind");
    }
}
