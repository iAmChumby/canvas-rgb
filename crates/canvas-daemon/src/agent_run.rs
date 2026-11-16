//! The run record: what one agent run made, and the copy of a scene taken before the run
//! changes or deletes it (#377).
//!
//! A run is named by `canvas-rgb ... --run <run>`: 1 to 64 of `A-Z`, `a-z`, `0-9`, `.`, `_` and
//! `-`, never `..`, never a leading or trailing `.`, never a Windows device name (`con`, `nul`,
//! `com0`...), checked before it is ever joined to a path. The record is
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

/// True when `run` is a run id: 1 to [`RUN_ID_MAX`] of `A-Z a-z 0-9 . _ -`, no `..`, no leading
/// or trailing `.` (Windows drops a trailing dot, so `x.` and `x` would share a folder), and
/// not a Windows device name.
pub fn is_run_id(run: &str) -> bool {
    !run.is_empty()
        && run.chars().count() <= RUN_ID_MAX
        && run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !run.contains("..")
        && !run.starts_with('.')
        && !run.ends_with('.')
        && !is_device_name(run)
}

/// True when the part of `name` before its first dot is a Windows device name (`con`, `prn`,
/// `aux`, `nul`, `com0`-`com9`, `lpt0`-`lpt9`), in any case: `con.json` is the console too.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        })
}

/// The refusal a bad run id gets: the rule, so the caller can fix it.
pub fn bad_run_id(run: &str) -> String {
    format!(
        "{run:?} is not a run id: use 1 to {RUN_ID_MAX} characters of A-Z, a-z, 0-9, '.', '_' or '-', with no '..', no '.' first or last, and not a Windows device name such as con or nul"
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

/// One user preset the run saved: the effect it belongs to and its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MadePreset {
    /// The effect's id, as the daemon names it.
    pub effect: String,
    /// The preset's name, as the daemon stored it.
    pub name: String,
}

/// One user effect the run imported: its id (`html:` and the file's stem).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MadeEffect {
    pub id: String,
}

/// A write that was about to be sent, or was sent and not yet noted: saved to the record before
/// the request goes out and removed once the record has its own update, so a crash in between
/// leaves a visible trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    /// `create`, `edit` or `delete` for a scene; `preset-save`, `preset-delete`, `import` or
    /// `effect-delete` for the effects and presets.
    pub op: String,
    /// What the write is about: the new scene's name for a `create`, the scene's id for an
    /// `edit` or a `delete`, `<effect>/<name>` for a preset, the file name for an `import`, the
    /// effect's id for an `effect-delete`.
    pub scene: String,
}

/// The record of one run: `agent-runs/<run>.json`. It is written as:
///
/// ```json
/// {
///   "run": "agent-20261115-1530",
///   "prior_active": "gaming",
///   "before": { "active": "gaming", "scenes": [], "group_states": [] },
///   "scenes": [{ "id": "night", "name": "Night", "rev": 1 }],
///   "presets": [{ "effect": "html:ink-drop", "name": "Night ink" }],
///   "effects": [{ "id": "html:night-ink" }],
///   "pending": [{ "op": "create", "scene": "Day" }]
/// }
/// ```
///
/// `pending`, `presets` and `effects` are left out when empty (an older record has none, and
/// still loads). `before` is the `scenes list --json` answer the run read at its first write, and
/// `scenes`, `presets` and `effects` are what the run made, in the order it made them.
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
    /// The user presets the run saved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<MadePreset>,
    /// The user effects the run imported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<MadeEffect>,
    /// Writes sent, or about to be sent, that the record has not caught up with. An entry that
    /// stays is a write whose outcome this CLI never saw.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<Pending>,
}

