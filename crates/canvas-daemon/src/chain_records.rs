//! The two records chain setup writes beside `lights.json`, so a crash cannot leave a header
//! raised for good: `chain-restore.json` holds the earlier length of a header an open setup is
//! holding raised, and `chain-intent.json` holds everything a commit needs to be undone.
//!
//! Both are written before the change they describe, and deleted once the change is settled (the
//! length is back, or the commit is done), so a start that finds one is looking at a crash. An
//! unreadable one is logged with its path and left exactly where it is: it is the only copy of
//! what the crash interrupted.

use std::io;
use std::path::Path;

use canvas_proto::config::ZONES_FILE;
use canvas_proto::scene::{CHAIN_INTENT_FILE, CHAIN_RESTORE_FILE, LIGHTS_FILE, SCENES_FILE};
use canvas_proto::DeviceId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::storage;

/// The earlier length of a header an open setup is holding raised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestoreRecord {
    pub device_id: DeviceId,
    pub zone_index: u32,
    /// The header's length before the setup raised it.
    pub leds: u32,
}

/// Everything a commit needs to be undone. `snapshot` is the folder name `storage::snapshot`
/// returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentRecord {
    pub device_id: DeviceId,
    pub zone_index: u32,
    /// The length the header had before the setup, which a rollback puts back.
    pub restore_leds: u32,
    /// The length the commit is about to write.
    pub final_leds: u32,
    /// The snapshot taken before the commit, by its folder name under `backups\`.
    pub snapshot: String,
}

/// Writes `chain-restore.json` atomically: it must be on disk before the length is raised.
pub fn write_restore(home: &Path, r: &RestoreRecord) -> io::Result<()> {
    storage::write_json(&home.join(CHAIN_RESTORE_FILE), r)
}

/// The record of an open setup, if there is one. `None` when the file is absent or could not be
/// read: an unreadable one is logged with its path and left on disk.
pub fn read_restore(home: &Path) -> Option<RestoreRecord> {
    read_record(home, CHAIN_RESTORE_FILE)
}

/// Deletes `chain-restore.json`. A missing file is not an error.
pub fn delete_restore(home: &Path) {
    delete_file(home, CHAIN_RESTORE_FILE);
}

/// Writes `chain-intent.json` atomically: it must be on disk before the final resize.
pub fn write_intent(home: &Path, i: &IntentRecord) -> io::Result<()> {
    storage::write_json(&home.join(CHAIN_INTENT_FILE), i)
}

/// The record of an unfinished commit, if there is one. `None` when the file is absent or could
/// not be read: an unreadable one is logged with its path and left on disk.
pub fn read_intent(home: &Path) -> Option<IntentRecord> {
    read_record(home, CHAIN_INTENT_FILE)
}

/// Deletes `chain-intent.json`. A missing file is not an error.
pub fn delete_intent(home: &Path) {
    delete_file(home, CHAIN_INTENT_FILE);
}

/// The files a chain commit may change, and nothing besides.
const COMMITTED_FILES: &[&str] = &[LIGHTS_FILE, SCENES_FILE, ZONES_FILE];

/// Copies `lights.json`, `scenes.json` and `zones.json` back from the snapshot folder; other
/// files are untouched. `snapshot` is the folder name [`storage::snapshot`] returned, and the
/// copies are read from under `backups\` in `home`, where it put them.
///
/// A file the snapshot does not hold is removed from `home`: the folder is the whole truth of
/// those three files, so a file that appeared after the snapshot was taken (for example a
/// `zones.json` the interrupted commit had just written) goes, and `home` ends as it was then.
pub fn restore_files_from_snapshot(home: &Path, snapshot: &str) -> io::Result<()> {
    let from = paths::backups_dir_in(home).join(snapshot);
    for name in COMMITTED_FILES {
        let source = from.join(name);
        let target = home.join(name);
        if source.exists() {
            let bytes = std::fs::read(&source)?;
            storage::write_atomic(&target, &bytes)?;
        } else if target.exists() {
            std::fs::remove_file(&target)?;
        }
    }
    Ok(())
}

