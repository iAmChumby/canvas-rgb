//! The run record: what one agent run made, and the copy of a scene taken before the run
//! changes or deletes it (#377).
//!
//! A run is named by `canvas-rgb ... --run <run>`: 1 to 64 of `A-Z`, `a-z`, `0-9`, `.`, `_` and
//! `-`, never `..`, checked before it is ever joined to a path. The record is
//! `<home>/agent-runs/<run>.json`, where `<home>` is [`crate::paths::home`], so
//! `CANVAS_RGB_HOME` moves it. Every scene an `edit` or a `delete` is about to change is
//! written whole, first, to `<home>/agent-runs/<run>/<scene>.rev<rev>.json`: that copy is the
//! undo.
//!
//! Both files are written with [`crate::storage::write_json`] (a temporary file, then a rename),
//! so a crash leaves the file it had and never half of a new one.

use std::path::PathBuf;

use canvas_proto::scene::Scene;
use serde::{Deserialize, Serialize};

/// The folder of the run records, under the daemon's user tree.
pub const RUNS_DIR: &str = "agent-runs";

/// The most characters a run id holds.
pub const RUN_ID_MAX: usize = 64;

/// True when `run` is a run id: 1 to [`RUN_ID_MAX`] of `A-Z a-z 0-9 . _ -`, and no `..`.
pub fn is_run_id(run: &str) -> bool {
    !run.is_empty()
        && run.chars().count() <= RUN_ID_MAX
        && run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !run.contains("..")
}

/// The refusal a bad run id gets: the rule, so the caller can fix it.
pub fn bad_run_id(run: &str) -> String {
    format!(
        "{run:?} is not a run id: use 1 to {RUN_ID_MAX} characters of A-Z, a-z, 0-9, '.', '_' or '-', with no '..'"
    )
}

/// Checks a run id before it is joined to any path.
pub fn check_run_id(run: &str) -> Result<(), String> {
    if is_run_id(run) {
        Ok(())
    } else {
        Err(bad_run_id(run))
    }
}

/// The folder of the run records: `<home>/agent-runs`.
pub fn runs_dir() -> PathBuf {
    crate::paths::home().join(RUNS_DIR)
}

/// One run's record: `<home>/agent-runs/<run>.json`.
pub fn record_path(run: &str) -> PathBuf {
    runs_dir().join(format!("{run}.json"))
}

/// One run's folder of scene copies: `<home>/agent-runs/<run>`.
pub fn run_dir(run: &str) -> PathBuf {
    runs_dir().join(run)
}

/// The copy of one scene taken before a write:
/// `<home>/agent-runs/<run>/<id>.rev<rev>.json`.
pub fn scene_copy_path(run: &str, id: &str, rev: u64) -> PathBuf {
    run_dir(run).join(format!("{id}.rev{rev}.json"))
}

/// One scene the run made, and the `rev` it was at when the run last wrote it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MadeScene {
    pub id: String,
    pub name: String,
    pub rev: u64,
}

/// The record of one run: `agent-runs/<run>.json`. It is written as:
///
/// ```json
/// {
///   "run": "agent-20261115-1530",
///   "prior_active": "gaming",
///   "before": { "active": "gaming", "scenes": [], "group_states": [] },
///   "scenes": [{ "id": "night", "name": "Night", "rev": 1 }]
/// }
/// ```
///
/// `before` is the `scenes list --json` answer the run read at its first write, and `scenes` is
/// what the run made, in the order it made them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// The run's id, as `--run` gave it.
    pub run: String,
    /// The active scene when the run first wrote, so an undo knows what to switch back to.
    pub prior_active: String,
    /// The `scenes list --json` answer at the run's first write.
    pub before: serde_json::Value,
    /// What the run made, and where each scene stood at the run's last write.
    #[serde(default)]
    pub scenes: Vec<MadeScene>,
}

impl RunRecord {
    /// A record for a run that has not written yet, from the scenes answer it read.
    pub fn start(run: &str, before: &serde_json::Value) -> Self {
        Self {
            run: run.to_string(),
            prior_active: active_of(before).to_string(),
            before: before.clone(),
            scenes: Vec::new(),
        }
    }