impl RunRecord {
    /// A record for a run that has not written yet, from the scenes answer it read.
    pub fn start(run: &str, before: &serde_json::Value) -> Self {
        Self {
            run: run.to_string(),
            prior_active: active_of(before).to_string(),
            before: before.clone(),
            scenes: Vec::new(),
            presets: Vec::new(),
            effects: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Notes that a write is about to be sent. [`save`](Self::save) it before sending.
    pub fn begin(&mut self, op: &str, scene: &str) {
        self.pending.push(Pending { op: op.to_string(), scene: scene.to_string() });
    }

    /// Removes the first pending entry for this write: the record has caught up with it.
    pub fn finish(&mut self, op: &str, scene: &str) {
        if let Some(at) = self.pending.iter().position(|p| p.op == op && p.scene == scene) {
            self.pending.remove(at);
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

    /// The preset of `effect` named `name`, if the run saved it. Both match without regard to
    /// case: the daemon keeps one preset per name whatever its case.
    pub fn made_preset(&self, effect: &str, name: &str) -> Option<&MadePreset> {
        self.presets
            .iter()
            .find(|p| p.effect.eq_ignore_ascii_case(effect) && p.name.eq_ignore_ascii_case(name))
    }

    /// Notes a preset the run just saved.
    pub fn note_preset(&mut self, effect: &str, name: &str) {
        if self.made_preset(effect, name).is_none() {
            self.presets.push(MadePreset { effect: effect.to_string(), name: name.to_string() });
        }
    }

    /// Drops a preset the run just deleted.
    pub fn forget_preset(&mut self, effect: &str, name: &str) {
        self.presets
            .retain(|p| !(p.effect.eq_ignore_ascii_case(effect) && p.name.eq_ignore_ascii_case(name)));
    }

    /// Drops every preset of an effect the run just deleted along with its presets.
    pub fn forget_presets_of(&mut self, effect: &str) {
        self.presets.retain(|p| !p.effect.eq_ignore_ascii_case(effect));
    }

    /// The effect with this id (case aside), if the run imported it.
    pub fn made_effect(&self, id: &str) -> Option<&MadeEffect> {
        self.effects.iter().find(|e| e.id.eq_ignore_ascii_case(id))
    }

    /// Notes an effect the run just imported.
    pub fn note_effect(&mut self, id: &str) {
        if self.made_effect(id).is_none() {
            self.effects.push(MadeEffect { id: id.to_string() });
        }
    }

    /// Drops an effect the run just deleted.
    pub fn forget_effect(&mut self, id: &str) {
        self.effects.retain(|e| !e.id.eq_ignore_ascii_case(id));
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

/// A 64-bit FNV-1a hash of some text: a stable name for a copy, so two copies of different
/// content never share a file and a copy is never written over.
fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// `text` as a piece of a file name: at most 40 of `A-Z a-z 0-9 - _`, anything else an `_`.
/// A copy's name starts with its hash, so no piece can make a Windows device name.
fn safe_stem(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' })
        .take(40)
        .collect()
}

/// The copy of one user preset taken before it is deleted:
/// `<home>/agent-runs/<run>/presets/<hash>.<effect>.<name>.json`. The hash covers the effect, the
/// name and the whole preset, so a preset saved again with other props is a new file.
pub fn preset_copy_path(run: &str, effect: &str, name: &str, content: &str) -> PathBuf {
    let hash = fnv1a(&format!("{effect}\0{name}\0{content}"));
    run_dir(run)
        .join("presets")
        .join(format!("{hash:016x}.{}.{}.json", safe_stem(effect), safe_stem(name)))
}

/// The copy of one user effect taken before it is deleted or written over:
/// `<home>/agent-runs/<run>/effects/<hash>.<id>.json`.
pub fn effect_copy_path(run: &str, id: &str, content: &str) -> PathBuf {
    let hash = fnv1a(&format!("{id}\0{content}"));
    run_dir(run).join("effects").join(format!("{hash:016x}.{}.json", safe_stem(id)))
}

/// Writes the whole preset, as the daemon answered it, with the effect it belongs to, to its copy
/// in the run's folder; the path is returned. Nothing is sent to the daemon until this has
/// returned `Ok`.
pub fn save_preset_copy(run: &str, effect: &str, preset: &serde_json::Value) -> Result<PathBuf, String> {
    check_run_id(run)?;
    let name = preset
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "the preset has no name, so it cannot be copied".to_string())?;
    let content = serde_json::to_string(preset).map_err(|e| e.to_string())?;
    let path = preset_copy_path(run, effect, name, &content);
    let doc = serde_json::json!({ "effect": effect, "preset": preset });
    crate::storage::write_json(&path, &doc)
        .map_err(|e| format!("could not save the preset to {}: {e}", path.display()))?;
    Ok(path)
}

/// Writes a user effect's whole content (its id, its file name, its source and, for a delete, its
/// presets) to its copy in the run's folder; the path is returned. Nothing is sent to the daemon
/// until this has returned `Ok`.
pub fn save_effect_copy(run: &str, doc: &serde_json::Value) -> Result<PathBuf, String> {
    check_run_id(run)?;
    let id = doc
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "the effect has no id, so it cannot be copied".to_string())?;
    let content = serde_json::to_string(doc).map_err(|e| e.to_string())?;
    let path = effect_copy_path(run, id, &content);
    crate::storage::write_json(&path, doc)
        .map_err(|e| format!("could not save the effect to {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_is_one_to_sixty_four_of_the_safe_characters_and_never_dot_dot() {
        let longest = "a".repeat(RUN_ID_MAX);
        let too_long = "a".repeat(RUN_ID_MAX + 1);
        for good in ["a", "agent-20261115-1530", "Agent_1.2", "_", "-", "console", "com10", "a.b", longest.as_str()] {
            assert!(is_run_id(good), "{good:?} is a run id");
        }
        for bad in [
            "", "..", "../x", "a/b", r"a\b", "a b", too_long.as_str(), "a..b", "café", "a:b",
            ".", ".x", "x.", "con", "CON", "Nul", "aux", "prn", "com1", "COM9", "lpt1", "LPT9", "con.json",
            "nul.x.y", "com0", "COM0", "lpt0", "Lpt0.txt", "conin$", "conout$", "con$",
        ] {
            assert!(!is_run_id(bad), "{bad:?} is not a run id");
        }
        assert!(check_run_id("../x").unwrap_err().contains("is not a run id"));
        assert!(check_run_id("agent-1").is_ok());
    }

    #[test]
    fn a_record_without_pending_loads_and_saves_without_it() {
        let old = r#"{ "run": "r", "prior_active": "a", "before": {}, "scenes": [] }"#;
        let mut record: RunRecord = serde_json::from_str(old).expect("an old record loads");
        assert!(record.pending.is_empty());
        assert!(!serde_json::to_string(&record).unwrap().contains("pending"));
        record.begin("create", "Night");
        record.begin("delete", "night");
        record.finish("create", "Night");
        assert_eq!(record.pending, vec![Pending { op: "delete".into(), scene: "night".into() }]);
        assert!(serde_json::to_string(&record).unwrap().contains("\"pending\""));
    }

    #[test]
    fn a_record_without_presets_or_effects_loads_and_saves_without_them() {
        let old = r#"{ "run": "r", "prior_active": "a", "before": {}, "scenes": [] }"#;
        let mut record: RunRecord = serde_json::from_str(old).expect("an old record loads");
        assert!(record.presets.is_empty() && record.effects.is_empty(), "the lists default to empty");
        let text = serde_json::to_string(&record).unwrap();
        assert!(!text.contains("presets") && !text.contains("effects"), "empty lists are left out: {text}");

        record.note_preset("html:ink", "Night");
        record.note_preset("HTML:INK", "NIGHT");
        record.note_effect("html:ink");
        record.note_effect("HTML:Ink");
        assert_eq!(record.presets.len(), 1, "one preset, whatever the case: {:?}", record.presets);
        assert_eq!(record.effects.len(), 1, "one effect, whatever the case: {:?}", record.effects);
        assert!(record.made_preset("html:INK", "night").is_some());
        assert!(record.made_preset("html:ink", "Day").is_none());
        assert!(record.made_preset("html:other", "Night").is_none());
        assert!(record.made_effect("HTML:ink").is_some());
        assert!(record.made_effect("html:other").is_none());
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains("\"presets\"") && text.contains("\"effects\""), "{text}");
        let back: RunRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(back, record, "a record with the lists reads back the same");

        record.note_preset("html:ink", "Day");
        record.note_preset("html:other", "Day");
        record.forget_preset("html:ink", "night");
        assert_eq!(record.presets.len(), 2, "only Night of html:ink went: {:?}", record.presets);
        record.forget_presets_of("HTML:ink");
        assert_eq!(record.presets, vec![MadePreset { effect: "html:other".into(), name: "Day".into() }]);
        record.forget_effect("html:INK");
        assert!(record.effects.is_empty());
    }

    #[test]
    fn a_copy_is_named_by_its_content_and_never_by_a_path_a_name_can_steer() {
        let first = preset_copy_path("r", "html:ink", "a/../b", "{\"props\":1}");
        let second = preset_copy_path("r", "html:ink", "a/../b", "{\"props\":2}");
        let other_name = preset_copy_path("r", "html:ink", "a_.._b", "{\"props\":1}");
        assert_ne!(first, second, "other props are another file");
        assert_ne!(first, other_name, "two names that read alike are two files");
        for path in [&first, &second, &other_name] {
            assert_eq!(path.parent().unwrap(), run_dir("r").join("presets"), "{path:?} stays in the run's folder");
            let file = path.file_name().unwrap().to_string_lossy();
            assert!(
                !file.contains('/') && !file.contains('\\') && !file.contains("..") && !file.contains(':'),
                "{file}"
            );
        }
        let a = effect_copy_path("r", "html:con", "x");
        let b = effect_copy_path("r", "html:con", "y");
        assert_ne!(a, b);
        assert_eq!(a.parent().unwrap(), run_dir("r").join("effects"));
        let file = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(file.ends_with(".html_con.json"), "{file}");
    }
}
