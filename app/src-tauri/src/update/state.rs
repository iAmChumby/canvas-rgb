//! `%LOCALAPPDATA%\canvas-rgb\tray\update.json`: what the tray remembers about updates between
//! runs, and the schedule rules that read it. A missing or unreadable file is the defaults.

use std::path::Path;

use canvas_proto::tauri::{CheckFailReason, UpdateInfo};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

/// The automatic check runs when this long has passed since the last finished check.
pub const CHECK_EVERY: Duration = Duration::hours(24);
/// Later hides the editor banner for at most this long.
pub const SNOOZE_FOR: Duration = Duration::hours(24);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LastResult {
    UpToDate,
    Available { version: String },
    Failed { reason: CheckFailReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    /// The automatic check. Check now ignores it.
    pub auto_check: bool,
    /// RFC 3339 time of the last check that got an answer (a failed check does not count).
    pub last_check: Option<String>,
    /// RFC 3339 time of the last automatic check that failed. With `last_check` it sets the day:
    /// at most one automatic check a day, whether the last one worked or not.
    pub last_attempt: Option<String>,
    pub last_result: Option<LastResult>,
    pub skipped_version: Option<String>,
    /// RFC 3339 time of Later. The banner stays hidden until the next automatic check or until
    /// `SNOOZE_FOR` has passed, whichever comes first.
    pub snoozed_at: Option<String>,
    /// The release whose installer was started, with its notes. Written before the launch: the tray
    /// dies during the install, so this is how the next start knows what was attempted.
    pub pending: Option<UpdateInfo>,
    /// When the installer for `pending` was started, in milliseconds since the Unix epoch. A result
    /// file older than this is an earlier run's and is ignored, also after a tray restart (#28).
    /// `None` in a state file written before this field existed: the result is then read as it is.
    pub pending_since_ms: Option<u64>,
    /// Set after an update until the person dismisses "Updated to X.Y.Z"; keeps the release notes.
    pub updated_to: Option<UpdateInfo>,
}

impl Default for Stored {
    fn default() -> Self {
        Stored {
            auto_check: true,
            last_check: None,
            last_attempt: None,
            last_result: None,
            skipped_version: None,
            snoozed_at: None,
            pending: None,
            pending_since_ms: None,
            updated_to: None,
        }
    }
}

pub fn load(path: &Path) -> Stored {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            // Keep what was there (it may hold a pending update) instead of overwriting it later.
            let bad = path.with_extension("json.bad");
            let kept = match std::fs::rename(path, &bad) {
                Ok(()) => format!("kept as {}", bad.display()),
                Err(re) => format!("could not be kept ({re})"),
            };
            crate::log::append(&format!("update: {} is unreadable ({e}), {kept}; using defaults", path.display()));
            Stored::default()
        }),
        Err(_) => Stored::default(),
    }
}

/// `Err` names the first existing folder or file from `root` down to `path` that is a symlink or a
/// junction (a reparse point). The tray never deletes, renames, writes through or follows one; it
/// leaves the cleanup to the person. A path that is not under `root` is refused too.
pub fn refuse_reparse(root: &Path, path: &Path) -> Result<(), String> {
    let rest = path
        .strip_prefix(root)
        .map_err(|_| format!("{} is not under {}", path.display(), root.display()))?;
    let mut current = root.to_path_buf();
    let steps = std::iter::once(None).chain(rest.components().map(Some));
    for part in steps {
        if let Some(part) = part {
            current.push(part);
        }
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() || is_reparse(&meta) => {
                return Err(format!("{} is a junction or symlink", current.display()));
            }
            Ok(_) => {}
            Err(_) => return Ok(()), // not there yet: nothing below it exists either
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Writes next to the file, then renames over it, so a crash never leaves half a file.
pub fn save(path: &Path, stored: &Stored) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(stored).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

pub fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

pub fn parse(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).ok()
}

/// Is the automatic check due? Never checked, or 24 h since the last one (a clock that moved
/// back counts as due, so a wrong clock cannot stop the check for good).
pub fn auto_check_due(stored: &Stored, now: OffsetDateTime) -> bool {
    if !stored.auto_check {
        return false;
    }
    // The later of the two marks the day.
    let last = [stored.last_check.as_deref(), stored.last_attempt.as_deref()]
        .into_iter()
        .flatten()
        .map(parse)
        .collect::<Option<Vec<_>>>()
        .and_then(|times| times.into_iter().max());
    match last {
        None => true,
        Some(last) => now < last || now - last >= CHECK_EVERY,
    }
}

/// Is the Later snooze still running?
pub fn snoozed(stored: &Stored, now: OffsetDateTime) -> bool {
    match stored.snoozed_at.as_deref().and_then(parse) {
        None => false,
        Some(at) => now >= at && now - at < SNOOZE_FOR,
    }
}