    /// What the record says about one scene, if the run made it.
    pub fn made(&self, id: &str) -> Option<&MadeScene> {
        self.scenes.iter().find(|scene| scene.id == id)
    }

    /// Notes a scene the run just made or saved: its `id`, its `name` and the `rev` it is at now.
    pub fn note(&mut self, scene: &Scene) {
        let made = MadeScene { id: scene.id.clone(), name: scene.name.clone(), rev: scene.rev };
        match self.scenes.iter().position(|old| old.id == made.id) {
            Some(at) => self.scenes[at] = made,
            None => self.scenes.push(made),
        }
    }

    /// Drops a scene the run just deleted: it is not a scene the run made any more.
    pub fn forget(&mut self, id: &str) {
        self.scenes.retain(|scene| scene.id != id);
    }

    /// Writes the record where [`read`] looks for it.
    pub fn save(&self) -> Result<PathBuf, String> {
        let path = record_path(&self.run);
        crate::storage::write_json(&path, self)
            .map_err(|e| format!("could not write the run record at {}: {e}", path.display()))?;
        Ok(path)
    }
}

/// The active scene id in a scenes answer.
fn active_of(answer: &serde_json::Value) -> &str {
    answer.get("active").and_then(|v| v.as_str()).unwrap_or_default()
}

/// The record of one run, or `Ok(None)` when the run has not written one yet. A record that is
/// there and cannot be read is an error: a create must not write over a record it cannot see.
pub fn read(run: &str) -> Result<Option<RunRecord>, String> {
    check_run_id(run)?;
    read_file(run)
}

/// The record of one run, or the refusal an `edit` or a `delete` gives. A record that is not
/// there and one that cannot be read refuse the same way: either way this run has nothing to
/// show for the scene, so it may not change or remove it.
pub fn load(run: &str) -> Result<RunRecord, String> {
    check_run_id(run)?;
    match read_file(run) {
        Ok(Some(record)) => Ok(record),
        _ => Err(no_record(run)),
    }
}

/// The refusal for a run this CLI has no record of.
pub fn no_record(run: &str) -> String {
    format!("no record of run {run}; nothing changed")
}

/// Reads the record's file, with no check of the run id: `Ok(None)` when it is not there.
fn read_file(run: &str) -> Result<Option<RunRecord>, String> {
    let path = record_path(run);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("the run record at {} cannot be read: {e}", path.display())),
    };
    serde_json::from_str::<RunRecord>(&text)
        .map(Some)
        .map_err(|e| format!("the run record at {} cannot be read: {e}", path.display()))
}

/// Writes the whole scene, as the daemon answered it, to its copy in the run's folder; the
/// path is returned. A scene is written before it is changed or deleted, and nothing is sent to
/// the daemon until this has returned `Ok`.
pub fn save_scene_copy(run: &str, scene: &serde_json::Value) -> Result<PathBuf, String> {
    check_run_id(run)?;
    let id = scene
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "the scene has no id, so it cannot be copied".to_string())?;
    if !is_run_id(id) {
        return Err(format!("{id:?} cannot be a file name, so the scene is not copied"));
    }
    let rev = scene
        .get("rev")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("the scene {id} has no rev, so it cannot be copied"))?;
    let path = scene_copy_path(run, id, rev);
    crate::storage::write_json(&path, scene)
        .map_err(|e| format!("could not save the scene to {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_is_one_to_sixty_four_of_the_safe_characters_and_never_dot_dot() {
        let longest = "a".repeat(RUN_ID_MAX);
        let too_long = "a".repeat(RUN_ID_MAX + 1);
        for good in ["a", "agent-20261115-1530", "Agent_1.2", ".", "_", "-", longest.as_str()] {
            assert!(is_run_id(good), "{good:?} is a run id");
        }
        for bad in ["", "..", "../x", "a/b", r"a\b", "a b", too_long.as_str(), "a..b", "café", "a:b"] {
            assert!(!is_run_id(bad), "{bad:?} is not a run id");
        }
        assert!(check_run_id("../x").unwrap_err().contains("is not a run id"));
        assert!(check_run_id("agent-1").is_ok());
    }
}