/// The files half of undoing an interrupted commit: puts `lights.json`, `scenes.json` and
/// `zones.json` back as the intent's snapshot holds them, and touches nothing else. The store in
/// memory is the caller's to reload from the same files.
pub fn rollback_files(home: &Path, intent: &IntentRecord) -> io::Result<()> {
    restore_files_from_snapshot(home, &intent.snapshot)
}

// ---- async wrappers (#241) ----
//
// Every function above does synchronous file I/O (reads, a temp file and `sync_all`, renames,
// copies). The chain routes are async, so they call these wrappers, which run the same function on
// a blocking thread: a stalled disk then holds a blocking thread, never a runtime worker.

/// Runs `work` on a blocking thread. `None` when the thread itself died (a panic).
async fn on_a_blocking_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!(error = %e, "a chain record's file work did not finish");
            None
        }
    }
}

fn did_not_finish() -> io::Error {
    io::Error::other("the file work did not finish")
}

/// [`write_restore`] on a blocking thread.
pub async fn write_restore_async(home: &Path, r: &RestoreRecord) -> io::Result<()> {
    let (home, r) = (home.to_path_buf(), r.clone());
    on_a_blocking_thread(move || write_restore(&home, &r)).await.unwrap_or_else(|| Err(did_not_finish()))
}

/// [`read_restore`] on a blocking thread. `None` also when the thread died.
pub async fn read_restore_async(home: &Path) -> Option<RestoreRecord> {
    let home = home.to_path_buf();
    on_a_blocking_thread(move || read_restore(&home)).await.flatten()
}

/// [`delete_restore`] on a blocking thread.
pub async fn delete_restore_async(home: &Path) {
    let home = home.to_path_buf();
    on_a_blocking_thread(move || delete_restore(&home)).await;
}

/// [`write_intent`] on a blocking thread.
pub async fn write_intent_async(home: &Path, i: &IntentRecord) -> io::Result<()> {
    let (home, i) = (home.to_path_buf(), i.clone());
    on_a_blocking_thread(move || write_intent(&home, &i)).await.unwrap_or_else(|| Err(did_not_finish()))
}

/// [`read_intent`] on a blocking thread. `None` also when the thread died.
pub async fn read_intent_async(home: &Path) -> Option<IntentRecord> {
    let home = home.to_path_buf();
    on_a_blocking_thread(move || read_intent(&home)).await.flatten()
}

/// [`delete_intent`] on a blocking thread.
pub async fn delete_intent_async(home: &Path) {
    let home = home.to_path_buf();
    on_a_blocking_thread(move || delete_intent(&home)).await;
}

/// [`rollback_files`] on a blocking thread.
pub async fn rollback_files_async(home: &Path, intent: &IntentRecord) -> io::Result<()> {
    let (home, intent) = (home.to_path_buf(), intent.clone());
    on_a_blocking_thread(move || rollback_files(&home, &intent))
        .await
        .unwrap_or_else(|| Err(did_not_finish()))
}

/// Reads one of the two records. A file that is there but does not parse is left exactly as it
/// is: it is the only copy of what a crash interrupted, and a rewrite would lose it.
fn read_record<T: DeserializeOwned>(home: &Path, name: &str) -> Option<T> {
    let path = home.join(name);
    let bytes = match paths::read_checked(&path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(
                path = %path.display(), error = %e,
                "{name} could not be read; it is left on disk and nothing is treated as held"
            );
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!(
                path = %path.display(), error = %e,
                "{name} did not parse; it is left on disk and nothing is treated as held"
            );
            None
        }
    }
}

/// Deletes one of the two records. A file that is already gone is not an error; one that will
/// not go is logged, because the next start replays it.
fn delete_file(home: &Path, name: &str) {
    let path = home.join(name);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(
            path = %path.display(), error = %e,
            "{name} could not be deleted; the next start replays it"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A temp home of its own for one test.
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-chain-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_restore_record_round_trips_and_deletes() {
        let home = temp_home("restore-round-trip");
        assert!(read_restore(&home).is_none(), "no file reads as no record");
        let record =
            RestoreRecord { device_id: DeviceId("openrgb:mb".into()), zone_index: 3, leds: 48 };
        write_restore(&home, &record).unwrap();
        assert_eq!(read_restore(&home), Some(record));
        delete_restore(&home);
        assert!(!home.join(CHAIN_RESTORE_FILE).exists());
        assert!(read_restore(&home).is_none());
        // Deleting what is not there is not an error either.
        delete_restore(&home);
    }

    #[test]
    fn an_unreadable_restore_record_reads_as_none_and_is_left_on_disk() {
        let home = temp_home("restore-unreadable");
        let path = home.join(CHAIN_RESTORE_FILE);
        let wrecked = b"{ this is not a restore record";
        std::fs::write(&path, wrecked).unwrap();
        assert!(read_restore(&home).is_none());
        assert_eq!(std::fs::read(&path).unwrap(), wrecked, "the file is left exactly as it was");
    }

    #[test]
    fn an_intent_record_round_trips() {
        let home = temp_home("intent-round-trip");
        assert!(read_intent(&home).is_none(), "no file reads as no record");
        let record = IntentRecord {
            device_id: DeviceId("openrgb:mb".into()),
            zone_index: 2,
            restore_leds: 48,
            final_leds: 16,
            snapshot: "20260101-120000-before-chain-commit".into(),
        };
        write_intent(&home, &record).unwrap();
        assert_eq!(read_intent(&home), Some(record));
        delete_intent(&home);
        assert!(!home.join(CHAIN_INTENT_FILE).exists());
        assert!(read_intent(&home).is_none());
        delete_intent(&home);
    }

    #[test]
    fn restore_files_from_snapshot_puts_back_all_three_files() {
        let home = temp_home("restore-files");
        let lights = br#"{"schema_version":1,"lights":[]}"#;
        let scenes = br#"{"schema_version":1,"active":"default","scenes":[]}"#;
        let zones = br#"{"schema_version":1,"sizes":{}}"#;
        std::fs::write(home.join(LIGHTS_FILE), lights).unwrap();
        std::fs::write(home.join(SCENES_FILE), scenes).unwrap();
        std::fs::write(home.join(ZONES_FILE), zones).unwrap();
        std::fs::write(home.join("other.txt"), b"not a file a chain touches").unwrap();

        // The snapshot the commit takes, through the same call the commit makes.
        let dir = storage::snapshot(&home, "before-chain-commit").unwrap();
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();

        // Every file changes after the snapshot.
        std::fs::write(home.join(LIGHTS_FILE), b"changed lights").unwrap();
        std::fs::write(home.join(SCENES_FILE), b"changed scenes").unwrap();
        std::fs::write(home.join(ZONES_FILE), b"changed zones").unwrap();
        std::fs::write(home.join("other.txt"), b"changed too, but no chain owns it").unwrap();

        restore_files_from_snapshot(&home, &name).unwrap();
        assert_eq!(std::fs::read(home.join(LIGHTS_FILE)).unwrap(), lights, "lights.json is back");
        assert_eq!(std::fs::read(home.join(SCENES_FILE)).unwrap(), scenes, "scenes.json is back");
        assert_eq!(std::fs::read(home.join(ZONES_FILE)).unwrap(), zones, "zones.json is back");
        assert_eq!(
            std::fs::read(home.join("other.txt")).unwrap(),
            b"changed too, but no chain owns it",
            "a file outside the three is not touched"
        );
    }

    #[test]
    fn a_file_the_snapshot_lacks_is_removed() {
        let home = temp_home("restore-lacks");
        let lights = br#"{"schema_version":1,"lights":[]}"#;
        std::fs::write(home.join(LIGHTS_FILE), lights).unwrap();
        let dir = storage::snapshot(&home, "before-chain-commit").unwrap();
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();

        // A zones.json the snapshot never held: home must go back to having none.
        std::fs::write(home.join(ZONES_FILE), b"written after the snapshot").unwrap();
        restore_files_from_snapshot(&home, &name).unwrap();
        assert!(!home.join(ZONES_FILE).exists(), "the file is gone again, as it was");
        assert_eq!(std::fs::read(home.join(LIGHTS_FILE)).unwrap(), lights);
    }
}
