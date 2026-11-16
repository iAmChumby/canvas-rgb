//! The `canvas-rgb` CLI.
//!
//! Every command talks to the daemon's API with the token file, prints a readable table
//! by default and JSON with `--json`, and exits non-zero with the API's error message
//! when something fails. A refused connection prints "daemon not running".

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::time::Duration;

use canvas_proto::peripheral::PeripheralAccess;
use canvas_proto::traits::Effect;

use crate::peer;

/// The most a request may take to reach the daemon. The tray's client uses the same 5 s
/// (`app/src-tauri/src/api.rs`): the API is on the loopback, so anything slower is down.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The most one request may take in all, so a daemon that accepts a connection and never
/// answers cannot hang the command (#328: an import sat for twenty minutes). An import is
/// parsed and installed by the daemon, so this is far more than a status read needs.
const REQUEST_LIMIT: Duration = Duration::from_secs(120);

/// How often a request that could not reach the daemon is made, and the pause between the
/// attempts. Nothing was sent by a failed connect, so a repeat cannot act twice; the machine
/// drops loopback connects under load, and the operating system spends about 21 s on one
/// before giving up (#328). Three attempts of [`CONNECT_TIMEOUT`] each, plus the pauses, stay
/// under 20 s in all.
const CONNECT_ATTEMPTS: u32 = 3;
const CONNECT_RETRY_PAUSE: Duration = Duration::from_millis(250);
/// The CLI's commands.
#[derive(Debug, Parser)]
#[command(name = "canvas-rgb", version, about = "canvas-rgb: lighting and fan control")]
pub struct Cli {
    /// Print the answer as JSON.
    #[arg(long, global = true)]
    pub json: bool,
    /// The API's base URL. Defaults to `http://127.0.0.1:9430`.
    #[arg(long, global = true, env = "CANVAS_RGB_API")]
    pub api: Option<String>,
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// The command names are fixed in `canvas_proto::cli`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the daemon's status.
    Status,
    /// List the devices OpenRGB found.
    #[command(subcommand)]
    Devices(DevicesCommand),
    /// The lights: one per device zone, and one per part of a chain.
    #[command(subcommand)]
    Lights(LightsCommand),
    /// The effect library.
    #[command(subcommand)]
    Effects(EffectsCommand),
    /// The scenes.
    #[command(subcommand)]
    Scenes(ScenesCommand),
    /// The lighting output.
    #[command(subcommand)]
    Output(OutputCommand),
    /// The fans.
    #[command(subcommand)]
    Fans(FansCommand),
    /// The peripherals: the mouse and the controller.
    #[command(subcommand)]
    Peripherals(PeripheralsCommand),
    /// Diagnostics.
    #[command(subcommand)]
    Diag(DiagCommand),
    /// The configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Install a signed build through the service, with no permission prompt.
    Update(UpdateArgs),
}

/// `canvas-rgb update --file <installer> --sig <installer.sig>`
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// The installer (`canvas-rgb-<version>-setup.exe`).
    #[arg(long)]
    pub file: String,
    /// The installer's `.sig` file from the release.
    #[arg(long)]
    pub sig: String,
}

impl UpdateArgs {
    /// Builds the request body: the installer as an absolute path and the text of the `.sig`
    /// file. The service checks the signature itself; this only fails on a missing file.
    pub fn request(&self) -> Result<canvas_proto::api::InstallUpdateRequest, String> {
        let installer = std::path::absolute(&self.file)
            .map_err(|e| format!("could not resolve {}: {e}", self.file))?;
        if !installer.is_file() {
            return Err(format!("{} is not a file", installer.display()));
        }
        let signature = std::fs::read_to_string(&self.sig)
            .map_err(|e| format!("could not read the signature file {}: {e}", self.sig))?;
        Ok(canvas_proto::api::InstallUpdateRequest {
            installer: installer.to_string_lossy().into_owned(),
            signature: signature.trim().to_string(),
        })
    }
}

/// `canvas-rgb devices ...`
#[derive(Debug, Subcommand)]
pub enum DevicesCommand {
    /// List the devices with their zones and LED counts.
    List,
}

/// `canvas-rgb lights ...`
#[derive(Debug, Subcommand)]
pub enum LightsCommand {
    /// List every light with its LED count and whether it is connected.
    List,
}

/// `canvas-rgb effects ...`
#[derive(Debug, Subcommand)]
pub enum EffectsCommand {
    /// List the effects: the built-ins and the user HTML effects.
    List,
    /// Activate an effect.
    Activate(ActivateArgs),
    /// Import a SignalRGB-format HTML effect into the effects folder.
    Import(ImportArgs),
    /// Check a SignalRGB-format HTML effect. No daemon needed.
    Check(CheckArgs),
    /// Print an effect's HTML source, or write it to a new file (#377).
    Source(SourceArgs),
    /// Delete an effect this run imported (#377).
    Delete(EffectDeleteArgs),
    /// The presets of an effect (#377).
    #[command(subcommand)]
    Preset(PresetCommand),
}

/// `canvas-rgb effects preset ...` (#377)
#[derive(Debug, Subcommand)]
pub enum PresetCommand {
    /// List an effect's presets: the built-in ones and yours.
    List(PresetListArgs),
    /// Save a new preset of yours: a name and the properties.
    Save(PresetSaveArgs),
    /// Delete a preset this run saved.
    Delete(PresetDeleteArgs),
}

/// `canvas-rgb effects preset list <effect>` (#377)
#[derive(Debug, Args)]
pub struct PresetListArgs {
    /// The effect's id.
    pub effect: String,
}

/// `canvas-rgb effects preset save <effect> <name> [--prop k=v]... [--run <run>]` (#377)
#[derive(Debug, Args)]
pub struct PresetSaveArgs {
    /// The effect's id.
    pub effect: String,
    /// The preset's name: a new one, at most 40 characters.
    pub name: String,
    /// A property of the preset, as `key=value`. Repeatable.
    #[arg(long = "prop", value_name = "KEY=VALUE")]
    pub props: Vec<String>,
    /// The run this save belongs to: its record is `agent-runs/<run>.json`.
    #[arg(long, value_name = "RUN")]
    pub run: Option<String>,
}

/// `canvas-rgb effects preset delete <effect> <name> --run <run> [--dry-run]` (#377)
#[derive(Debug, Args)]
pub struct PresetDeleteArgs {
    /// The effect's id.
    pub effect: String,
    /// The preset's name.
    pub name: String,
    /// The run that saved the preset: its record is `agent-runs/<run>.json`.
    #[arg(long, value_name = "RUN")]
    pub run: String,
    /// Run every check and print what would happen; delete nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `canvas-rgb effects source <effect> [--out <file>]` (#377)
#[derive(Debug, Args)]
pub struct SourceArgs {
    /// The effect's id: an `html:` effect, a stock one included.
    pub effect: String,
    /// Write the source to this new file instead of printing it. An existing file is not
    /// written over.
    #[arg(long, value_name = "FILE")]
    pub out: Option<String>,
}

/// `canvas-rgb effects delete <effect> [--keep-presets] --run <run> [--dry-run]` (#377)
#[derive(Debug, Args)]
pub struct EffectDeleteArgs {
    /// The effect's id.
    pub effect: String,
    /// Keep the effect's presets in the library. Without it they go with the effect.
    #[arg(long)]
    pub keep_presets: bool,
    /// The run that imported the effect: its record is `agent-runs/<run>.json`.
    #[arg(long, value_name = "RUN")]
    pub run: String,
    /// Run every check and print what would happen; delete nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `canvas-rgb effects activate <Name|id> [--<prop> <value>]...`
#[derive(Debug, Args)]
pub struct ActivateArgs {
    /// The effect's name or id. Names match case-insensitively.
    pub effect: String,
    /// The properties, as `--<prop> <value>` pairs, e.g. `--color #ff0000`.
    #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
    pub props: Vec<String>,
    /// The groups of the active scene to play it on, by name. Repeatable. With no group, every
    /// group of the active scene plays it.
    ///
    /// The properties are a trailing list (everything after the effect is one), so a `--group`
    /// after them is taken out of that list by [`split_groups`]: the flag works before and after
    /// the properties.
    #[arg(long = "group", value_name = "NAME")]
    pub groups: Vec<String>,
}

/// The groups named by `--group <name>` in the activate's trailing arguments, and the rest of
/// them.
///
/// The properties are a trailing var arg (everything after the effect), so `--group` can come
/// before or after them: the pairs are taken out of the list, whatever their place.
pub fn split_groups(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut groups = Vec::new();
    let mut props = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == canvas_proto::cli::GROUP_FLAG {
            if let Some(name) = rest.next() {
                groups.push(name.clone());
                continue;
            }
        }
        // `--group=NAME` is the same flag.
        if let Some(name) = arg.strip_prefix(&format!("{}=", canvas_proto::cli::GROUP_FLAG)) {
            groups.push(name.to_string());
            continue;
        }
        props.push(arg.clone());
    }
    (groups, props)
}

/// `canvas-rgb scenes ...`
#[derive(Debug, Subcommand)]
pub enum ScenesCommand {
    /// List the scenes: the active one is marked.
    List,
    /// Set the scene by name. Names match case-insensitively.
    Set {
        /// The scene's name.
        name: String,
    },
    /// Make a scene from a file of groups (#377).
    Create(CreateArgs),
    /// Replace a scene's groups from a file (#377).
    Edit(EditArgs),
    /// Delete a scene this run made (#377).
    Delete(DeleteArgs),
    /// Put the scenes in a new order: name every scene once (#371).
    Order(OrderArgs),
    /// The groups of a scene.
    #[command(subcommand)]
    Group(ScenesGroupCommand),
}

/// `canvas-rgb scenes create <name> [--from <groups.json>] [--run <run>] [--dry-run]` (#377)
#[derive(Debug, Args)]
pub struct CreateArgs {
    /// The new scene's name.
    pub name: String,
    /// A JSON file holding the groups: an array of `NewGroup`.
    #[arg(long, value_name = "FILE")]
    pub from: Option<String>,
    /// The run this create belongs to: its record is `agent-runs/<run>.json`.
    #[arg(long, value_name = "RUN")]
    pub run: Option<String>,
    /// Run every check and print what would happen; save nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `canvas-rgb scenes edit <scene> --from <groups.json> --run <run> [--dry-run]` (#377)
#[derive(Debug, Args)]
pub struct EditArgs {
    /// The scene, by id or name.
    pub scene: String,
    /// A JSON file holding the groups: an array of `NewGroup`.
    #[arg(long, value_name = "FILE")]
    pub from: String,
    /// The run that made the scene: its record is `agent-runs/<run>.json`.
    #[arg(long, value_name = "RUN")]
    pub run: String,
    /// Run every check and print what would happen; save nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `canvas-rgb scenes delete <scene> --run <run> [--force] [--dry-run]` (#377)
#[derive(Debug, Args)]
pub struct DeleteArgs {
    /// The scene, by id or name.
    pub scene: String,
    /// The run that made the scene: its record is `agent-runs/<run>.json`.
    #[arg(long, value_name = "RUN")]
    pub run: String,
    /// Delete it even though it changed since the run made it.
    #[arg(long)]
    pub force: bool,
    /// Run every check and print what would happen; save nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `canvas-rgb scenes order <scene>... [--dry-run]` (#371)
#[derive(Debug, Args)]
pub struct OrderArgs {
    /// Every scene once, in the new order. A scene is named by its id or its name.
    #[arg(required = true, num_args = 1..)]
    pub scenes: Vec<String>,
    /// Check the order and print it; save nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `canvas-rgb scenes group ...`
#[derive(Debug, Subcommand)]
pub enum ScenesGroupCommand {
    /// Move or copy a group to another scene (#368).
    Move(GroupMoveArgs),
}

/// `canvas-rgb scenes group move <group> --from <scene> --to <scene> [--copy] [--dry-run]` (#368)
#[derive(Debug, Args)]
pub struct GroupMoveArgs {
    /// The group, by name or id.
    pub group: String,
    /// The scene the group is in, by id or name.
    #[arg(long)]
    pub from: String,
    /// The scene the group goes to, by id or name.
    #[arg(long)]
    pub to: String,
    /// Copy the group: the source scene keeps it.
    #[arg(long)]
    pub copy: bool,
    /// Run every check and print what would happen; save nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `canvas-rgb effects import <file> [--replace] [--run <run>]`
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The HTML file to import.
    pub file: String,
    /// Overwrite an effect of the same name. With `--run`, a run may replace only an effect that
    /// same run imported: any other is refused, and the effect's whole content is copied into the
    /// run's folder before the import is sent. Without `--run` this is the plain overwrite a
    /// person at the keyboard uses, with no record and no copy.
    #[arg(long)]
    pub replace: bool,
    /// The run this import belongs to: its record is `agent-runs/<run>.json` (#377).
    #[arg(long, value_name = "RUN")]
    pub run: Option<String>,
}

/// `canvas-rgb effects check <file> [--png <path>] [--preview <dir>] [--prop k=v]...`
#[derive(Debug, Args)]
pub struct CheckArgs {
    /// The HTML file to check.
    pub file: String,
    /// Write frame 60 as a PNG.
    #[arg(long)]
    pub png: Option<String>,
    /// Write the library tile as a GIF and a contact sheet of its frames into this folder.
    #[arg(long, value_name = "DIR")]
    pub preview: Option<String>,
    /// A property to draw the 120 frames and the PNG with, as `key=value`. Repeatable. The
    /// library tile is always drawn at the defaults (#377).
    #[arg(long = "prop", value_name = "KEY=VALUE")]
    pub props: Vec<String>,
}

/// `canvas-rgb output ...`
#[derive(Debug, Subcommand)]
pub enum OutputCommand {
    /// Pause the lighting: black once, then the render loop stops.
    Pause,
    /// Resume the lighting.
    Resume,
}

/// `canvas-rgb fans ...`
#[derive(Debug, Subcommand)]
pub enum FansCommand {
    /// Show the fan headers, temperatures and the active profile.
    Status,
    /// Switch to a fan profile.
    Profile {
        /// The profile's name.
        name: String,
    },
}

/// `canvas-rgb peripherals ...`
#[derive(Debug, Subcommand)]
pub enum PeripheralsCommand {
    /// List the peripherals the adapters see.
    List,
    /// Show one peripheral: what it has, what it holds and what the daemon may do with it.
    Show {
        /// The peripheral's id.
        id: String,
    },
    /// Turn reads and config writes on or off for one device.
    Access {
        /// The peripheral's id.
        id: String,
        /// Read the device's settings. Off means the daemon never opens it.
        #[arg(long, value_enum)]
        reads: Option<Switch>,
        /// Write config changes to the device. Needs reads.
        #[arg(long, value_enum)]
        writes: Option<Switch>,
    },
}

/// `on` or `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Switch {
    /// On.
    On,
    /// Off.
    Off,
}

impl Switch {
    /// True for `on`.
    pub fn is_on(self) -> bool {
        matches!(self, Switch::On)
    }
}

/// `canvas-rgb diag ...`
#[derive(Debug, Subcommand)]
pub enum DiagCommand {
    /// Dump the colours as the hardware reports them.
    Readback,
    /// Render frames locally, without the daemon.
    DumpFrames(DumpFramesArgs),
    /// Zip the logs and the config.
    Bundle {
        /// Where to write the zip.
        #[arg(long)]
        out: Option<String>,
    },
    /// Make a temperature source look failed (the safety test).
    FailSensor {
        /// `cpu`, `gpu` or `board:<name>`.
        source: String,
        /// Clear the failure instead of setting it.
        #[arg(long)]
        clear: bool,
    },
}

/// `canvas-rgb diag dump-frames --effect <id> --frames <n> --out <dir> ...`
#[derive(Debug, Args)]
pub struct DumpFramesArgs {
    /// The effect's id.
    #[arg(long)]
    pub effect: String,
    /// How many frames to render.
    #[arg(long, default_value_t = 60)]
    pub frames: u32,
    /// The frame rate to render at.
    #[arg(long, default_value_t = 60)]
    pub fps: u32,
    /// Where to write the frames.
    #[arg(long)]
    pub out: String,
    /// The frame width.
    #[arg(long, default_value_t = 320)]
    pub width: u32,
    /// The frame height.
    #[arg(long, default_value_t = 200)]
    pub height: u32,
    /// The properties, as `--prop k=v` pairs.
    #[arg(long = "prop", value_name = "KEY=VALUE")]
    pub props: Vec<String>,
}

/// `canvas-rgb config ...`
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Export everything as one JSON file.
    Export {
        /// Where to write the file.
        file: String,
    },
    /// Import a file that `config export` wrote.
    Import {
        /// The file to read.
        file: String,
    },
}

/// Parses a `--color #ff0000` style property value against a property kind.
///
/// The kinds are: color, number, bool, choice, hue and angle.
pub fn parse_prop_value(kind: &canvas_proto::props::PropKind, raw: &str) -> Result<canvas_proto::props::PropValue, String> {
    use canvas_proto::props::{PropKind, PropValue};
    match kind {
        PropKind::Color { .. } => raw
            .parse::<canvas_proto::Rgb8>()
            .map(PropValue::Color)
            .map_err(|e| e.to_string()),
        PropKind::Number { min, max, .. } => {
            let value: f64 = raw.parse().map_err(|_| format!("{raw:?} is not a number"))?;
            if value < *min || value > *max {
                return Err(format!("{value} is outside {min}..={max}"));
            }
            Ok(PropValue::Number(value))
        }
        PropKind::Bool { .. } => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(PropValue::Bool(true)),
            "false" | "no" | "off" | "0" => Ok(PropValue::Bool(false)),
            _ => Err(format!("{raw:?} is not a boolean")),
        },
        PropKind::Choice { options, .. } => {
            let match_ = options.iter().find(|o| o.eq_ignore_ascii_case(raw));
            match match_ {
                Some(o) => Ok(PropValue::Choice(o.clone())),
                None => Err(format!("{raw:?} is not one of {options:?}")),
            }
        }
        PropKind::Hue { .. } => {
            let value: f64 = raw.parse().map_err(|_| format!("{raw:?} is not a number"))?;
            if !(0.0..=360.0).contains(&value) {
                return Err(format!("a hue must be 0..=360, got {value}"));
            }
            Ok(PropValue::Hue(value))
        }
        PropKind::Angle { .. } => {
            let value: f64 = raw.parse().map_err(|_| format!("{raw:?} is not a number"))?;
            if !(0.0..=360.0).contains(&value) {
                return Err(format!("an angle must be 0..=360, got {value}"));
            }
            Ok(PropValue::Angle(value))
        }
    }
}

/// Parses a `--prop key=value` pair.
pub fn parse_prop_pair(pair: &str) -> Result<(String, String), String> {
    let (key, value) = pair
        .split_once('=')
        .ok_or_else(|| format!("{pair:?} is not a key=value pair"))?;
    Ok((key.trim().to_string(), value.trim().to_string()))
}

/// Parses the `--prop key=value` arguments of `effects check` and `effects preset save` (#377).
///
/// Each argument is split at its first `=`, so a value may hold more of them. An argument with
/// no `=`, one with no key, and a key given twice are refused with one plain sentence each; a
/// repeated key would otherwise win silently by its place.
pub fn parse_prop_args(args: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(args.len());
    for arg in args {
        let (key, value) = parse_prop_pair(arg)?;
        if key.is_empty() {
            return Err(format!("{arg:?} has no property name before the ="));
        }
        if pairs.iter().any(|(seen, _)| *seen == key) {
            return Err(format!("--prop {key} is given twice; give each property once"));
        }
        pairs.push((key, value));
    }
    Ok(pairs)
}

/// Turns parsed `--prop` pairs into typed properties, each checked against the kind `kind_of`
/// finds for its key. A key the effect does not have, and a value of the wrong kind or outside
/// its range, are refused; a schema `kind_of` cannot read is its own sentence, not "not a
/// property of this effect".
fn typed_props(
    pairs: &[(String, String)],
    kind_of: impl Fn(&str) -> Result<Option<canvas_proto::props::PropKind>, String>,
) -> Result<canvas_proto::props::Props, String> {
    let mut props = canvas_proto::props::Props::new();
    for (key, raw) in pairs {
        let Some(kind) = kind_of(key)? else {
            return Err(format!("{key} is not a property of this effect"));
        };
        let value = parse_prop_value(&kind, raw).map_err(|e| format!("--prop {key}: {e}"))?;
        props.insert(key.clone(), value);
    }
    Ok(props)
}

/// The kind of the property `key` in an effect's schema as the daemon's JSON carries it:
/// `Ok(None)` when the effect has no such key, and an error when the key's kind cannot be read,
/// so a schema this CLI does not understand is never reported as a missing property.
fn json_prop_kind(
    properties: &[serde_json::Value],
    key: &str,
) -> Result<Option<canvas_proto::props::PropKind>, String> {
    let Some(found) = properties
        .iter()
        .find(|property| property.get("key").and_then(|v| v.as_str()) == Some(key))
    else {
        return Ok(None);
    };
    let Some(kind) = found.get("kind") else {
        return Err(format!("the daemon's schema for {key} carries no kind, so --prop {key} cannot be checked"));
    };
    serde_json::from_value(kind.clone()).map(Some).map_err(|e| {
        format!("the daemon's schema for {key} carries a kind this command cannot read: {e}")
    })
}

/// Parses the `--<prop> <value>` pairs the CLI takes after `effects activate`.
pub fn parse_activate_props(args: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some(key) = arg.strip_prefix("--") else {
            return Err(format!("{arg:?} is not a --property"));
        };
        if key.is_empty() {
            return Err("an empty property name".to_string());
        }
        let Some(value) = args.get(index + 1) else {
            return Err(format!("--{key} needs a value"));
        };
        map.insert(key.to_string(), value.clone());
        index += 2;
    }
    Ok(map)
}

/// The canvas the check renders on: the same size the daemon uses.
pub const CHECK_WIDTH: u32 = 320;
pub const CHECK_HEIGHT: u32 = 200;
/// The frames the check renders.
pub const CHECK_FRAMES: u32 = 120;
/// The frame the PNG is taken from.
pub const CHECK_PNG_FRAME: u32 = 60;

/// One property of the checked effect.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedProperty {
    /// The property's key, which is also the global the script reads.
    pub key: String,
    /// The type: color, number, boolean, choice or hue.
    pub kind: String,
    /// The default value, as the file declares it.
    pub default: String,
}

/// What `effects check` found in a file.
#[derive(Debug, Clone)]
pub struct CheckReport {
    pub name: String,
    pub description: String,
    pub properties: Vec<CheckedProperty>,
    pub frames: u32,
    /// The mean time one frame took, in milliseconds.
    pub mean_frame_ms: f64,
    /// The slowest frame, in milliseconds.
    pub max_frame_ms: f64,
    /// The picture is not the same in every frame.
    pub moves: bool,
    /// The picture is one flat colour.
    pub flat: bool,
    /// Where the PNG was written, when one was asked for.
    pub png: Option<String>,
    /// The script errors the runtime reported while the frames were rendered.
    pub errors: Vec<String>,
}

/// Checks a SignalRGB-format HTML effect file, with no daemon involved.
///
/// The file is read and handed to [`check_source`]. The error is the parser's own message, or
/// why the file could not be read.
pub fn check_effect(path: &str, png: Option<&str>) -> Result<CheckReport, String> {
    check_effect_with(path, png, &[])
}

/// [`check_effect`] with `--prop key=value` arguments: the frames and the PNG are drawn with
/// those properties laid over the defaults. The arguments are parsed before the file is read.
/// The file is read through [`read_capped`], at [`canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES`],
/// so a check never loads more than an import would take.
pub fn check_effect_with(path: &str, png: Option<&str>, props: &[String]) -> Result<CheckReport, String> {
    parse_prop_args(props)?;
    let source = check_source_file(path)?;
    check_source_with(&source, png, props)
}

/// An effect file's text, read capped: no more than an import may hold.
fn check_source_file(path: &str) -> Result<String, String> {
    read_capped(path, canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES, "an effect file")
        .map_err(|e| e.to_string())
}

/// Checks a SignalRGB-format HTML effect's source, with no daemon involved.
///
/// The source is parsed with the same runtime the daemon uses, then 120 frames are rendered on
/// the 320x200 canvas. The error is the parser's own message. `effects check` reads a file and
/// calls this; the editor's preview route renders a draft with the same runtime.
pub fn check_source(source: &str, png: Option<&str>) -> Result<CheckReport, String> {
    check_source_with(source, png, &[])
}

/// [`check_source`] with `--prop key=value` arguments laid over the effect's defaults (#377). A
/// key the effect does not have, or a value its kind refuses, is an error and nothing is drawn.
pub fn check_source_with(source: &str, png: Option<&str>, prop_args: &[String]) -> Result<CheckReport, String> {
    let pairs = parse_prop_args(prop_args)?;
    let mut effect = canvas_html::HtmlEffect::from_source("check", source, &canvas_proto::props::Props::new())
        .map_err(|e| crate::user_effects::load_error_sentence(&e.to_string()))?;
    let schema = effect.schema();
    let mut props = schema.defaults();
    props.extend(typed_props(&pairs, |key| {
        Ok(schema.properties.iter().find(|p| p.key == key).map(|p| p.kind.clone()))
    })?);
    effect.init(&props);

    let mut canvas = canvas_proto::Canvas::new(CHECK_WIDTH, CHECK_HEIGHT);
    let inputs = canvas_proto::Inputs::default();
    let step = 1.0 / 60.0;

    let mut times: Vec<f64> = Vec::with_capacity(CHECK_FRAMES as usize);
    let mut hashes: Vec<u64> = Vec::with_capacity(CHECK_FRAMES as usize);
    let mut shot: Option<Vec<u8>> = None;

    for frame in 0..CHECK_FRAMES {
        let started = std::time::Instant::now();
        effect.update(step, &mut canvas, &inputs);
        times.push(started.elapsed().as_secs_f64() * 1000.0);
        hashes.push(frame_hash(&canvas));
        if frame == CHECK_PNG_FRAME {
            shot = Some(rgb_bytes(&canvas));
        }
        // A script that failed is reported once and more frames tell us nothing. A
        // script that hangs would cost the whole frame budget on every frame, so the
        // check stops there too.
        if !effect.errors().is_empty() {
            break;
        }
    }

    // The picture moves when two frames differ.
    let moves = hashes.iter().any(|h| *h != hashes[0]);
    // A flat picture is one colour across the whole canvas.
    let flat = distinct_colours(&canvas) <= 1;

    let mean_frame_ms = if times.is_empty() {
        0.0
    } else {
        times.iter().sum::<f64>() / times.len() as f64
    };
    let max_frame_ms = times.iter().copied().fold(0.0_f64, f64::max);

    let png_path = match (png, shot) {
        (Some(out), Some(bytes)) => {
            write_png(out, CHECK_WIDTH, CHECK_HEIGHT, &bytes)?;
            Some(out.to_string())
        }
        (Some(out), None) => {
            return Err(format!("the file has no frame {CHECK_PNG_FRAME} to write to {out}"))
        }
        (None, _) => None,
    };

    Ok(CheckReport {
        name: schema.name,
        description: schema.description,
        properties: schema
            .properties
            .iter()
            .map(|p| CheckedProperty {
                key: p.key.clone(),
                kind: prop_kind_name(&p.kind).to_string(),
                default: prop_default(&p.kind),
            })
            .collect(),
        frames: times.len() as u32,
        mean_frame_ms,
        max_frame_ms,
        moves,
        flat,
        png: png_path,
        errors: effect.errors().to_vec(),
    })
}

/// The name of a property type, as the report shows it.
fn prop_kind_name(kind: &canvas_proto::props::PropKind) -> &'static str {
    use canvas_proto::props::PropKind;
    match kind {
        PropKind::Color { .. } => "color",
        PropKind::Number { .. } => "number",
        PropKind::Bool { .. } => "boolean",
        PropKind::Choice { .. } => "choice",
        PropKind::Hue { .. } => "hue",
        PropKind::Angle { .. } => "angle",
    }
}

/// The default value of a property, as text.
fn prop_default(kind: &canvas_proto::props::PropKind) -> String {
    use canvas_proto::props::PropKind;
    match kind {
        PropKind::Color { default } => default.to_hex(),
        PropKind::Number { default, .. } => format!("{default}"),
        PropKind::Bool { default } => format!("{default}"),
        PropKind::Choice { default, .. } => default.clone(),
        PropKind::Hue { default } => format!("{default}"),
        PropKind::Angle { default } => format!("{default}"),
    }
}

/// A hash of the canvas' pixels, so two frames can be compared.
fn frame_hash(canvas: &canvas_proto::Canvas) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for pixel in &canvas.pixels {
        for value in pixel {
            let byte = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// How many different colours the canvas holds.
fn distinct_colours(canvas: &canvas_proto::Canvas) -> usize {
    let mut seen = std::collections::HashSet::new();
    for pixel in &canvas.pixels {
        let rgb = pixel.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        seen.insert(rgb);
        if seen.len() > 1 {
            return seen.len();
        }
    }
    seen.len()
}

/// The canvas' pixels as RGB bytes.
fn rgb_bytes(canvas: &canvas_proto::Canvas) -> Vec<u8> {
    let mut out = Vec::with_capacity(canvas.pixels.len() * 3);
    for pixel in &canvas.pixels {
        for value in pixel {
            out.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

/// Writes the RGB bytes as a PNG.
fn write_png(path: &str, width: u32, height: u32, rgb: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("could not create {path}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("could not write the PNG header of {path}: {e}"))?;
    writer
        .write_image_data(rgb)
        .map_err(|e| format!("could not write the PNG {path}: {e}"))
}

/// What `effects check` found in the effect's library tile (see [`crate::thumbnail`]).
#[derive(Debug, Clone)]
pub struct TileCheck {
    /// The tile's measures, the same ones the quality bar reads.
    pub score: crate::thumbnail::TileScore,
    /// What the tile fails of the bar. Empty when it passes.
    pub problems: Vec<String>,
    /// The recipe the file carries (`<meta thumbnail ...>`), when it has one.
    pub recipe: Option<canvas_proto::props::ThumbnailRecipe>,
    /// The GIF of the tile, when a preview folder was given.
    pub gif: Option<String>,
    /// The contact sheet of the tile's frames, when a preview folder was given.
    pub sheet: Option<String>,
}

/// The factor the preview GIF is scaled up by (64x40 becomes 320x200).
const PREVIEW_GIF_SCALE: usize = 5;
/// The factor the contact sheet's frames are scaled up by.
const PREVIEW_SHEET_SCALE: usize = 2;
/// The contact sheet's columns, and the gap between frames in pixels.
const PREVIEW_SHEET_COLUMNS: usize = 6;
const PREVIEW_SHEET_GAP: usize = 4;

/// Renders the effect's library tile exactly as the daemon does, scores it against the
/// quality bar, and (given a folder) writes the tile as a GIF and a contact sheet of its
/// frames, so the picture can be looked at and not only measured.
pub fn check_tile(path: &str, preview_dir: Option<&str>) -> Result<TileCheck, String> {
    let source = check_source_file(path)?;
    // The tile is rendered with the fixed tile seed, so the check scores the same picture
    // every time it is run.
    let mut effect = canvas_html::HtmlEffect::from_source_seeded(
        "check",
        &source,
        &canvas_proto::props::Props::new(),
        crate::thumbnail::TILE_SEED,
    )
    .map_err(|e| e.to_string())?
    // A tile frame is never dropped for being slow, so the check scores the same picture on
    // any machine.
    .tile_mode();
    let schema = effect.schema();
    let body = crate::thumbnail::render(&mut effect, &schema.defaults())?;
    let score = crate::thumbnail::score(&body).ok_or("the tile render was malformed")?;
    let mut problems: Vec<String> = effect.warnings().to_vec();
    // A frame that failed is a failure of the tile, not a quietly different picture.
    problems.extend(effect.errors().iter().map(|e| format!("the script failed while the tile was drawn: {e}")));
    problems.extend(crate::thumbnail::problems(&score, schema.thumbnail.as_ref()));

    let (mut gif, mut sheet) = (None, None);
    if let Some(dir) = preview_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {dir}: {e}"))?;
        let stem = std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "effect".to_string());
        let gif_path = std::path::Path::new(dir).join(format!("{stem}.gif"));
        let sheet_path = std::path::Path::new(dir).join(format!("{stem}-sheet.png"));
        write_tile_gif(&gif_path, &body)?;
        write_tile_sheet(&sheet_path, &body)?;
        gif = Some(gif_path.to_string_lossy().into_owned());
        sheet = Some(sheet_path.to_string_lossy().into_owned());
    }
    Ok(TileCheck { score, problems, recipe: schema.thumbnail, gif, sheet })
}

/// The width, height and RGB frames of a thumbnail body.
fn tile_frames(body: &[u8]) -> (usize, usize, Vec<&[u8]>) {
    let w = u16::from_le_bytes([body[0], body[1]]) as usize;
    let h = u16::from_le_bytes([body[2], body[3]]) as usize;
    (w, h, body[6..].chunks_exact(w * h * 3).collect())
}

/// Scales an RGB frame up by whole pixels.
fn scale_up(frame: &[u8], w: usize, h: usize, factor: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * factor * factor * 3);
    for y in 0..h * factor {
        for x in 0..w * factor {
            let i = ((y / factor) * w + x / factor) * 3;
            out.extend_from_slice(&frame[i..i + 3]);
        }
    }
    out
}

/// Writes the tile as a looping GIF at the library's 12 frames a second.
pub fn write_tile_gif(path: &std::path::Path, body: &[u8]) -> Result<(), String> {
    let (w, h, frames) = tile_frames(body);
    let (gw, gh) = (w * PREVIEW_GIF_SCALE, h * PREVIEW_GIF_SCALE);
    let file = std::fs::File::create(path).map_err(|e| format!("could not create {}: {e}", path.display()))?;
    let mut encoder = gif::Encoder::new(std::io::BufWriter::new(file), gw as u16, gh as u16, &[])
        .map_err(|e| format!("could not write the GIF {}: {e}", path.display()))?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| format!("could not write the GIF {}: {e}", path.display()))?;
    for frame in frames {
        let rgb = scale_up(frame, w, h, PREVIEW_GIF_SCALE);
        let mut f = gif::Frame::from_rgb_speed(gw as u16, gh as u16, &rgb, 10);
        // Hundredths of a second: 12 frames a second is 8.
        f.delay = 8;
        encoder
            .write_frame(&f)
            .map_err(|e| format!("could not write the GIF {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Writes every frame of the tile in a grid, in order, left to right and top to bottom.
pub fn write_tile_sheet(path: &std::path::Path, body: &[u8]) -> Result<(), String> {
    let (w, h, frames) = tile_frames(body);
    let (fw, fh) = (w * PREVIEW_SHEET_SCALE, h * PREVIEW_SHEET_SCALE);
    let columns = PREVIEW_SHEET_COLUMNS.min(frames.len().max(1));
    let rows = frames.len().div_ceil(columns);
    let gap = PREVIEW_SHEET_GAP;
    let (sw, sh) = (columns * fw + (columns + 1) * gap, rows * fh + (rows + 1) * gap);
    // A mid grey between the frames, so a black frame still shows its edge.
    let mut sheet = vec![0x40u8; sw * sh * 3];
    for (n, frame) in frames.iter().enumerate() {
        let big = scale_up(frame, w, h, PREVIEW_SHEET_SCALE);
        let (ox, oy) = (gap + (n % columns) * (fw + gap), gap + (n / columns) * (fh + gap));
        for y in 0..fh {
            let from = y * fw * 3;
            let to = ((oy + y) * sw + ox) * 3;
            sheet[to..to + fw * 3].copy_from_slice(&big[from..from + fw * 3]);
        }
    }
    write_png(&path.to_string_lossy(), sw as u32, sh as u32, &sheet)
}

/// A CLI error.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Nothing answered: the daemon is not running, or the port refused the connection.
    /// Nothing was sent, so the token is safe.
    #[error("the daemon is not running (nothing answers on {0})")]
    NotRunning(String),
    /// The daemon is not running, or the port is held by another program: the API token
    /// is not sent to it.
    #[error("{0}")]
    NotTrusted(String),
    /// The connection was made, and the daemon did not answer inside its request limit. The
    /// request may have reached it: whether it acted is not known (#328).
    #[error("{0}")]
    NoAnswer(String),
    #[error("{}", api_error_text(*.status, .message, .body))]
    Api {
        status: u16,
        message: String,
        /// The error body the API sent, whole: a `409` carries the current scene or order.
        body: Option<serde_json::Value>,
    },
    #[error("{0}")]
    Other(String),
}

/// What an API error says: the status and the message, and for a `409` the current `rev` of
/// the scene or the current order the body carries, so the owner sees what changed.
fn api_error_text(status: u16, message: &str, body: &Option<serde_json::Value>) -> String {
    let mut text = format!("the API returned {status}: {message}");
    let Some(body) = body else { return text };
    if let Some(scene) = body.get("scene") {
        let id = scene.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        if let Some(rev) = scene.get("rev").and_then(|v| v.as_u64()) {
            text.push_str(&format!(" (scene {id} is at rev {rev} now)"));
        }
    }
    if let Some(order) = body.get("order").and_then(|v| v.as_array()) {
        let ids: Vec<&str> = order.iter().filter_map(|v| v.as_str()).collect();
        text.push_str(&format!(" (the order is now: {})", ids.join(", ")));
    }
    text
}

impl CliError {
    /// The error body the API sent, for `--json`: it goes to stdout while stderr gets the message.
    pub fn json_body(&self) -> Option<&serde_json::Value> {
        match self {
            CliError::Api { body, .. } => body.as_ref(),
            _ => None,
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        CliError::Other(message)
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        CliError::Other(error.to_string())
    }
}

impl CliError {
    /// The exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::NotRunning(_) | CliError::NotTrusted(_) | CliError::NoAnswer(_) => 2,
            CliError::Api { .. } => 1,
            CliError::Other(_) => 1,
        }
    }
}

/// What `--json` prints on stdout when `scenes create`, `scenes edit`, `scenes delete` or one of
/// the effects commands of #377 (`effects import`, `check`, `source`, `delete` and `preset
/// list|save|delete`) fails: one document, `{"saved": false, "run": <run or null>, "error":
/// "<text>"}`, for every kind of failure. Any other command answers `None` and keeps its own
/// output.
pub fn failure_document(command: &Command, error: &CliError) -> Option<serde_json::Value> {
    let run = match command {
        Command::Scenes(ScenesCommand::Create(args)) => args.run.as_deref(),
        Command::Scenes(ScenesCommand::Edit(args)) => Some(args.run.as_str()),
        Command::Scenes(ScenesCommand::Delete(args)) => Some(args.run.as_str()),
        Command::Effects(EffectsCommand::Import(args)) => args.run.as_deref(),
        Command::Effects(EffectsCommand::Delete(args)) => Some(args.run.as_str()),
        Command::Effects(EffectsCommand::Check(_) | EffectsCommand::Source(_)) => None,
        Command::Effects(EffectsCommand::Preset(PresetCommand::List(_))) => None,
        Command::Effects(EffectsCommand::Preset(PresetCommand::Save(args))) => args.run.as_deref(),
        Command::Effects(EffectsCommand::Preset(PresetCommand::Delete(args))) => Some(args.run.as_str()),
        _ => return None,
    };
    Some(serde_json::json!({ "saved": false, "run": run, "error": error.to_string() }))
}

/// What a single request ended with, before it is turned into a message.
enum Attempt {
    /// The request never got an answer: nothing connected, or no reply inside the limit. The
    /// error is kept whole so a connect failure can be told from the rest and repeated.
    NoReply(ureq::Error),
    /// The daemon answered, and the answer is an error for the caller.
    Answer(CliError),
}

/// What a failed request means, and what the command says about it (#328).
///
/// Three cases, because they are three different things to a person:
///
/// - nothing answered (a refused connection, or no connection inside [`CONNECT_TIMEOUT`]):
///   the daemon is not running, and nothing was sent;
/// - the connection was made and the daemon did not answer inside `limit`: the daemon is
///   there but stuck, and whether it acted is not known;
/// - anything else keeps the transport's own text.
fn transport_error(error: ureq::Error, base: &str, limit: Option<Duration>) -> CliError {
    if is_connect_failure(&error) {
        return CliError::NotRunning(base.to_string());
    }
    match error {
        // A phase timeout is the limit firing: the daemon took the connection and did not
        // answer inside it.
        ureq::Error::Timeout(_) => CliError::NoAnswer(format!(
            "the daemon did not answer within {} ({base})",
            limit_text(limit)
        )),
        other => CliError::Other(other.to_string()),
    }
}

/// True when the request never reached anything: no connection was made, so nothing was
/// sent. Only this failure is safe to try again, and only this one is "the daemon is not
/// running". A reset or a timeout while a request is in flight is a different thing: the
/// daemon may have acted, so it is not repeated.
fn is_connect_failure(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::Timeout(ureq::Timeout::Connect) => true,
        ureq::Error::ConnectionFailed => true,
        ureq::Error::Io(cause) => matches!(
            cause.kind(),
            std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::AddrNotAvailable
                | std::io::ErrorKind::HostUnreachable
                | std::io::ErrorKind::NetworkUnreachable
        ),
        _ => false,
    }
}

/// A request limit as a message: "120 s", or "300 ms" for a test's short one.
fn limit_text(limit: Option<Duration>) -> String {
    match limit {
        Some(limit) if limit.as_secs() > 0 => format!("{} s", limit.as_secs()),
        Some(limit) => format!("{} ms", limit.as_millis()),
        None => "its limit".to_string(),
    }
}

/// The HTTP agent every command uses. 4xx and 5xx come back as responses, so the API's own
/// message is shown; and every request is bounded, so one that cannot reach the daemon or is
/// never answered fails instead of waiting on the machine (#328).
fn bounded_agent() -> ureq::Agent {
    bounded_agent_with(CONNECT_TIMEOUT, REQUEST_LIMIT)
}

/// As [`bounded_agent`], with the limits a test needs to reach them in a second.
fn bounded_agent_with(connect: Duration, global: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(connect))
        .timeout_global(Some(global))
        .build()
        .into()
}

/// The API client the commands use.
pub struct Client {
    base: String,
    token: String,
    /// The agent answers 4xx and 5xx with the response, so the API's own message can be
    /// shown instead of a bare status code.
    agent: ureq::Agent,
}

impl Client {
    /// Builds a client from the token file and the base URL.
    ///
    /// The API token is a bearer secret, so the client is only built when the process
    /// holding the port really is the daemon, running as this user. Otherwise the token
    /// is never sent.
    pub fn new(base: Option<&str>) -> Result<Self, CliError> {
        let base = base
            .map(str::to_string)
            .unwrap_or_else(canvas_proto::consts::api_base_url);
        peer::check_base(&base).map_err(|refusal| CliError::NotTrusted(refusal.message()))?;
        let token_path = crate::paths::token_file();
        let token = match std::fs::read_to_string(&token_path) {
            Ok(text) => text.trim().to_string(),
            Err(e) => {
                return Err(CliError::Other(format!(
                    "could not read the API token at {}: {e}",
                    token_path.display()
                )))
            }
        };
        if token.is_empty() {
            return Err(CliError::Other(format!(
                "the API token at {} is empty",
                token_path.display()
            )));
        }
        let agent = bounded_agent();
        Ok(Self { base, token, agent })
    }

    /// Sends a request and returns the body.
    ///
    /// A connect failure is tried again, up to [`CONNECT_ATTEMPTS`] times: nothing was sent, so
    /// a repeat cannot act twice, and the machine drops loopback connects under load (#328).
    /// Every other failure is reported at once.
    pub fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, CliError> {
        let limit = self.agent.config().timeouts().global;
        let mut attempt = 1;
        loop {
            match self.attempt(method, path, body) {
                Ok(answer) => return Ok(answer),
                Err(Attempt::Answer(error)) => return Err(error),
                Err(Attempt::NoReply(error)) => {
                    if is_connect_failure(&error) && attempt < CONNECT_ATTEMPTS {
                        eprintln!(
                            "canvas-rgb: could not reach the daemon at {}; trying again (attempt {} of {})",
                            self.base, attempt, CONNECT_ATTEMPTS
                        );
                        attempt += 1;
                        std::thread::sleep(CONNECT_RETRY_PAUSE);
                        continue;
                    }
                    return Err(transport_error(error, &self.base, limit));
                }
            }
        }
    }

    /// One request, with no repeat: send it, read the whole body, and turn the answer into a
    /// value. A transport failure comes back whole, so [`Client::request`] can tell a connect
    /// failure (safe to repeat: nothing was sent) from a reply that never came.
    fn attempt(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, Attempt> {
        let url = format!("{}{}", self.base.trim_end_matches('/'), path);
        let auth = format!("Bearer {}", self.token);

        let response = match (method, body) {
            ("GET", None) => self.agent.get(&url).header("Authorization", &auth).call(),
            ("GET", Some(text)) => self
                .agent
                .get(&url)
                .header("Authorization", &auth)
                .force_send_body()
                .send(text.as_bytes()),
            ("POST", None) => {
                self.agent.post(&url).header("Authorization", &auth).send(&[] as &[u8])
            }
            ("POST", Some(text)) => self
                .agent
                .post(&url)
                .header("Authorization", &auth)
                .content_type("application/json")
                .send(text.as_bytes()),
            ("PUT", None) => self.agent.put(&url).header("Authorization", &auth).send(&[] as &[u8]),
            ("PUT", Some(text)) => self
                .agent
                .put(&url)
                .header("Authorization", &auth)
                .content_type("application/json")
                .send(text.as_bytes()),
            ("PATCH", None) => {
                self.agent.patch(&url).header("Authorization", &auth).send(&[] as &[u8])
            }
            ("PATCH", Some(text)) => self
                .agent
                .patch(&url)
                .header("Authorization", &auth)
                .content_type("application/json")
                .send(text.as_bytes()),
            ("DELETE", None) => self.agent.delete(&url).header("Authorization", &auth).call(),
            (other, _) => {
                return Err(Attempt::Answer(CliError::Other(format!(
                    "unsupported HTTP method {other}"
                ))));
            }
        };

        let mut response = response.map_err(Attempt::NoReply)?;

        let status = response.status().as_u16();
        // A body that cannot be read is a failed request, never an empty answer: half an answer
        // is not an answer (#328).
        let text = response.body_mut().read_to_string().map_err(Attempt::NoReply)?;
        if status >= 400 {
            // The API answers with `{ "error": ..., "message": ... }`; the body is kept whole.
            let body = serde_json::from_str::<serde_json::Value>(&text).ok();
            let message = body
                .as_ref()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
                .unwrap_or(text);
            return Err(Attempt::Answer(CliError::Api { status, message, body }));
        }
        if text.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&text).or(Ok(serde_json::Value::String(text)))
    }
}

/// The API the commands talk to.
///
/// [`Client`] is the real one. The trait exists so the commands can be tested without a
/// daemon: the tests answer with a fake and check what was asked for.
pub trait Api {
    /// Sends a request and returns the body.
    fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, CliError>;
}

impl Api for Client {
    fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, CliError> {
        self.request(method, path, body)
    }
}

/// What a command prints.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// The JSON answer, printed with `--json`.
    Json(serde_json::Value),
    /// The text the command prints.
    Text(String),
}

/// `canvas-rgb scenes list`: the scenes, the active one marked.
pub fn scenes_list(api: &dyn Api, json: bool) -> Result<Output, CliError> {
    let answer = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    Ok(if json {
        Output::Json(answer.clone())
    } else {
        Output::Text(scenes_list_text(&answer))
    })
}

/// `canvas-rgb scenes set <name>`: sets the scene by name, case-insensitively, or by its id.
///
/// A name that matches nothing prints the list and fails: the owner sees the names the daemon
/// knows and picks one. A name that matches several scenes is refused too, and the matches are
/// listed: scene names are not unique, and picking one of them is the owner's call. An id
/// names one scene, so it is accepted as well.
pub fn scenes_set(api: &dyn Api, name: &str, json: bool) -> Result<Output, CliError> {
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let found = resolve_scene(&list, name, json)?;
    set_scene(api, &found, json)
}

/// The scene `name` means in a scenes answer: an exact id first, then a name without regard to
/// case. A name that matches nothing fails and prints the list; a name that matches several
/// scenes is refused and the matches are listed (scene names are not unique, and picking one
/// of them is the owner's call).
fn resolve_scene(list: &serde_json::Value, name: &str, json: bool) -> Result<serde_json::Value, CliError> {
    let scenes = list.get("scenes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    // An exact id names one scene.
    if let Some(found) = scenes.iter().find(|scene| scene.get("id").and_then(|v| v.as_str()) == Some(name)) {
        return Ok(found.clone());
    }
    let matches: Vec<&serde_json::Value> = scenes
        .iter()
        .filter(|scene| {
            scene
                .get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|scene_name| scene_name.eq_ignore_ascii_case(name))
        })
        .collect();
    if matches.len() > 1 {
        let names: Vec<String> = matches
            .iter()
            .map(|scene| {
                let id = scene.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let scene_name = scene.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                format!("{id} ({scene_name})")
            })
            .collect();
        return Err(CliError::Other(format!(
            "{name} names {} scenes: {}; name one of them by its id",
            matches.len(),
            names.join(", ")
        )));
    }
    let Some(found) = matches.first() else {
        let list_text = scenes_list_text(list);
        if json {
            return Err(CliError::Other(format!(
                "no scene is named {name}; the scenes are: {}",
                list_text.trim()
            )));
        }
        return Err(CliError::Other(format!("no scene is named {name}\n{list_text}")));
    };
    Ok((*found).clone())
}

fn str_of<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value.get(key).and_then(|v| v.as_str()).unwrap_or("?")
}

/// `canvas-rgb scenes order <scene>... [--dry-run]` (#371): names every scene once, in the new
/// order. It reads the scenes, checks the names, and writes the order with the order it read as
/// `expected_order`, so another window's change is a `409` and not a lost update.
pub fn scenes_order(api: &dyn Api, names: &[String], dry_run: bool, json: bool) -> Result<Output, CliError> {
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let scenes = list.get("scenes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let now: Vec<String> = scenes.iter().map(|s| str_of(s, "id").to_string()).collect();
    let mut order: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let found = resolve_scene(&list, name, json)?;
        order.push(str_of(&found, "id").to_string());
    }
    let mut sorted = order.clone();
    sorted.sort();
    let mut sorted_now = now.clone();
    sorted_now.sort();
    if sorted != sorted_now {
        let list_text = scenes_list_text(&list);
        return Err(CliError::Other(if json {
            format!("name every scene once; the scenes are: {}", list_text.trim())
        } else {
            format!("name every scene once, in the order you want\n{list_text}")
        }));
    }
    let title = |id: &String| {
        scenes.iter().find(|s| str_of(s, "id") == id).map(|s| str_of(s, "name").to_string()).unwrap_or_default()
    };
    let titles = order.iter().map(title).collect::<Vec<_>>().join(", ");
    if dry_run {
        return Ok(if json {
            Output::Json(serde_json::json!({ "saved": false, "order": order }))
        } else {
            Output::Text(format!("would order: {titles}\n"))
        });
    }
    let body = serde_json::to_string(&canvas_proto::api::ScenesWrite {
        order: Some(order),
        expected_order: Some(now),
        ..Default::default()
    })
    .map_err(|e| CliError::Other(e.to_string()))?;
    let answer = api.call("PUT", canvas_proto::api::routes::SCENES, Some(&body))?;
    Ok(if json { Output::Json(answer) } else { Output::Text(format!("the scenes are in this order: {titles}\n")) })
}

/// `canvas-rgb scenes group move <group> --from <scene> --to <scene> [--copy] [--dry-run]`
/// (#368). The scenes resolve like `scenes set`, the group by id and then by name. It sends the
/// revs it read, so a scene that changed since is a `409` and nothing moves.
pub fn scenes_group_move(api: &dyn Api, args: &GroupMoveArgs, json: bool) -> Result<Output, CliError> {
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let from = resolve_scene(&list, &args.from, json)?;
    let to = resolve_scene(&list, &args.to, json)?;
    let groups = from.get("groups").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let group = match groups.iter().find(|g| str_of(g, "id") == args.group) {
        Some(found) => found.clone(),
        None => {
            let named: Vec<&serde_json::Value> =
                groups.iter().filter(|g| str_of(g, "name").eq_ignore_ascii_case(&args.group)).collect();
            match named.as_slice() {
                [one] => (*one).clone(),
                [] => {
                    let names: Vec<&str> = groups.iter().map(|g| str_of(g, "name")).collect();
                    return Err(CliError::Other(format!(
                        "{} has no group named {}; its groups are: {}",
                        str_of(&from, "name"),
                        args.group,
                        names.join(", ")
                    )));
                }
                many => {
                    let ids: Vec<String> =
                        many.iter().map(|g| format!("{} ({})", str_of(g, "id"), str_of(g, "name"))).collect();
                    return Err(CliError::Other(format!(
                        "{} names {} groups in {}: {}; name one of them by its id",
                        args.group,
                        many.len(),
                        str_of(&from, "name"),
                        ids.join(", ")
                    )));
                }
            }
        }
    };
    let rev = |scene: &serde_json::Value| scene.get("rev").and_then(|v| v.as_u64()).unwrap_or_default();
    let body = serde_json::to_string(&canvas_proto::api::GroupMove {
        to: str_of(&to, "id").to_string(),
        copy: args.copy,
        expected_rev_from: rev(&from),
        expected_rev_to: rev(&to),
        dry_run: args.dry_run,
    })
    .map_err(|e| CliError::Other(e.to_string()))?;
    let path = canvas_proto::api::routes::GROUP_MOVE
        .replace("{id}", str_of(&from, "id"))
        .replace("{group}", str_of(&group, "id"));
    let answer = api.call("POST", &path, Some(&body))?;
    if json {
        return Ok(Output::Json(answer));
    }
    let done: canvas_proto::api::GroupMoved = serde_json::from_value(answer)
        .map_err(|e| CliError::Other(format!("the daemon's answer was not a group move: {e}")))?;
    Ok(Output::Text(group_move_text(
        args,
        str_of(&group, "name"),
        str_of(&from, "name"),
        str_of(&to, "name"),
        &done,
    )))
}

/// One line: `moved Desk from Gaming to Movie: 2 devices taken from Everything; 1 group removed`.
/// A dry run starts `would move` / `would copy`.
fn group_move_text(
    args: &GroupMoveArgs,
    group: &str,
    from: &str,
    to: &str,
    done: &canvas_proto::api::GroupMoved,
) -> String {
    let verb = match (args.dry_run, args.copy) {
        (false, false) => "moved",
        (false, true) => "copied",
        (true, false) => "would move",
        (true, true) => "would copy",
    };
    let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut details: Vec<String> = done
        .taken
        .iter()
        .map(|t| format!("{} taken from {}", count(t.lights.len(), "device", "devices"), t.group_name))
        .collect();
    if !done.emptied.is_empty() {
        details.push(format!("{} removed", count(done.emptied.len(), "group", "groups")));
    }
    let tail = if details.is_empty() { String::new() } else { format!(": {}", details.join("; ")) };
    format!("{verb} {group} from {from} to {to}{tail}\n")
}

/// `canvas-rgb lights list [--json]`: every light, its LED count and whether it is connected.
pub fn lights_list(api: &dyn Api, json: bool) -> Result<Output, CliError> {
    let answer = api.call("GET", canvas_proto::api::routes::LIGHTS, None)?;
    Ok(if json {
        Output::Json(answer.clone())
    } else {
        Output::Text(lights_list_text(&answer))
    })
}

/// The lights, one per line: the id, the owner's label, its LEDs and whether it is connected.
pub fn lights_list_text(answer: &serde_json::Value) -> String {
    let lights = answer.get("lights").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if lights.is_empty() {
        return "no lights\n".to_string();
    }
    let mut text = format!("{:<28} {:<28} {:<6} CONNECTED\n", "ID", "LABEL", "LEDS");
    for light in &lights {
        let id = light.get("light").and_then(|l| l.get("id")).and_then(|v| v.as_str()).unwrap_or("?");
        let label = str_of(light, "label");
        let leds = light.get("led_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let connected = if light.get("connected").and_then(|v| v.as_bool()).unwrap_or(false) {
            "yes"
        } else {
            "no"
        };
        text.push_str(&format!("{id:<28} {label:<28} {leds:<6} {connected}\n"));
    }
    text
}

/// `canvas-rgb scenes create <name> [--from <groups.json>] [--run <run>] [--dry-run]` (#377).
///
/// It reads the scenes first: a name another scene already has is refused here, because the
/// daemon lets two scenes share one (the editor's Duplicate relies on it). With `--from` or
/// `--dry-run` it first asks the daemon its version: an older daemon ignores `groups` and
/// `dry_run` and would save a scene, so nothing is sent to one. With `--run` a real create
/// saves the run's record with the create pending before the request goes out, and with the
/// scene noted after: the list it read, the active scene, and the new scene.
pub fn scenes_create(api: &dyn Api, args: &CreateArgs, json: bool) -> Result<Output, CliError> {
    // A run id is checked before it is joined to a path, and before anything is sent.
    let existing = match &args.run {
        Some(run) => crate::agent_run::read(run)?,
        None => None,
    };
    // The file is read before anything is asked of the daemon.
    let groups = match &args.from {
        Some(file) => Some(read_groups(file)?),
        None => None,
    };
    if args.from.is_some() || args.dry_run {
        check_daemon_version(api, env!("CARGO_PKG_VERSION"))?;
    }
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let name = args.name.trim().to_string();
    // A name matches without regard to case, as `scenes set` matches it.
    let taken = scenes_of(&list)
        .into_iter()
        .find(|scene| str_of(scene, "name").eq_ignore_ascii_case(&name));
    if let Some(taken) = taken {
        return Err(CliError::Other(format!(
            "a scene named \"{name}\" already exists (id {}); pick another name",
            str_of(&taken, "id")
        )));
    }
    // Only a real create that the run named goes into the record.
    let mut record = None;
    if let (Some(run), false) = (&args.run, args.dry_run) {
        let mut started = existing.unwrap_or_else(|| crate::agent_run::RunRecord::start(run, &list));
        started.begin("create", &name);
        started
            .save()
            .map_err(|e| CliError::Other(format!("{e}; nothing was sent")))?;
        record = Some(started);
    }
    let key = name.clone();
    let body = serde_json::to_string(&canvas_proto::api::SceneCreate {
        name,
        copy_of: None,
        groups,
        dry_run: args.dry_run,
    })
    .map_err(|e| CliError::Other(e.to_string()))?;
    let answer = match api.call("POST", canvas_proto::api::routes::SCENES, Some(&body)) {
        Ok(answer) => answer,
        Err(e) => {
            if let Some(record) = record.as_mut() {
                settle_pending(record, "create", &key, &e);
            }
            return Err(e);
        }
    };
    let (saved, scene) = match scene_answer(&answer, args.dry_run) {
        Ok(done) => done,
        Err(e) => {
            // A real write the daemon says it did not save left nothing behind to account for.
            if let (Some(record), true) = (record.as_mut(), says_not_saved(&answer)) {
                clear_pending(record, "create", &key);
            }
            return Err(e);
        }
    };
    if let Some(mut record) = record {
        record.note(&scene);
        record.finish("create", &key);
        record.save().map_err(|e| {
            CliError::Other(format!(
                "the scene {} was created, but its run record could not be written: {e}",
                scene.id
            ))
        })?;
    }
    scene_output(true, saved, args.run.as_deref(), &scene, json)
}

/// `canvas-rgb scenes edit <scene> --from <groups.json> --run <run> [--dry-run]` (#377).
///
/// Only a scene the run made may be edited. The daemon's version is checked first (an older
/// daemon ignores `dry_run` and saves). The scene as it is now is copied into the run's folder
/// and the edit is saved to the record as pending before the write is sent, so there is always a
/// way back; the `rev` it read goes with the write, so a scene another window changed is a `409`
/// and nothing is lost.
pub fn scenes_edit(api: &dyn Api, args: &EditArgs, json: bool) -> Result<Output, CliError> {
    let mut record = crate::agent_run::load(&args.run)?;
    let groups = read_groups(&args.from)?;
    check_daemon_version(api, env!("CARGO_PKG_VERSION"))?;
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let found = resolve_scene(&list, &args.scene, json)?;
    let id = str_of(&found, "id").to_string();
    if record.made(&id).is_none() {
        return Err(CliError::Other(format!(
            "{id} was not made by run {}; nothing changed{}",
            args.run,
            pending_note(&record, &args.run)
        )));
    }
    let current: canvas_proto::scene::Scene = serde_json::from_value(found.clone())
        .map_err(|e| CliError::Other(format!("the daemon's answer was not a scene: {e}")))?;
    let groups = groups_with_ids(&groups, &current.groups);
    let scene = canvas_proto::scene::Scene { groups, ..current };
    if !args.dry_run {
        // The copy goes first: a failed write still leaves the scene as it was.
        crate::agent_run::save_scene_copy(&args.run, &found)?;
        record.begin("edit", &id);
        record
            .save()
            .map_err(|e| CliError::Other(format!("{e}; nothing was sent")))?;
    }
    let body = serde_json::to_string(&canvas_proto::api::ScenePut {
        scene,
        expected_rev: found.get("rev").and_then(|v| v.as_u64()).unwrap_or_default(),
        dry_run: args.dry_run,
    })
    .map_err(|e| CliError::Other(e.to_string()))?;
    let path = canvas_proto::api::routes::SCENE.replace("{id}", &id);
    let answer = match api.call("PUT", &path, Some(&body)) {
        Ok(answer) => answer,
        Err(e) => {
            if !args.dry_run {
                settle_pending(&mut record, "edit", &id, &e);
            }
            return Err(e);
        }
    };
    let (saved, scene) = match scene_answer(&answer, args.dry_run) {
        Ok(done) => done,
        Err(e) => {
            // A real write the daemon says it did not save left nothing behind to account for.
            if !args.dry_run && says_not_saved(&answer) {
                clear_pending(&mut record, "edit", &id);
            }
            return Err(e);
        }
    };
    if !args.dry_run {
        record.note(&scene);
        record.finish("edit", &id);
        record.save().map_err(|e| {
            CliError::Other(format!("the scene {} was saved, but its run record could not be written: {e}", scene.id))
        })?;
    }
    scene_output(false, saved, Some(args.run.as_str()), &scene, json)
}

/// `canvas-rgb scenes delete <scene> --run <run> [--force] [--dry-run]` (#377).
///
/// Three refusals, all before anything is sent: a scene the run did not make, the active scene
/// (the daemon would not refuse it: it switches the lights to another scene instead), and a
/// scene that changed since the run made it unless `--force` says otherwise. `--force` skips
/// only the last one. The scene is copied into the run's folder, and the delete is saved to the
/// record as pending, before the delete is sent.
///
/// A scene the record does not list is never deleted, even when the record holds a pending
/// create of its name: that create has no known outcome, and a scene of the same name may be
/// one someone made by hand. The refusal says so, and the scene is left for the app.
pub fn scenes_delete(api: &dyn Api, args: &DeleteArgs, json: bool) -> Result<Output, CliError> {
    let mut record = crate::agent_run::load(&args.run)?;
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let found = resolve_scene(&list, &args.scene, json)?;
    let id = str_of(&found, "id").to_string();
    let name = str_of(&found, "name").to_string();
    let Some(made) = record.made(&id).cloned() else {
        return Err(CliError::Other(delete_refusal(&record, &args.run, &id, &name)));
    };
    if list.get("active").and_then(|v| v.as_str()) == Some(id.as_str()) {
        return Err(CliError::Other(format!(
            "{name} is the active scene. Switch first: canvas-rgb scenes set \"{}\"",
            record.prior_active
        )));
    }
    let now = found.get("rev").and_then(|v| v.as_u64()).unwrap_or_default();
    if now != made.rev && !args.force {
        return Err(CliError::Other(format!(
            "{name} changed since run {} made it (rev {now}, was {}); nothing deleted. Use --force to delete it anyway.",
            args.run, made.rev
        )));
    }
    if !args.dry_run {
        // The copy goes first: the scene is still there if the delete fails.
        crate::agent_run::save_scene_copy(&args.run, &found)?;
        record.begin("delete", &id);
        record
            .save()
            .map_err(|e| CliError::Other(format!("{e}; nothing was sent")))?;
        let path = canvas_proto::api::routes::SCENE.replace("{id}", &id);
        if let Err(e) = api.call("DELETE", &path, None) {
            settle_pending(&mut record, "delete", &id, &e);
            return Err(e);
        }
        record.forget(&id);
        record.finish("delete", &id);
        record.save().map_err(|e| {
            CliError::Other(format!(
                "the scene {id} was deleted, but its run record could not be written: {e}"
            ))
        })?;
    }
    Ok(if json {
        Output::Json(serde_json::json!({ "saved": !args.dry_run, "run": args.run, "deleted": id }))
    } else {
        let said = if args.dry_run { "would delete" } else { "deleted" };
        Output::Text(format!("{said} {id}\n"))
    })
}

/// The refusal for a scene the run's record does not list. When the record holds a pending
/// create of this scene's name (case aside), it says that create has no known outcome and leaves
/// the scene to the app; otherwise it names any pending writes the run holds.
fn delete_refusal(record: &crate::agent_run::RunRecord, run: &str, id: &str, name: &str) -> String {
    let same_name = record
        .pending
        .iter()
        .any(|p| p.op == "create" && p.scene.trim().eq_ignore_ascii_case(name.trim()));
    if same_name {
        return format!(
            "{id} was not recorded as made by run {run}, and a create of \"{name}\" by this run has no known outcome, so nothing was deleted; delete it in the app if it is the run's."
        );
    }
    format!("{id} was not made by run {run}; nothing changed{}", pending_note(record, run))
}

/// A sentence naming the writes a run's record holds with no known outcome, or nothing when it
/// holds none: ` Run x holds writes with no known outcome: create Night, delete movie.`
fn pending_note(record: &crate::agent_run::RunRecord, run: &str) -> String {
    if record.pending.is_empty() {
        return String::new();
    }
    let list: Vec<String> = record.pending.iter().map(|p| format!("{} {}", p.op, p.scene)).collect();
    format!(". Run {run} holds writes with no known outcome: {}.", list.join(", "))
}

/// A write failed after its pending entry was saved. The entry is removed only when the write
/// certainly did not happen: the daemon refused it (an API error under `500`), or nothing was
/// ever connected to (`NotRunning`, `NotTrusted`). Every other failure keeps the entry, because
/// the request may have reached the daemon first: no answer, a `5xx`, and an error the transport
/// raised mid-request (a reset after the body went out). The record is saved again after the
/// removal; if that fails, the entry stays, which only says more.
fn settle_pending(record: &mut crate::agent_run::RunRecord, op: &str, scene: &str, error: &CliError) {
    let refused = match error {
        CliError::NotRunning(_) | CliError::NotTrusted(_) => true,
        CliError::Api { status, .. } => *status < 500,
        CliError::NoAnswer(_) | CliError::Other(_) => false,
    };
    if refused {
        clear_pending(record, op, scene);
    }
}

/// Removes a pending entry and saves the record; a failed save leaves the entry on disk.
fn clear_pending(record: &mut crate::agent_run::RunRecord, op: &str, scene: &str) {
    record.finish(op, scene);
    let _ = record.save();
}

/// True when an answer says, in so many words, that the daemon did not save.
fn says_not_saved(answer: &serde_json::Value) -> bool {
    answer.get("saved").and_then(|v| v.as_bool()) == Some(false)
}

/// The parts of a version `major.minor.patch`, with a leading `v` and any `-pre` or `+build`
/// tail dropped. Anything else cannot be compared and answers `None`.
fn version_parts(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Asks the daemon its version (the `version` of the status the `status` command prints) and
/// refuses when it is older than `own`, this command line's version, or cannot be read. A
/// daemon older than the command ignores `groups` and `dry_run` and WRITES, and a check after
/// the answer is too late, so this runs before any write is sent.
fn check_daemon_version(api: &dyn Api, own: &str) -> Result<(), CliError> {
    let status = api.call("GET", canvas_proto::api::routes::STATUS, None)?;
    let reported = status.get("version").and_then(|v| v.as_str());
    let tail = "restart canvas-rgb, then try again; nothing was changed.";
    let Some(reported) = reported else {
        return Err(CliError::Other(format!(
            "the running canvas-rgb reported no version, and this command line is version {own}: {tail}"
        )));
    };
    let (Some(daemon), Some(ours)) = (version_parts(reported), version_parts(own)) else {
        return Err(CliError::Other(format!(
            "the running canvas-rgb reported version \"{reported}\", which cannot be read, and this command line is version {own}: {tail}"
        )));
    };
    if daemon < ours {
        return Err(CliError::Other(format!(
            "the running canvas-rgb is version {reported}, older than this command line's {own}: {tail}"
        )));
    }
    Ok(())
}

/// The scenes in a scenes answer.
fn scenes_of(list: &serde_json::Value) -> Vec<serde_json::Value> {
    list.get("scenes").and_then(|v| v.as_array()).cloned().unwrap_or_default()
}

/// The most a `--from` file may hold: 1 MiB.
const GROUPS_FILE_MAX: u64 = 1024 * 1024;

/// The groups a `--from <groups.json>` file holds: an array of [`canvas_proto::api::NewGroup`].
/// A file over [`GROUPS_FILE_MAX`] is refused without being read whole.
fn read_groups(file: &str) -> Result<Vec<canvas_proto::api::NewGroup>, CliError> {
    use std::io::Read as _;
    let unreadable = |e: std::io::Error| CliError::Other(format!("could not read {file}: {e}"));
    let mut text = String::new();
    std::fs::File::open(file)
        .map_err(unreadable)?
        .take(GROUPS_FILE_MAX + 1)
        .read_to_string(&mut text)
        .map_err(unreadable)?;
    if text.len() as u64 > GROUPS_FILE_MAX {
        return Err(CliError::Other(format!("{file} is over 1 MiB, more than a groups file may hold")));
    }
    serde_json::from_str(&text)
        .map_err(|e| CliError::Other(format!("{file} is not a list of groups: {e}")))
}

/// The groups a scene holds after an edit: each [`canvas_proto::api::NewGroup`] with an id.
///
/// A group keeps the id the scene's group of the same name had, so a group that only changed
/// its props or its brightness keeps the id the editor knows it by; a new name gets
/// [`slug`](crate::scenes::slug) and, when that is taken, a number.
fn groups_with_ids(
    groups: &[canvas_proto::api::NewGroup],
    current: &[canvas_proto::scene::Group],
) -> Vec<canvas_proto::scene::Group> {
    let mut used: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let kept = current
            .iter()
            .find(|old| old.name == group.name && !used.contains(&old.id))
            .map(|old| old.id.clone());
        let id = match kept {
            Some(id) => id,
            None => crate::scenes::fresh_id(
                &crate::scenes::slug(&group.name),
                used.iter().map(String::as_str),
            ),
        };
        used.push(id.clone());
        out.push(canvas_proto::scene::Group {
            id,
            name: group.name.clone(),
            members: group.members.clone(),
            plays: group.plays.clone(),
            mode: group.mode,
            brightness: group.brightness,
        });
    }
    out
}

/// What a create, an edit or a dry run of either answers: whether it was saved, and the scene.
///
/// A real write answers the plain [`canvas_proto::scene::Scene`], with no `saved` field; a dry
/// run answers a [`canvas_proto::api::SceneDryRun`], the same scene with `saved: false` beside
/// it. A dry run whose answer lacks `saved: false` came from a daemon that ignored `dry_run`, so
/// it may have written: that is an error, and so is a real write the daemon says it did not save.
fn scene_answer(
    answer: &serde_json::Value,
    dry_run: bool,
) -> Result<(bool, canvas_proto::scene::Scene), CliError> {
    let saved = match (dry_run, answer.get("saved").and_then(|v| v.as_bool())) {
        (true, Some(false)) => false,
        (true, Some(true)) => {
            return Err(CliError::Other(
                "this was a dry run, but the daemon says it saved the scene".to_string(),
            ));
        }
        (true, None) => {
            return Err(CliError::Other(
                "the daemon did not say whether it saved: this was a dry run, so it may have saved the scene; check canvas-rgb scenes list"
                    .to_string(),
            ));
        }
        (false, Some(false)) => {
            return Err(CliError::Other("the daemon says it did not save the scene".to_string()));
        }
        (false, _) => true,
    };
    let scene = answer.get("scene").cloned().unwrap_or_else(|| answer.clone());
    let scene = serde_json::from_value(scene)
        .map_err(|e| CliError::Other(format!("the daemon's answer was not a scene: {e}")))?;
    Ok((saved, scene))
}

/// What a create or an edit prints: `created desk: 2 groups` or `saved desk: 2 groups`, and a
/// dry run's own words. With `--json` it is one document: the `saved` flag, the run (when one
/// was named) and the scene as the daemon answered it.
fn scene_output(
    created: bool,
    saved: bool,
    run: Option<&str>,
    scene: &canvas_proto::scene::Scene,
    json: bool,
) -> Result<Output, CliError> {
    let count = scene.groups.len();
    if json {
        return Ok(Output::Json(serde_json::json!({
            "saved": saved,
            "run": run,
            "scene": scene,
        })));
    }
    let verb = match (created, saved) {
        (true, true) => "created",
        (true, false) => "would create",
        (false, true) => "saved",
        (false, false) => "would save",
    };
    let unsaved = if saved { "" } else { " (nothing saved)" };
    Ok(Output::Text(format!(
        "{verb} {}: {count} {}{unsaved}\n",
        scene.id,
        groups_word(count)
    )))
}

/// `1 group` or `2 groups`.
fn groups_word(count: usize) -> &'static str {
    if count == 1 {
        "group"
    } else {
        "groups"
    }
}

/// Sets the scene the answer names.
fn set_scene(api: &dyn Api, scene: &serde_json::Value, json: bool) -> Result<Output, CliError> {
    let id = scene.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let canonical = scene.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let body = serde_json::json!({ "id": id }).to_string();
    let answer = api.call("PUT", canvas_proto::api::routes::SCENE_ACTIVE, Some(&body))?;
    Ok(if json {
        Output::Json(answer.clone())
    } else {
        Output::Text(format!("the scene is {canonical}\n"))
    })
}

/// The scenes, one per line: the id, the name, and a mark on the active one.
pub fn scenes_list_text(answer: &serde_json::Value) -> String {
    let active = answer.get("active").and_then(|v| v.as_str()).unwrap_or("");
    let scenes = answer.get("scenes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if scenes.is_empty() {
        return "no scenes\n".into();
    }
    let mut out = String::new();
    for scene in &scenes {
        let id = scene.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = scene.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let marker = if id == active { "*" } else { " " };
        out.push_str(&format!("{marker} {id:<24} {name}\n"));
    }
    out
}

// Effects and presets (#377).
//
// None of these commands needs [`check_daemon_version`]: the version gate exists for a request
// whose body carries a field an older daemon would ignore and then write without. Every request
// here is one the daemon has served since 0.10 (a preset POST and DELETE, an effect DELETE and its
// `keep_presets` query, an import and its `replace`, the source route), and an older daemon that
// lacks a route answers 404, loudly, with nothing written.

/// `text` as one path segment: everything but `A-Z a-z 0-9 - . _ ~` is percent-encoded, so an
/// effect id like `html:my effect` or a preset name with a `/` in it cannot change the route.
fn path_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A route with `{id}` filled in with an effect's id.
fn effect_route(route: &str, effect: &str) -> String {
    route.replace("{id}", &path_segment(effect))
}

/// The route of one preset of an effect.
fn preset_route(effect: &str, name: &str) -> String {
    canvas_proto::api::routes::EFFECT_PRESET
        .replace("{id}", &path_segment(effect))
        .replace("{name}", &path_segment(name))
}

/// The effect's canonical id in an `GET /effects/{id}` answer, or what the caller typed.
fn effect_id_of(detail: &serde_json::Value, typed: &str) -> String {
    detail
        .get("schema")
        .and_then(|schema| schema.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or(typed)
        .to_string()
}

/// Reads a file of at most `max` bytes as text, reading no more than `max + 1` bytes of it. A
/// file over the cap is refused with one sentence before anything is sent.
fn read_capped(file: &str, max: usize, what: &str) -> Result<String, CliError> {
    use std::io::Read as _;
    let unreadable = |e: std::io::Error| CliError::Other(format!("could not read {file}: {e}"));
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .map_err(unreadable)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() > max {
        return Err(CliError::Other(format!("{file} is over {max} bytes, more than {what} may hold")));
    }
    String::from_utf8(bytes).map_err(|_| CliError::Other(format!("{file} is not text (UTF-8)")))
}

/// Writes `bytes` to a file that does not exist yet.
fn write_new_file(path: &str, bytes: &[u8]) -> Result<(), CliError> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                CliError::Other(format!("{path} already exists; pick another file name"))
            } else {
                CliError::Other(format!("could not create {path}: {e}"))
            }
        })?;
    file.write_all(bytes)
        .map_err(|e| CliError::Other(format!("could not write {path}: {e}")))
}

/// The record of a run that is about to write for the first time: the one it has, or a new one
/// started from the scenes as they are now (the `before` an undo needs).
fn record_for_write(
    api: &dyn Api,
    run: &str,
    existing: Option<crate::agent_run::RunRecord>,
) -> Result<crate::agent_run::RunRecord, CliError> {
    match existing {
        Some(record) => Ok(record),
        None => {
            let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
            Ok(crate::agent_run::RunRecord::start(run, &list))
        }
    }
}

/// Saves the record with a write pending, before the request goes out. A record that cannot be
/// written sends nothing.
fn begin_write(record: &mut crate::agent_run::RunRecord, op: &str, what: &str) -> Result<(), CliError> {
    record.begin(op, what);
    record
        .save()
        .map(|_| ())
        .map_err(|e| CliError::Other(format!("{e}; nothing was sent")))
}

/// The refusal for a thing a run did not make: `<what> was not made by run <run>; nothing
/// changed`, with the writes the run holds with no known outcome named.
fn not_made_by(record: &crate::agent_run::RunRecord, run: &str, what: &str) -> CliError {
    CliError::Other(format!("{what} was not made by run {run}; nothing changed{}", pending_note(record, run)))
}

/// True when any group of the scene plays the effect (the id matches without regard to case).
fn scene_plays_effect(scene: &serde_json::Value, effect: &str) -> bool {
    scene.get("groups").and_then(|v| v.as_array()).is_some_and(|groups| {
        groups.iter().any(|group| {
            let plays = group.get("plays");
            plays.and_then(|p| p.get("type")).and_then(|v| v.as_str()) == Some("effect")
                && plays
                    .and_then(|p| p.get("effect"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|played| played.eq_ignore_ascii_case(effect))
        })
    })
}

/// True when a JSON string field names the effect (the ids match without regard to case).
fn names_effect(field: Option<&serde_json::Value>, effect: &str) -> bool {
    field.and_then(|v| v.as_str()).is_some_and(|id| id.eq_ignore_ascii_case(effect))
}

/// True when a layer of a stack names the effect, as the layer's own effect or as its mask's (the
/// ids match without regard to case).
fn stack_names_effect(stack: &serde_json::Value, effect: &str) -> bool {
    stack.get("layers").and_then(|v| v.as_array()).is_some_and(|layers| {
        layers.iter().any(|layer| {
            names_effect(layer.get("effect"), effect)
                || names_effect(layer.get("mask").and_then(|mask| mask.get("effect")), effect)
        })
    })
}

/// The names of the saved stacks that name the effect as a layer or as a layer's mask, quoted, in
/// the order the daemon lists them. The list route carries no layer ids, so each stack is read
/// whole through the same client; a stack that cannot be read refuses the delete rather than
/// letting it go blind.
fn stacks_naming_effect(api: &dyn Api, effect: &str) -> Result<Vec<String>, CliError> {
    let listed = api.call("GET", canvas_proto::api::routes::STACKS, None)?;
    let mut names = Vec::new();
    for summary in listed.get("stacks").and_then(|v| v.as_array()).into_iter().flatten() {
        let id = str_of(summary, "id");
        let detail = api.call("GET", &effect_route(canvas_proto::api::routes::STACK, id), None)?;
        if detail.get("stack").is_some_and(|stack| stack_names_effect(stack, effect)) {
            names.push(format!("\"{}\"", str_of(summary, "name")));
        }
    }
    Ok(names)
}

/// The effect's own presets, from a `GET /effects/{id}` answer. A missing `user_presets`, or one
/// that is not an array, is an error and never "no presets": a copy has to be the effect's whole
/// content, so a delete must not drop presets it failed to copy. `after` says what did not happen.
fn user_presets_of<'a>(detail: &'a serde_json::Value, after: &str) -> Result<&'a Vec<serde_json::Value>, CliError> {
    detail
        .get("user_presets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CliError::Other(format!("could not read the effect's presets; {after}")))
}

/// The whole of an effect, copied into the run's folder before it is deleted or written over: its
/// id, file name and source, and with `presets` also its user presets (a delete takes them too).
/// The effect's source is read from the daemon; an answer with no source, or with no readable list
/// of presets, is an error, and nothing is copied or sent.
fn copy_effect(api: &dyn Api, run: &str, effect: &str, presets: bool) -> Result<(), CliError> {
    let answer = api.call("GET", &effect_route(canvas_proto::api::routes::EFFECT_SOURCE, effect), None)?;
    let Some(source) = answer.get("source").and_then(|v| v.as_str()) else {
        return Err(CliError::Other(format!("the daemon sent no source for {effect}; nothing changed")));
    };
    let mut doc = serde_json::json!({
        "id": effect,
        "file_name": answer.get("file_name").cloned().unwrap_or(serde_json::Value::Null),
        "source": source,
    });
    if presets {
        let detail = api.call("GET", &effect_route(canvas_proto::api::routes::EFFECT, effect), None)?;
        doc["user_presets"] =
            serde_json::Value::Array(user_presets_of(&detail, "nothing deleted")?.clone());
    }
    crate::agent_run::save_effect_copy(run, &doc)?;
    Ok(())
}

/// `canvas-rgb effects import <file> [--replace] [--run <run>]`.
///
/// The file is read first, capped at [`canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES`]: a file
/// that is missing or too big is a local problem, reported without asking the daemon anything.
/// With `--run` the import is saved to the run's record as pending before the request goes out,
/// and the new effect's id is noted after. With `--run` and `--replace`, an effect that already
/// has the file's name is overwritten only when the run imported it, and its whole content is
/// copied into the run's folder first; any other is refused. Without `--run` nothing is recorded
/// or copied, as before.
pub fn effects_import(api: &dyn Api, args: &ImportArgs, json: bool) -> Result<Output, CliError> {
    // A run id is checked before it is joined to a path, and before anything is read or sent.
    let existing = match &args.run {
        Some(run) => crate::agent_run::read(run)?,
        None => None,
    };
    let path = std::path::Path::new(&args.file);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| CliError::Other(format!("{} has no file name", args.file)))?;
    let html = read_capped(&args.file, canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES, "an effect file")?;

    let mut record = None;
    if let Some(run) = &args.run {
        if args.replace {
            // The id the file will have, as the daemon derives it: `html:` and the file's stem.
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let id = format!("{}{stem}", crate::user_effects::USER_EFFECT_PREFIX);
            let listed = api.call("GET", canvas_proto::api::routes::EFFECTS, None)?;
            let exists = listed
                .get("effects")
                .and_then(|v| v.as_array())
                .is_some_and(|list| list.iter().any(|e| str_of(e, "id").eq_ignore_ascii_case(&id)));
            if exists {
                let made = existing.as_ref().and_then(|r| r.made_effect(&id)).map(|m| m.id.clone());
                let Some(made) = made else {
                    return Err(match &existing {
                        Some(record) => not_made_by(record, run, &id),
                        None => CliError::Other(format!("{id} was not made by run {run}; nothing changed")),
                    });
                };
                // The copy goes first: a failed copy sends nothing.
                copy_effect(api, run, &made, false)?;
            }
        }
        let mut started = record_for_write(api, run, existing)?;
        begin_write(&mut started, "import", &file_name)?;
        record = Some(started);
    }
    let body = serde_json::json!({ "file_name": file_name, "html": html, "replace": args.replace }).to_string();
    let answer = match api.call("POST", canvas_proto::api::routes::EFFECTS_IMPORT, Some(&body)) {
        Ok(answer) => answer,
        Err(e) => {
            if let Some(record) = record.as_mut() {
                settle_pending(record, "import", &file_name, &e);
            }
            return Err(e);
        }
    };
    let effect = answer.get("effect");
    let id = effect.and_then(|e| e.get("id")).and_then(|v| v.as_str()).map(str::to_string);
    let name = effect.and_then(|e| e.get("name")).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let replaced = answer.get("replaced").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Some(mut record) = record {
        // An answer with no id leaves the pending entry: the effect may be there, unnamed.
        let Some(id) = &id else {
            return Err(CliError::Other(format!(
                "the daemon's answer named no effect, so {file_name} is not in the run record; check canvas-rgb effects list"
            )));
        };
        record.note_effect(id);
        record.finish("import", &file_name);
        record.save().map_err(|e| {
            CliError::Other(format!("the effect {id} was imported, but its run record could not be written: {e}"))
        })?;
    }
    let id = id.unwrap_or_else(|| "?".to_string());
    Ok(if json {
        Output::Json(serde_json::json!({
            "saved": true,
            "run": args.run,
            "effect": effect.cloned().unwrap_or(serde_json::Value::Null),
            "replaced": replaced,
        }))
    } else if replaced {
        Output::Text(format!("imported {id} ({name}); it replaced the file of the same name\n"))
    } else {
        Output::Text(format!("imported {id} ({name})\n"))
    })
}

/// `canvas-rgb effects source <effect> [--out <file>]`: an `html:` effect's source, stock ones
/// included. Printed as it is, or written to a new file (an existing file is never written over).
pub fn effects_source(api: &dyn Api, args: &SourceArgs, json: bool) -> Result<Output, CliError> {
    let answer = api.call("GET", &effect_route(canvas_proto::api::routes::EFFECT_SOURCE, &args.effect), None)?;
    let Some(source) = answer.get("source").and_then(|v| v.as_str()) else {
        return Err(CliError::Other(format!("the daemon sent no source for {}", args.effect)));
    };
    let Some(out) = &args.out else {
        return Ok(if json { Output::Json(answer.clone()) } else { Output::Text(source.to_string()) });
    };
    write_new_file(out, source.as_bytes())?;
    Ok(if json {
        Output::Json(serde_json::json!({
            "id": args.effect,
            "file_name": answer.get("file_name").cloned().unwrap_or(serde_json::Value::Null),
            "out": out,
            "bytes": source.len(),
        }))
    } else {
        Output::Text(format!("wrote {} bytes of {} to {out}\n", source.len(), args.effect))
    })
}

/// `canvas-rgb effects delete <effect> [--keep-presets] --run <run> [--dry-run]`.
///
/// Refused, before anything is sent, for an effect the run did not import, for an effect any
/// scene still plays, and for an effect a saved stack uses as a layer or as a layer's mask (the
/// scenes and the stacks are named; there is no flag to force it, because those groups and
/// layers would go dark). The effect's whole content is copied into the run's folder, and the
/// delete is saved to the record as pending, before the delete is sent. A dry run runs the checks
/// and sends, copies and writes nothing.
///
/// The scenes and the stacks are read before the delete and the daemon does not refuse: a scene
/// or a stack edited into the effect between that read and the DELETE is not seen. The gap is
/// named in `.build/reports/fixpass-412.md` (item 3, which lives in the main checkout), and a
/// daemon-side refusal is the follow-up.
pub fn effects_delete(api: &dyn Api, args: &EffectDeleteArgs, json: bool) -> Result<Output, CliError> {
    let mut record = crate::agent_run::load(&args.run)?;
    let Some(made) = record.made_effect(&args.effect).map(|m| m.id.clone()) else {
        return Err(not_made_by(&record, &args.run, &args.effect));
    };
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let players: Vec<String> = scenes_of(&list)
        .iter()
        .filter(|scene| scene_plays_effect(scene, &made))
        .map(|scene| format!("\"{}\"", str_of(scene, "name")))
        .collect();
    if !players.is_empty() {
        return Err(CliError::Other(format!(
            "{made} is played by {} {}; nothing deleted. Move those groups to another effect first.",
            if players.len() == 1 { "the scene" } else { "the scenes" },
            players.join(", ")
        )));
    }
    let stackers = stacks_naming_effect(api, &made)?;
    if !stackers.is_empty() {
        return Err(CliError::Other(format!(
            "{made} is a layer or a mask of {} {}; nothing deleted. Change those layers to another effect first.",
            if stackers.len() == 1 { "the stack" } else { "the stacks" },
            stackers.join(", ")
        )));
    }
    if args.dry_run {
        return Ok(effect_deleted_output(&args.run, &made, None, true, json));
    }
    // The copy goes first: the effect is still there if the delete fails.
    copy_effect(api, &args.run, &made, true)?;
    begin_write(&mut record, "effect-delete", &made)?;
    let mut path = effect_route(canvas_proto::api::routes::EFFECT, &made);
    if args.keep_presets {
        path.push_str("?keep_presets=true");
    }
    let answer = match api.call("DELETE", &path, None) {
        Ok(answer) => answer,
        Err(e) => {
            settle_pending(&mut record, "effect-delete", &made, &e);
            return Err(e);
        }
    };
    record.forget_effect(&made);
    if !args.keep_presets {
        record.forget_presets_of(&made);
    }
    record.finish("effect-delete", &made);
    record.save().map_err(|e| {
        CliError::Other(format!("the effect {made} was deleted, but its run record could not be written: {e}"))
    })?;
    Ok(effect_deleted_output(&args.run, &made, Some(&answer), false, json))
}

/// What `effects delete` prints: `deleted html:x (2 presets removed)`, or `would delete html:x`.
fn effect_deleted_output(
    run: &str,
    id: &str,
    answer: Option<&serde_json::Value>,
    dry_run: bool,
    json: bool,
) -> Output {
    let removed = answer.and_then(|a| a.get("presets_removed")).and_then(|v| v.as_u64()).unwrap_or(0);
    if json {
        return Output::Json(serde_json::json!({
            "saved": !dry_run,
            "run": run,
            "deleted": id,
            "presets_removed": removed,
        }));
    }
    let said = if dry_run { "would delete" } else { "deleted" };
    let tail = match removed {
        0 => String::new(),
        1 => " (1 preset removed)".to_string(),
        n => format!(" ({n} presets removed)"),
    };
    Output::Text(format!("{said} {id}{tail}\n"))
}

/// `canvas-rgb effects preset list <effect>`: the built-in presets and the user's own.
///
/// The answer must carry the effect's own list: "none of yours" is a list, and a missing or
/// unreadable one is an error rather than an effect that looks presetless.
pub fn preset_list(api: &dyn Api, args: &PresetListArgs, json: bool) -> Result<Output, CliError> {
    let detail = api.call("GET", &effect_route(canvas_proto::api::routes::EFFECT, &args.effect), None)?;
    let built_in = detail
        .get("schema")
        .and_then(|schema| schema.get("presets"))
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
    let user = serde_json::Value::Array(user_presets_of(&detail, "nothing changed")?.clone());
    if json {
        return Ok(Output::Json(serde_json::json!({
            "effect": effect_id_of(&detail, &args.effect),
            "presets": built_in,
            "user_presets": user,
        })));
    }
    Ok(Output::Text(preset_list_text(&built_in, &user)))
}

/// The presets, one per line: where it comes from, its name, and its properties as `key=value`.
fn preset_list_text(built_in: &serde_json::Value, user: &serde_json::Value) -> String {
    let mut text = String::new();
    for (source, list) in [("built-in", built_in), ("yours", user)] {
        for preset in list.as_array().into_iter().flatten() {
            let props = preset
                .get("props")
                .and_then(|v| v.as_object())
                .map(|props| {
                    props
                        .iter()
                        .map(|(key, value)| format!("{key}={}", prop_value_text(value)))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            text.push_str(&format!("{source:<9} {:<40} {props}\n", str_of(preset, "name")));
        }
    }
    if text.is_empty() {
        "no presets\n".to_string()
    } else {
        text
    }
}

/// `canvas-rgb effects preset save <effect> <name> [--prop k=v]... [--run <run>]`.
///
/// The properties are parsed, and checked against the effect's schema, before anything is
/// written. A save is a POST only: a name that already exists (a built-in preset's included) is a
/// `409`, never an overwrite. With `--run` the save is pending in the record before the request
/// goes out and the preset is noted after.
pub fn preset_save(api: &dyn Api, args: &PresetSaveArgs, json: bool) -> Result<Output, CliError> {
    let existing = match &args.run {
        Some(run) => crate::agent_run::read(run)?,
        None => None,
    };
    let pairs = parse_prop_args(&args.props)?;
    let detail = api.call("GET", &effect_route(canvas_proto::api::routes::EFFECT, &args.effect), None)?;
    let effect = effect_id_of(&detail, &args.effect);
    let properties = detail
        .get("schema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let props = typed_props(&pairs, |key| json_prop_kind(&properties, key))?;
    let name = args.name.trim().to_string();
    let what = format!("{effect}/{name}");

    let mut record = None;
    if let Some(run) = &args.run {
        let mut started = record_for_write(api, run, existing)?;
        begin_write(&mut started, "preset-save", &what)?;
        record = Some(started);
    }
    let body = serde_json::to_string(&canvas_proto::api::PresetCreate { name: name.clone(), props })
        .map_err(|e| CliError::Other(e.to_string()))?;
    let answer = match api.call("POST", &effect_route(canvas_proto::api::routes::EFFECT_PRESETS, &effect), Some(&body)) {
        Ok(answer) => answer,
        Err(e) => {
            if let Some(record) = record.as_mut() {
                settle_pending(record, "preset-save", &what, &e);
            }
            return Err(e);
        }
    };
    if let Some(mut record) = record {
        record.note_preset(&effect, &name);
        record.finish("preset-save", &what);
        record.save().map_err(|e| {
            CliError::Other(format!(
                "the preset \"{name}\" of {effect} was saved, but its run record could not be written: {e}"
            ))
        })?;
    }
    Ok(if json {
        Output::Json(serde_json::json!({
            "saved": true,
            "run": args.run,
            "effect": effect,
            "preset": name,
            "user_presets": answer.get("user_presets").cloned().unwrap_or(serde_json::Value::Null),
        }))
    } else {
        Output::Text(format!("saved preset \"{name}\" of {effect}\n"))
    })
}

/// `canvas-rgb effects preset delete <effect> <name> --run <run> [--dry-run]`.
///
/// Only a preset the run saved may be deleted: any other is refused before anything is sent. The
/// preset is copied whole into the run's folder, and the delete is saved to the record as
/// pending, before the delete is sent. A dry run runs the checks and writes nothing.
pub fn preset_delete(api: &dyn Api, args: &PresetDeleteArgs, json: bool) -> Result<Output, CliError> {
    let mut record = crate::agent_run::load(&args.run)?;
    let detail = api.call("GET", &effect_route(canvas_proto::api::routes::EFFECT, &args.effect), None)?;
    let effect = effect_id_of(&detail, &args.effect);
    let wanted = args.name.trim();
    let found = detail
        .get("user_presets")
        .and_then(|v| v.as_array())
        .and_then(|list| list.iter().find(|p| str_of(p, "name").eq_ignore_ascii_case(wanted)))
        .cloned();
    let Some(found) = found else {
        return Err(CliError::Other(format!(
            "{effect} has no preset of yours named \"{wanted}\"; nothing changed"
        )));
    };
    let name = str_of(&found, "name").to_string();
    if record.made_preset(&effect, &name).is_none() {
        return Err(not_made_by(&record, &args.run, &format!("the preset \"{name}\" of {effect}")));
    }
    if !args.dry_run {
        let what = format!("{effect}/{name}");
        // The copy goes first: the preset is still there if the delete fails.
        crate::agent_run::save_preset_copy(&args.run, &effect, &found)?;
        begin_write(&mut record, "preset-delete", &what)?;
        if let Err(e) = api.call("DELETE", &preset_route(&effect, &name), None) {
            settle_pending(&mut record, "preset-delete", &what, &e);
            return Err(e);
        }
        record.forget_preset(&effect, &name);
        record.finish("preset-delete", &what);
        record.save().map_err(|e| {
            CliError::Other(format!(
                "the preset \"{name}\" of {effect} was deleted, but its run record could not be written: {e}"
            ))
        })?;
    }
    Ok(if json {
        Output::Json(serde_json::json!({
            "saved": !args.dry_run,
            "run": args.run,
            "deleted": { "effect": effect, "preset": name },
        }))
    } else {
        let said = if args.dry_run { "would delete" } else { "deleted" };
        Output::Text(format!("{said} preset \"{name}\" of {effect}\n"))
    })
}

/// `canvas-rgb fans status`: the fan headers, the temperatures and the active profile.
///
/// The JSON answer is the API's own, unchanged: it carries every field, `available`
/// among them. The text is the table below.
pub fn fans_status(api: &dyn Api, json: bool) -> Result<Output, CliError> {
    let answer = api.call("GET", canvas_proto::api::routes::FANS, None)?;
    Ok(if json {
        Output::Json(answer.clone())
    } else {
        Output::Text(fans_status_text(&answer))
    })
}

/// `canvas-rgb peripherals list`: the peripherals the adapters see.
///
/// The enumeration opens nothing: a listed peripheral is not one the daemon may touch.
pub fn peripherals_list(api: &dyn Api, json: bool) -> Result<Output, CliError> {
    let answer = api.call("GET", canvas_proto::api::routes::PERIPHERALS, None)?;
    Ok(if json {
        Output::Json(answer.clone())
    } else {
        Output::Text(peripherals_list_text(&answer))
    })
}

/// `canvas-rgb peripherals show <id>`: what the device is, what it has and what it holds now.
pub fn peripherals_show(api: &dyn Api, id: &str, json: bool) -> Result<Output, CliError> {
    let answer = api.call("GET", &peripheral_path(id), None)?;
    Ok(if json {
        Output::Json(answer.clone())
    } else {
        Output::Text(peripherals_show_text(&answer))
    })
}

/// `canvas-rgb peripherals access <id> --reads on|off --writes on|off`.
///
/// A flag that is not given keeps what it is now, so the daemon is asked for the current
/// access first.
pub fn peripherals_access(
    api: &dyn Api,
    id: &str,
    reads: Option<bool>,
    writes: Option<bool>,
    json: bool,
) -> Result<Output, CliError> {
    if reads.is_none() && writes.is_none() {
        return Err(CliError::Other(
            "give --reads and/or --writes".to_string(),
        ));
    }
    let mut access = PeripheralAccess::default();
    if reads.is_none() || writes.is_none() {
        // One flag only: the other keeps the value it has now.
        let detail = api.call("GET", &peripheral_path(id), None)?;
        let current = detail
            .get("access")
            .cloned()
            .ok_or_else(|| CliError::Other(format!("the daemon did not report the access of {id}")))?;
        access = serde_json::from_value(current)
            .map_err(|e| CliError::Other(format!("could not read the access of {id}: {e}")))?;
    }
    if let Some(reads) = reads {
        access.reads = reads;
    }
    if let Some(writes) = writes {
        access.writes = writes;
    }
    if access.writes && !access.reads {
        return Err(CliError::Other(
            "config writes need reads: turn reads on too".to_string(),
        ));
    }
    let body = serde_json::to_string(&access).map_err(|e| CliError::Other(e.to_string()))?;
    let answer = api.call("PUT", &access_path(id), Some(&body))?;
    Ok(if json {
        Output::Json(answer.clone())
    } else {
        Output::Text(peripherals_access_text(id, &answer))
    })
}

/// The path of one peripheral.
fn peripheral_path(id: &str) -> String {
    format!("{}/{}", canvas_proto::api::routes::PERIPHERALS, id)
}

/// The path of one peripheral's access.
fn access_path(id: &str) -> String {
    format!("{}/access", peripheral_path(id))
}

/// The text `peripherals list` prints.
pub fn peripherals_list_text(answer: &serde_json::Value) -> String {
    let peripherals = answer
        .get("peripherals")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if peripherals.is_empty() {
        return "no peripherals\n".to_string();
    }
    let mut text = String::new();
    text.push_str(&format!("{:<26} {:<15} {:<20} {:<10} ID\n", "NAME", "KIND", "ADAPTER", "OPEN"));
    for peripheral in &peripherals {
        let name = peripheral.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = peripheral.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let adapter = peripheral.get("adapter").and_then(|v| v.as_str()).unwrap_or("?");
        let id = peripheral.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let open = if peripheral.get("connected").and_then(|v| v.as_bool()).unwrap_or(false) {
            "open"
        } else {
            "closed"
        };
        text.push_str(&format!("{name:<26} {kind:<15} {adapter:<20} {open:<10} {id}\n"));
    }
    text
}

/// The text `peripherals show <id>` prints.
pub fn peripherals_show_text(answer: &serde_json::Value) -> String {
    let mut text = String::new();
    let info = answer.get("info").cloned().unwrap_or(serde_json::Value::Null);
    let mut row = |key: &str, label: &str| {
        let value = info.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let value = if value.is_empty() { "-" } else { value };
        text.push_str(&format!("{label:<12} {value}\n"));
    };
    row("id", "id");
    row("name", "name");
    row("vendor", "vendor");
    row("kind", "kind");
    row("adapter", "adapter");
    row("serial", "serial");
    row("location", "location");
    let connected = info.get("connected").and_then(|v| v.as_bool()).unwrap_or(false);
    text.push_str(&format!("{:<12} {}\n", "open", if connected { "yes" } else { "no" }));
    let access = answer.get("access").cloned().unwrap_or(serde_json::Value::Null);
    let reads = access.get("reads").and_then(|v| v.as_bool()).unwrap_or(false);
    let writes = access.get("writes").and_then(|v| v.as_bool()).unwrap_or(false);
    text.push_str(&format!(
        "{:<12} reads {}, writes {}\n",
        "access",
        if reads { "on" } else { "off" },
        if writes { "on" } else { "off" }
    ));
    match answer.get("problem").and_then(|v| v.as_str()) {
        Some(problem) => text.push_str(&format!("{:<12} {problem}\n", "problem")),
        None => text.push_str(&format!("{:<12} none\n", "problem")),
    }
    let values = answer.get("values").cloned().unwrap_or(serde_json::Value::Null);
    let sections = answer
        .get("schema")
        .and_then(|s| s.get("sections"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for section in &sections {
        let title = section.get("title").and_then(|v| v.as_str()).unwrap_or("");
        text.push_str(&format!("\n{title}\n"));
        for property in section
            .get("properties")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let key = property.get("key").and_then(|v| v.as_str()).unwrap_or("?");
            let kind = property.get("kind").cloned().unwrap_or(serde_json::Value::Null);
            let value = values
                .get(key)
                .map(prop_value_text)
                .unwrap_or_else(|| "-".to_string());
            text.push_str(&format!(
                "  {key:<24} {:<22} {value}\n",
                prop_kind_summary(&kind)
            ));
        }
    }
    text
}

/// The text `peripherals access <id>` prints.
pub fn peripherals_access_text(id: &str, answer: &serde_json::Value) -> String {
    let reads = answer.get("reads").and_then(|v| v.as_bool()).unwrap_or(false);
    let writes = answer.get("writes").and_then(|v| v.as_bool()).unwrap_or(false);
    format!(
        "{id}: reads {}, writes {}\n",
        if reads { "on" } else { "off" },
        if writes { "on" } else { "off" }
    )
}

/// The text `fans status` prints.
///
/// The MODE column says what the chip is doing. A header that is not on this board
/// (`available: false`, issue #153) says that and nothing else: the asked mode is left
/// out, because nothing was asked of a header that is not wired up.
pub fn fans_status_text(answer: &serde_json::Value) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "chip          {}\n",
        answer
            .get("chip")
            .and_then(|v| v.as_str())
            .unwrap_or("not detected")
    ));
    text.push_str(&format!(
        "profile       {}\n",
        answer
            .get("active_profile")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    ));
    text.push_str(&format!(
        "floor         {}%\n",
        answer
            .get("floor_percent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    ));
    let headers = answer
        .get("headers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if headers.is_empty() {
        text.push_str("no fan headers\n");
    } else {
        text.push_str(&format!(
            "{:<14} {:<10} {:<8} MODE\n",
            "HEADER", "RPM", "DUTY"
        ));
        for header in &headers {
            let id = header.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let rpm = header
                .get("rpm")
                .and_then(|v| v.as_u64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into());
            let duty = header
                .get("pwm_percent")
                .and_then(|v| v.as_u64())
                .map(|v| format!("{v}%"))
                .unwrap_or_else(|| "-".into());
            text.push_str(&format!(
                "{:<14} {:<10} {:<8} {}\n",
                id,
                rpm,
                duty,
                header_mode_text(header)
            ));
        }
    }
    let temps = answer
        .get("temps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for temp in &temps {
        let label = temp.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let celsius = temp
            .get("celsius")
            .and_then(|v| v.as_f64())
            .map(|v| format!("{v:.1} C"))
            .unwrap_or_else(|| "-".into());
        text.push_str(&format!("{:<14} {celsius}\n", label));
    }
    text
}

/// The MODE column of one fan header.
///
/// A header that is not on this board is read first and says so: it is never written,
/// so nothing was asked of it and the BIOS is not refusing anything. Every other header
/// says what the chip is doing; what was asked is shown only when it differs.
fn header_mode_text(header: &serde_json::Value) -> String {
    let available = header
        .get("available")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !available {
        return "not on this board".to_string();
    }
    let asked = describe_mode(header.get("mode"));
    let on_bios = header
        .get("on_bios")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reason = header.get("override_reason").and_then(|v| v.as_str());
    match (on_bios, asked.as_str(), reason) {
        (true, "BIOS", _) => "BIOS".to_string(),
        (true, _, Some(reason)) => format!("BIOS (asked {asked}; {reason})"),
        (true, _, None) => format!("BIOS (asked {asked})"),
        (false, _, Some(reason)) => format!("{asked} ({reason})"),
        (false, _, None) => asked,
    }
}

/// A one-line description of a fan mode.
fn describe_mode(mode: Option<&serde_json::Value>) -> String {
    let Some(mode) = mode else {
        return "?".into();
    };
    match mode.get("mode").and_then(|v| v.as_str()) {
        Some("bios") => "BIOS".into(),
        Some("fixed") => {
            let percent = mode.get("percent").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("fixed {percent}%")
        }
        Some("curve") => {
            let points = mode
                .get("points")
                .and_then(|v| v.as_array())
                .map(|p| p.len())
                .unwrap_or(0);
            format!("curve ({points} points)")
        }
        _ => "?".into(),
    }
}

/// A property value, as the CLI shows it.
fn prop_value_text(value: &serde_json::Value) -> String {
    match serde_json::from_value::<canvas_proto::props::PropValue>(value.clone()) {
        Ok(canvas_proto::props::PropValue::Color(color)) => color.to_hex(),
        Ok(canvas_proto::props::PropValue::Number(number)) => format!("{number}"),
        Ok(canvas_proto::props::PropValue::Bool(on)) => {
            if on { "on" } else { "off" }.to_string()
        }
        Ok(canvas_proto::props::PropValue::Choice(choice)) => choice,
        Ok(canvas_proto::props::PropValue::Hue(hue)) => format!("{hue}"),
        Ok(canvas_proto::props::PropValue::Angle(angle)) => format!("{angle}"),
        Err(_) => value.to_string(),
    }
}

/// A property's kind, as one line: its type and what it takes.
fn prop_kind_summary(kind: &serde_json::Value) -> String {
    match kind.get("type").and_then(|v| v.as_str()) {
        Some("number") => {
            let min = kind.get("min").and_then(|v| v.as_f64()).unwrap_or_default();
            let max = kind.get("max").and_then(|v| v.as_f64()).unwrap_or_default();
            format!("number {min}..{max}")
        }
        Some("choice") => {
            let options: Vec<&str> = kind
                .get("options")
                .and_then(|v| v.as_array())
                .map(|list| list.iter().filter_map(|o| o.as_str()).collect())
                .unwrap_or_default();
            format!("choice {}", options.join("|"))
        }
        Some("bool") => "bool".to_string(),
        Some("color") => "color".to_string(),
        Some("hue") => "hue".to_string(),
        Some("angle") => "angle".to_string(),
        _ => "?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::props::PropKind;
    use std::path::PathBuf;

    #[test]
    fn the_update_request_holds_an_absolute_path_and_the_trimmed_signature() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-update-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let installer = dir.join("setup.exe");
        let sig = dir.join("setup.exe.sig");
        std::fs::write(&installer, b"x").unwrap();
        std::fs::write(&sig, "c2ln\r\n").unwrap();
        let args = UpdateArgs {
            file: installer.to_string_lossy().into_owned(),
            sig: sig.to_string_lossy().into_owned(),
        };
        let request = args.request().unwrap();
        assert!(std::path::Path::new(&request.installer).is_absolute());
        assert_eq!(request.signature, "c2ln");

        let missing = UpdateArgs { file: installer.to_string_lossy().into_owned(), sig: "no-such.sig".into() };
        assert!(missing.request().is_err());
        let no_file = UpdateArgs { file: dir.join("none.exe").to_string_lossy().into_owned(), sig: args.sig.clone() };
        assert!(no_file.request().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_update_command_parses() {
        let cli = Cli::try_parse_from(["canvas-rgb", "update", "--file", "a.exe", "--sig", "a.sig"]).unwrap();
        assert!(matches!(cli.command, Command::Update(_)));
        assert!(Cli::try_parse_from(["canvas-rgb", "update", "--file", "a.exe"]).is_err());
    }

    #[test]
    fn colors_are_parsed() {
        let kind = PropKind::Color { default: canvas_proto::Rgb8::BLACK };
        let value = parse_prop_value(&kind, "#ff0000").unwrap();
        assert_eq!(value, canvas_proto::props::PropValue::Color(canvas_proto::Rgb8::new(255, 0, 0)));
        assert!(parse_prop_value(&kind, "ff0000").is_err());
    }

    #[test]
    fn numbers_are_range_checked() {
        let kind = PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 10.0 };
        assert!(parse_prop_value(&kind, "50").is_ok());
        assert!(parse_prop_value(&kind, "500").is_err());
        assert!(parse_prop_value(&kind, "abc").is_err());
    }

    #[test]
    fn booleans_and_choices_are_parsed() {
        let bool_kind = PropKind::Bool { default: false };
        assert_eq!(
            parse_prop_value(&bool_kind, "true").unwrap(),
            canvas_proto::props::PropValue::Bool(true)
        );
        let choice_kind = PropKind::Choice { options: vec!["a".into(), "b".into()], default: "a".into() };
        assert_eq!(
            parse_prop_value(&choice_kind, "B").unwrap(),
            canvas_proto::props::PropValue::Choice("b".into())
        );
        assert!(parse_prop_value(&choice_kind, "c").is_err());
    }

    #[test]
    fn hues_are_range_checked() {
        let kind = PropKind::Hue { default: 0.0 };
        assert!(parse_prop_value(&kind, "180").is_ok());
        assert!(parse_prop_value(&kind, "400").is_err());
    }

    #[test]
    fn angles_are_parsed() {
        let kind = PropKind::Angle { default: 90.0 };
        assert_eq!(
            parse_prop_value(&kind, "180").unwrap(),
            canvas_proto::props::PropValue::Angle(180.0)
        );
        assert!(parse_prop_value(&kind, "400").is_err());
        assert!(parse_prop_value(&kind, "abc").is_err());
    }

    #[test]
    fn activate_props_are_pairs() {
        let args = vec!["--color".to_string(), "#ff0000".to_string(), "--speed".to_string(), "20".to_string()];
        let props = parse_activate_props(&args).unwrap();
        assert_eq!(props.get("color").map(String::as_str), Some("#ff0000"));
        assert_eq!(props.get("speed").map(String::as_str), Some("20"));
    }

    #[test]
    fn a_missing_property_value_is_an_error() {
        let args = vec!["--color".to_string()];
        assert!(parse_activate_props(&args).is_err());
    }

    #[test]
    fn prop_pairs_are_split_on_the_first_equals() {
        let (key, value) = parse_prop_pair("color=#ff0000").unwrap();
        assert_eq!(key, "color");
        assert_eq!(value, "#ff0000");
        assert!(parse_prop_pair("nope").is_err());
    }

    /// #328: a request that cannot reach the daemon, or is never answered, used to wait on
    /// the operating system's own timeout (about 21 s for a dropped connect) or for ever.
    /// The bounds are what end that wait, so they are pinned here.
    #[test]
    fn the_client_bounds_every_request_so_a_wedged_daemon_cannot_hang_it() {
        let timeouts = bounded_agent().config().timeouts();
        assert_eq!(timeouts.connect, Some(CONNECT_TIMEOUT));
        assert_eq!(timeouts.global, Some(REQUEST_LIMIT));
    }

    /// A client for the transport tests: the port is the subject, so the ownership check in
    /// `Client::new` (which needs a listener named `canvas-rgbd.exe`) is not on the path.
    fn client_for(port: u16, agent: ureq::Agent) -> Client {
        Client {
            base: format!("http://127.0.0.1:{port}"),
            token: "0123456789abcdef".to_string(),
            agent,
        }
    }

    /// A port nothing listens on, for a refused connect.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the local address").port();
        drop(listener);
        port
    }

    /// Runs `answer` on the first connection to `listener`, and gives up if nothing connects
    /// within 2 s. A client whose whole budget covers the connect can give up before it
    /// connects, and a thread blocked in `accept` would then hold the test's `join` for ever.
    fn serve_once(
        listener: std::net::TcpListener,
        answer: impl FnOnce(std::net::TcpStream) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            listener.set_nonblocking(true).expect("the listener is non-blocking");
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(2) {
                if let Ok((stream, _)) = listener.accept() {
                    answer(stream);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    }

    /// #328: nothing answered, so nothing was sent, and the message says that -- never that
    /// the daemon timed out.
    #[test]
    fn a_refused_connect_says_the_daemon_is_not_running() {
        let port = closed_port();
        let error = client_for(port, bounded_agent())
            .request("GET", "/api/v1/status", None)
            .expect_err("nothing listens there");
        assert_eq!(
            error.to_string(),
            format!("the daemon is not running (nothing answers on http://127.0.0.1:{port})")
        );
        assert_eq!(error.exit_code(), 2);
    }

    /// #328: the OS spends about 21 s on a dropped connect before giving up. The CLI's own
    /// connect limit is what ends that wait, and its failure is the "not running" one.
    #[test]
    fn a_connect_timeout_says_the_daemon_is_not_running() {
        let error = transport_error(
            ureq::Error::Timeout(ureq::Timeout::Connect),
            "http://127.0.0.1:9430",
            Some(REQUEST_LIMIT),
        );
        assert!(!matches!(error, CliError::NoAnswer(_)), "{error}");
        assert!(matches!(error, CliError::NotRunning(_)), "{error}");
        assert_eq!(
            error.to_string(),
            "the daemon is not running (nothing answers on http://127.0.0.1:9430)"
        );
    }

    /// #328: a daemon that took the connection and did not answer is a different fault from
    /// one that is not running, and the message names the limit that ended the wait.
    #[test]
    fn a_silent_daemon_says_it_did_not_answer_within_the_limit() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the local address").port();
        // Take the connection and answer nothing at all, for longer than the client's budget.
        let server = serve_once(listener, |stream| {
            std::thread::sleep(Duration::from_secs(2));
            drop(stream);
        });
        let agent = bounded_agent_with(CONNECT_TIMEOUT, Duration::from_millis(300));
        let error = client_for(port, agent)
            .request("GET", "/api/v1/status", None)
            .expect_err("the daemon never answered");
        assert!(
            error.to_string().contains("did not answer within 300 ms"),
            "{error}"
        );
        assert!(matches!(error, CliError::NoAnswer(_)), "{error}");
        assert_eq!(error.exit_code(), 2);
        server.join().ok();
    }

    /// #328: a body that cannot be read is a failed command, not an empty answer. The limit
    /// makes a body cut short reachable, so it must not read as a 2xx with no content.
    #[test]
    fn a_body_that_cannot_be_read_fails_the_command() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the local address").port();
        let server = serve_once(listener, |mut stream| {
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            // Promise 100 bytes and send 5, then close.
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort");
        });
        let error = client_for(port, bounded_agent())
            .request("GET", "/api/v1/status", None)
            .expect_err("the body was cut short");
        assert_ne!(error.exit_code(), 0);
        assert!(
            !error.to_string().starts_with("the daemon is not running"),
            "the connection was made: {error}"
        );
        assert!(error.to_string().contains("io"), "the transport's own text: {error}");
        server.join().ok();
    }

    #[test]
    fn no_client_is_built_for_a_port_nobody_listens_on() {
        // A port that was just freed: nothing listens there, so the token (which is
        // never read) would go nowhere.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the local address").port();
        drop(listener);

        let error = match Client::new(Some(&format!("http://127.0.0.1:{port}"))) {
            Ok(_) => panic!("the client must not be built"),
            Err(e) => e,
        };
        assert!(matches!(error, CliError::NotTrusted(_)));
        assert_eq!(error.to_string(), "canvas-rgb daemon not running");
        assert_eq!(error.exit_code(), 2);
    }

    /// A temporary folder for one test, removed when the test ends.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, contents: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "canvas-rgb-check-{}-{name}",
                std::process::id()
            ));
            std::fs::write(&path, contents).expect("the file is written");
            Self(path)
        }

        fn path(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// An effect that draws a moving block.
    const MOVING: &str = r##"<head><title>Moving</title>
<meta property="speed" label="Speed" type="number" min="0" max="100" default="10" />
<meta property="tint" label="Tint" type="color" default="#ffcc00" />
</head><body><canvas id="c" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  var x = 0;
  function draw() {
    ctx.fillStyle = "#101020";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = tint;
    ctx.fillRect(x, 40, 30, 30);
    x = (x + 3) % 300;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

    /// The check works on the source itself, with no file involved (the editor's preview
    /// renders a draft the same way the CLI checks one).
    #[test]
    fn check_source_checks_a_draft_with_no_file() {
        let report = check_source(MOVING, None).expect("the source loads");
        assert_eq!(report.name, "Moving");
        assert!(report.moves, "the picture moves");
        assert!(!report.flat, "the picture is not one flat colour");
        assert!(report.errors.is_empty(), "{:?}", report.errors);
    }

    #[test]
    fn a_moving_effect_passes_the_check() {
        let file = TempFile::new("moving", MOVING);
        let report = check_effect(&file.path(), None).expect("the effect loads");
        assert_eq!(report.name, "Moving");
        assert_eq!(report.frames, 120);
        assert!(report.moves, "the picture moves");
        assert!(!report.flat, "the picture is not one flat colour");
        assert!(report.mean_frame_ms >= 0.0);
        assert!(report.max_frame_ms >= report.mean_frame_ms);
        // The properties are listed with their types and defaults.
        assert_eq!(report.properties.len(), 2);
        assert_eq!(report.properties[0].key, "speed");
        assert_eq!(report.properties[0].kind, "number");
        assert_eq!(report.properties[0].default, "10");
        assert_eq!(report.properties[1].key, "tint");
        assert_eq!(report.properties[1].kind, "color");
        assert_eq!(report.properties[1].default, "#ffcc00");
    }

    #[test]
    fn a_file_that_does_not_parse_gives_the_parser_s_message() {
        let file = TempFile::new("broken", "<head><title>No script</title></head>");
        let error = check_effect(&file.path(), None).expect_err("the file must not load");
        assert!(error.contains("parse:"), "the parser's message is kept: {error}");
    }

    /// SHOULD-FIX 1 of the #75 review: `effects check` reported a load that ran out of time in
    /// the runtime's raw `timeout: ...` words; it now uses the same plain sentence the editor
    /// and the daemon's failed list use.
    #[test]
    fn a_timed_out_check_is_reported_in_plain_words() {
        let runaway = r##"<head><title>Loop</title></head>
<body><canvas id="c" width="16" height="16"></canvas></body>
<script>
  while (true) {}
</script>"##;
        let error = check_source(runaway, None).expect_err("a runaway top-level script times out");
        assert!(!error.starts_with("timeout: "), "the runtime's prefix leaked: {error}");
        assert!(error.contains("took too long to load"), "a plain sentence is shown: {error}");
        assert!(error.contains("stopped"), "the sentence says the script was stopped: {error}");
    }

    /// SHOULD-FIX 4 of the #412 review: `effects check` reads a file given by path like an import
    /// does, capped and with the same sentence, so a check never loads more than an import takes.
    #[test]
    fn a_check_file_over_the_cap_is_refused_with_the_one_cap_sentence() {
        let big = TempFile::new(
            "check-big",
            &"x".repeat(canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES + 1),
        );
        let said = format!(
            "is over {} bytes, more than an effect file may hold",
            canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES
        );
        let error = check_effect(&big.path(), None).expect_err("a file over the cap is refused");
        assert!(error.contains(&said), "read_capped's sentence is kept: {error}");
        let error = check_effect_with(&big.path(), None, &["speed=1".to_string()])
            .expect_err("a file over the cap is refused with --prop too");
        assert!(error.contains(&said), "{error}");
        let error = check_tile(&big.path(), None).expect_err("the tile is not drawn from it");
        assert!(error.contains(&said), "{error}");
        // One byte less is read: the cap itself is allowed. Whatever the check makes of the file,
        // it is not the cap's refusal.
        let fits = TempFile::new(
            "check-fits",
            &format!(
                "<head><title>Fits</title></head><body></body><script>void 0;</script>{}",
                " ".repeat(canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES - 100)
            ),
        );
        if let Err(error) = check_effect(&fits.path(), None) {
            assert!(!error.contains("is over"), "a file at the cap is read: {error}");
        }
    }

    #[test]
    fn an_effect_that_draws_nothing_is_flat() {
        let file = TempFile::new(
            "flat",
            "<head><title>Flat</title></head><body><canvas id=\"c\"></canvas></body><script>\
             var c = document.getElementById('c'); var ctx = c.getContext('2d'); \
             function draw() { window.requestAnimationFrame(draw); } \
             window.requestAnimationFrame(draw);</script>",
        );
        let report = check_effect(&file.path(), None).expect("the effect loads");
        assert!(report.flat, "the picture is one flat colour");
        assert!(!report.moves, "the picture never changes");
    }

    #[test]
    fn a_hanging_effect_is_reported_and_the_check_stops_there() {
        // The frame callback never ends. The runtime stops it at its budget, the error
        // is reported and the check stops instead of paying the budget on every frame.
        let file = TempFile::new(
            "hanging",
            "<head><title>Hanging</title></head><body><canvas id=\"c\"></canvas></body><script>\
             var c = document.getElementById('c'); var ctx = c.getContext('2d'); \
             function draw() { while (true) {} } \
             window.requestAnimationFrame(draw);</script>",
        );
        let started = std::time::Instant::now();
        let report = check_effect(&file.path(), None).expect("the effect loads");
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_millis() < 5_000,
            "the check took {elapsed:?}, it must stop at the first failure"
        );
        assert_eq!(report.frames, 1, "the check stopped after the first frame");
        assert!(!report.errors.is_empty(), "the error is reported");
        assert!(
            report.errors[0].contains("ran longer than"),
            "the message names the budget: {:?}",
            report.errors
        );
    }

    /// A bright picture that comes back to where it began every 60 frames, with a tile
    /// recipe that makes the 24-frame tile cover exactly one turn (1 s).
    const BRIGHT_LOOP: &str = r##"<head>
  <title>Bright loop</title>
  <meta thumbnail warm_up="0" loop="1" />
</head>
<body><canvas id="stage" width="320" height="200"></canvas></body>
<script>
  var ctx = document.getElementById("stage").getContext("2d");
  var t = 0;
  function draw() {
    t++;
    var v = Math.round(170 + 60 * Math.sin(t * Math.PI * 2 / 60));
    ctx.fillStyle = "rgb(" + v + "," + (255 - v) + ",200)";
    ctx.fillRect(0, 0, 320, 200);
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

    /// A near-black picture that pulses a little.
    const DARK_PULSE: &str = r##"<head>
  <title>Dark pulse</title>
</head>
<body><canvas id="stage" width="320" height="200"></canvas></body>
<script>
  var ctx = document.getElementById("stage").getContext("2d");
  var t = 0;
  function draw() {
    t++;
    var v = 4 + (t % 6);
    ctx.fillStyle = "rgb(" + v + "," + v + "," + v + ")";
    ctx.fillRect(0, 0, 320, 200);
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

    #[test]
    fn a_bright_looping_tile_passes_the_bar_and_writes_its_preview() {
        let file = TempFile::new("bright-tile", BRIGHT_LOOP);
        let dir = std::env::temp_dir().join(format!("canvas-rgb-tile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tile = check_tile(&file.path(), Some(&dir.to_string_lossy())).expect("the tile renders");
        assert!(tile.problems.is_empty(), "{:?}", tile.problems);
        assert_eq!(tile.score.rows_lit, crate::thumbnail::HEIGHT as usize);

        let gif = std::fs::read(tile.gif.as_ref().expect("a GIF path")).expect("the GIF is there");
        assert_eq!(&gif[..6], b"GIF89a");
        let sheet = std::fs::read(tile.sheet.as_ref().expect("a sheet path")).expect("the sheet is there");
        assert_eq!(&sheet[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_thumbnail_tag_is_reported_and_the_effect_is_still_checked() {
        let bad = BRIGHT_LOOP.replace(r#"loop="1""#, r#"loop="1s""#);
        let file = TempFile::new("bad-tile-tag", &bad);
        // The effect itself still loads and runs.
        let report = check_effect(&file.path(), None).expect("the effect loads");
        assert!(report.moves);
        // The tile check names the ignored tag, and the tile is rendered with the defaults.
        let tile = check_tile(&file.path(), None).expect("the tile renders");
        assert!(tile.problems.iter().any(|p| p.contains("<meta thumbnail> tag was ignored")), "{:?}", tile.problems);
        assert!(tile.recipe.is_none());
        // The tile is a picture and not a blank frame.
        assert!(
            tile.score.lit_avg > 0.5,
            "the tile is blank: only {:.0} % of it is lit",
            tile.score.lit_avg * 100.0
        );
    }

    #[test]
    fn a_dark_tile_fails_the_bar_and_writes_nothing_unasked() {
        let file = TempFile::new("dark-tile", DARK_PULSE);
        let tile = check_tile(&file.path(), None).expect("the tile renders");
        assert!(!tile.problems.is_empty(), "a near-black tile must fail the bar");
        assert!(tile.gif.is_none() && tile.sheet.is_none());
    }

    #[test]
    fn a_png_can_be_written() {
        let file = TempFile::new("png", MOVING);
        let out = std::env::temp_dir().join(format!(
            "canvas-rgb-check-{}-frame.png",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out);
        let report = check_effect(&file.path(), Some(&out.to_string_lossy())).expect("the effect loads");
        assert_eq!(report.png.as_deref(), Some(out.to_string_lossy().as_ref()));
        let bytes = std::fs::read(&out).expect("the PNG is there");
        // A PNG starts with the eight-byte signature.
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        let _ = std::fs::remove_file(&out);
    }

    /// An API that answers every request with one fixed value.
    struct FakeApi(serde_json::Value);

    impl Api for FakeApi {
        fn call(
            &self,
            method: &str,
            path: &str,
            _body: Option<&str>,
        ) -> Result<serde_json::Value, CliError> {
            assert_eq!(method, "GET");
            assert_eq!(path, canvas_proto::api::routes::FANS);
            Ok(self.0.clone())
        }
    }

    /// An API that answers the scenes list and records the request that set a scene.
    struct ScenesApi {
        answer: serde_json::Value,
        asked: std::sync::Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl Api for ScenesApi {
        fn call(
            &self,
            method: &str,
            path: &str,
            body: Option<&str>,
        ) -> Result<serde_json::Value, CliError> {
            self.asked.lock().unwrap().push((method.to_string(), path.to_string(), body.map(str::to_string)));
            if path == canvas_proto::api::routes::SCENES {
                return Ok(self.answer.clone());
            }
            Ok(serde_json::json!({ "ok": true }))
        }
    }

    /// The scenes answer the CLI reads.
    fn a_scenes_answer() -> serde_json::Value {
        serde_json::json!({
            "active": "gaming",
            "scenes": [
                { "id": "gaming", "name": "Gaming" },
                { "id": "movie", "name": "Movie" },
            ],
            "group_states": []
        })
    }

    /// The scene is set by its name, whatever the case: the owner types the name they see.
    #[test]
    fn scenes_set_matches_names_without_case() {
        let api = ScenesApi {
            answer: a_scenes_answer(),
            asked: std::sync::Mutex::new(Vec::new()),
        };
        for name in ["gaming", "GAMING", "GaMiNg"] {
            let output = super::scenes_set(&api, name, false).expect("the scene is set");
            match output {
                super::Output::Text(text) => assert!(text.contains("Gaming"), "{text}"),
                other => panic!("{other:?}"),
            }
            let asked = api.asked.lock().unwrap().clone();
            let last = asked.last().expect("the set was sent");
            assert_eq!(last.0, "PUT");
            assert_eq!(last.1, canvas_proto::api::routes::SCENE_ACTIVE);
            let body: serde_json::Value = serde_json::from_str(last.2.as_deref().expect("a body")).unwrap();
            assert_eq!(body["id"], "gaming", "the id of the scene whose name matched");
        }
    }

    /// A name that matches nothing prints the list and fails: the owner sees the names.
    #[test]
    fn scenes_set_with_no_match_prints_the_list_and_fails() {
        let api = ScenesApi {
            answer: a_scenes_answer(),
            asked: std::sync::Mutex::new(Vec::new()),
        };
        let error = super::scenes_set(&api, "nope", false).expect_err("no scene has that name");
        let message = error.to_string();
        assert!(message.contains("nope"), "{message}");
        assert!(message.contains("Gaming"), "the list is in the message: {message}");
        assert!(message.contains("Movie"), "the list is in the message: {message}");
        let asked = api.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "only the list was asked for");
    }

    /// The list marks the active scene.
    #[test]
    fn scenes_list_marks_the_active_one() {
        let api = ScenesApi {
            answer: a_scenes_answer(),
            asked: std::sync::Mutex::new(Vec::new()),
        };
        let output = super::scenes_list(&api, false).expect("the list");
        match output {
            super::Output::Text(text) => {
                let lines: Vec<&str> = text.lines().collect();
                assert_eq!(lines.len(), 2, "{text}");
                assert!(lines[0].starts_with('*'), "the active scene is marked: {text}");
                assert!(lines[0].contains("gaming") && lines[0].contains("Gaming"), "{text}");
                assert!(lines[1].starts_with(' '), "the other scene is not: {text}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A name that names two scenes is refused, and the matches are listed: scene names are
    /// not unique, and picking one is the owner's call.
    #[test]
    fn scenes_set_refuses_an_ambiguous_name_and_lists_the_matches() {
        let api = ScenesApi {
            answer: serde_json::json!({
                "active": "one",
                "scenes": [
                    { "id": "one", "name": "Gaming" },
                    { "id": "two", "name": "GAMING" },
                ],
                "group_states": []
            }),
            asked: std::sync::Mutex::new(Vec::new()),
        };
        let error = super::scenes_set(&api, "gaming", false).expect_err("the name is ambiguous");
        let message = error.to_string();
        assert!(message.contains("gaming"), "{message}");
        assert!(message.contains("two"), "the matches are listed: {message}");
        assert!(message.contains("one"), "the matches are listed: {message}");
        let asked = api.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "only the list was asked for");
    }

    /// The `--group` flag works before and after the properties: the properties are a trailing
    /// list, so the flag is taken out of it.
    #[test]
    fn the_group_flag_works_before_and_after_the_properties() {
        let args = |rest: &[&str]| -> Vec<String> { rest.iter().map(|s| s.to_string()).collect() };
        let (groups, props) = super::split_groups(&args(&["--color", "#f00", "--group", "Intake"]));
        assert_eq!(groups, vec!["Intake".to_string()]);
        assert_eq!(props, args(&["--color", "#f00"]));

        let (groups, props) = super::split_groups(&args(&["--group", "Intake", "--color", "#f00"]));
        assert_eq!(groups, vec!["Intake".to_string()]);
        assert_eq!(props, args(&["--color", "#f00"]));

        // Two groups, and no properties.
        let (groups, props) =
            super::split_groups(&args(&["--group", "Intake", "--group", "Desk"]));
        assert_eq!(groups, vec!["Intake".to_string(), "Desk".to_string()]);
        assert!(props.is_empty());

        // No flag: the properties are the properties.
        let (groups, props) = super::split_groups(&args(&["--color", "#f00"]));
        assert!(groups.is_empty());
        assert_eq!(props, args(&["--color", "#f00"]));
    }

    /// The `--group` flag works through clap, before and after the properties, in both forms.
    #[test]
    fn the_group_flag_parses_through_clap() {
        for args in [
            vec!["canvas-rgb", "effects", "activate", "solid", "--group", "Intake", "--color", "red"],
            vec!["canvas-rgb", "effects", "activate", "solid", "--color", "red", "--group", "Intake"],
            vec!["canvas-rgb", "effects", "activate", "solid", "--group=Intake", "--color", "red"],
            vec!["canvas-rgb", "effects", "activate", "solid", "--color", "red", "--group=Intake"],
            vec!["canvas-rgb", "effects", "activate", "--group", "Intake", "solid", "--color", "red"],
        ] {
            let cli = Cli::try_parse_from(&args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
            match cli.command {
                Command::Effects(EffectsCommand::Activate(activate)) => {
                    let (from_props, prop_args) = super::split_groups(&activate.props);
                    let mut groups = activate.groups.clone();
                    groups.extend(from_props);
                    assert_eq!(groups, vec!["Intake".to_string()], "{args:?}");
                    assert_eq!(
                        prop_args,
                        vec!["--color".to_string(), "red".to_string()],
                        "{args:?}"
                    );
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// An id names one scene, so it is accepted even when the names are ambiguous.
    #[test]
    fn scenes_set_accepts_an_id() {
        let api = ScenesApi {
            answer: serde_json::json!({
                "active": "one",
                "scenes": [
                    { "id": "one", "name": "Gaming" },
                    { "id": "two", "name": "GAMING" },
                ],
                "group_states": []
            }),
            asked: std::sync::Mutex::new(Vec::new()),
        };
        let output = super::scenes_set(&api, "two", false).expect("the id names one scene");
        match output {
            super::Output::Text(text) => assert!(text.contains("GAMING"), "{text}"),
            other => panic!("{other:?}"),
        }
        let asked = api.asked.lock().unwrap().clone();
        let last = asked.last().expect("the set was sent");
        let body: serde_json::Value = serde_json::from_str(last.2.as_deref().expect("a body")).unwrap();
        assert_eq!(body["id"], "two");
    }

    /// The commands parse: `scenes list`, `scenes set <name>`, and `--group` on the activate.
    #[test]
    fn the_scenes_commands_parse() {
        let cli = Cli::try_parse_from(["canvas-rgb", "scenes", "list"]).unwrap();
        assert!(matches!(cli.command, Command::Scenes(ScenesCommand::List)));
        let cli = Cli::try_parse_from(["canvas-rgb", "scenes", "set", "Gaming"]).unwrap();
        match cli.command {
            Command::Scenes(ScenesCommand::Set { name }) => assert_eq!(name, "Gaming"),
            other => panic!("{other:?}"),
        }
        let cli =
            Cli::try_parse_from(["canvas-rgb", "effects", "activate", "solid", "--group", "Intake"])
                .unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Activate(args)) => {
                assert_eq!(args.groups, vec!["Intake".to_string()]);
                assert_eq!(args.effect, "solid");
            }
            other => panic!("{other:?}"),
        }
    }

    /// An API that answers the scenes list with groups and revs and records every request. The
    /// reply to a write is the one given (an answer, or an error status with its body).
    struct OpsApi {
        reply: Result<serde_json::Value, (u16, serde_json::Value)>,
        asked: std::sync::Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl OpsApi {
        fn answering(reply: serde_json::Value) -> Self {
            Self { reply: Ok(reply), asked: std::sync::Mutex::new(Vec::new()) }
        }

        fn failing(status: u16, body: serde_json::Value) -> Self {
            Self { reply: Err((status, body)), asked: std::sync::Mutex::new(Vec::new()) }
        }

        fn asked(&self) -> Vec<(String, String, Option<String>)> {
            self.asked.lock().unwrap().clone()
        }

        /// The one write the command sent, body parsed.
        fn write(&self) -> (String, String, serde_json::Value) {
            let asked = self.asked();
            assert_eq!(asked.len(), 2, "one read, one write: {asked:?}");
            let (method, path, body) = asked[1].clone();
            (method, path, serde_json::from_str(body.as_deref().expect("a body")).unwrap())
        }
    }

    impl Api for OpsApi {
        fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, CliError> {
            self.asked.lock().unwrap().push((method.to_string(), path.to_string(), body.map(str::to_string)));
            if method == "GET" && path == canvas_proto::api::routes::SCENES {
                return Ok(a_scenes_list_with_groups());
            }
            match &self.reply {
                Ok(value) => Ok(value.clone()),
                Err((status, body)) => Err(CliError::Api {
                    status: *status,
                    message: body.get("message").and_then(|m| m.as_str()).unwrap_or_default().to_string(),
                    body: Some(body.clone()),
                }),
            }
        }
    }

    fn a_scenes_list_with_groups() -> serde_json::Value {
        serde_json::json!({
            "active": "gaming",
            "scenes": [
                { "id": "gaming", "name": "Gaming", "rev": 4, "groups": [
                    { "id": "desk", "name": "Desk" }, { "id": "wall", "name": "Wall" } ] },
                { "id": "movie", "name": "Movie", "rev": 7, "groups": [ { "id": "lamp", "name": "Lamp" } ] },
                { "id": "sleep", "name": "Sleep", "rev": 2, "groups": [] },
            ],
            "group_states": []
        })
    }

    fn scene(id: &str, rev: u64) -> canvas_proto::scene::Scene {
        canvas_proto::scene::Scene { id: id.into(), name: id.into(), rev, groups: Vec::new() }
    }

    /// What the daemon answers to a move that takes 2 devices from Everything and empties it.
    fn a_moved_answer(saved: bool) -> serde_json::Value {
        serde_json::to_value(canvas_proto::api::GroupMoved {
            group_id: "desk".into(),
            saved,
            scenes: vec![scene("gaming", 5), scene("movie", 8)],
            before: vec![scene("gaming", 4), scene("movie", 7)],
            taken: vec![canvas_proto::api::GroupTaken {
                group_id: "all".into(),
                group_name: "Everything".into(),
                lights: vec!["a".into(), "b".into()],
            }],
            emptied: vec!["Everything".into()],
        })
        .unwrap()
    }

    fn move_args(group: &str, from: &str, to: &str, copy: bool, dry_run: bool) -> GroupMoveArgs {
        GroupMoveArgs { group: group.into(), from: from.into(), to: to.into(), copy, dry_run }
    }

    fn text_of(output: Output) -> String {
        match output {
            Output::Text(text) => text,
            other => panic!("{other:?}"),
        }
    }

    /// The command names are the spine's: the clap tree has a command at each spine path.
    #[test]
    fn the_new_commands_are_at_the_spines_names() {
        use clap::CommandFactory;
        let root = Cli::command();
        for path in [
            canvas_proto::cli::SCENES_ORDER,
            canvas_proto::cli::SCENES_GROUP_MOVE,
            canvas_proto::cli::LIGHTS_LIST,
            canvas_proto::cli::SCENES_CREATE,
            canvas_proto::cli::SCENES_EDIT,
            canvas_proto::cli::SCENES_DELETE,
        ] {
            let mut at = &root;
            for step in path {
                at = at.find_subcommand(step).unwrap_or_else(|| panic!("no command {step} on the path {path:?}"));
            }
        }
        let cli = Cli::try_parse_from(["canvas-rgb", "scenes", "order", "Sleep", "gaming", "--dry-run"]).unwrap();
        match cli.command {
            Command::Scenes(ScenesCommand::Order(args)) => {
                assert_eq!(args.scenes, ["Sleep", "gaming"]);
                assert!(args.dry_run);
            }
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from([
            "canvas-rgb", "scenes", "group", "move", "Desk", "--from", "gaming", "--to", "movie", "--copy",
        ])
        .unwrap();
        match cli.command {
            Command::Scenes(ScenesCommand::Group(ScenesGroupCommand::Move(args))) => {
                assert_eq!((args.group.as_str(), args.from.as_str(), args.to.as_str()), ("Desk", "gaming", "movie"));
                assert!(args.copy && !args.dry_run);
            }
            other => panic!("{other:?}"),
        }
        assert!(Cli::try_parse_from(["canvas-rgb", "scenes", "order"]).is_err(), "an order names scenes");

        let cli = Cli::try_parse_from(["canvas-rgb", "lights", "list"]).unwrap();
        assert!(matches!(cli.command, Command::Lights(LightsCommand::List)));
        let cli = Cli::try_parse_from([
            "canvas-rgb", "scenes", "create", "Night", "--from", "groups.json", "--run", "agent-1", "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Scenes(ScenesCommand::Create(args)) => assert_eq!(
                (args.name.as_str(), args.from.as_deref(), args.run.as_deref(), args.dry_run),
                ("Night", Some("groups.json"), Some("agent-1"), true)
            ),
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from([
            "canvas-rgb", "scenes", "edit", "movie", "--from", "groups.json", "--run", "agent-1",
        ])
        .unwrap();
        match cli.command {
            Command::Scenes(ScenesCommand::Edit(args)) => assert_eq!(
                (args.scene.as_str(), args.from.as_str(), args.run.as_str(), args.dry_run),
                ("movie", "groups.json", "agent-1", false)
            ),
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from(["canvas-rgb", "scenes", "delete", "movie", "--run", "agent-1", "--force"])
            .unwrap();
        match cli.command {
            Command::Scenes(ScenesCommand::Delete(args)) => {
                assert_eq!((args.scene.as_str(), args.run.as_str()), ("movie", "agent-1"));
                assert!(args.force && !args.dry_run);
            }
            other => panic!("{other:?}"),
        }
        // --run is required on an edit and on a delete, and --from on an edit: a run's guard
        // cannot be left off by forgetting a flag.
        assert!(
            Cli::try_parse_from(["canvas-rgb", "scenes", "edit", "movie", "--from", "groups.json"]).is_err(),
            "an edit names the run"
        );
        assert!(
            Cli::try_parse_from(["canvas-rgb", "scenes", "delete", "movie"]).is_err(),
            "a delete names the run"
        );
        assert!(
            Cli::try_parse_from(["canvas-rgb", "scenes", "edit", "movie", "--run", "agent-1"]).is_err(),
            "an edit reads a groups file"
        );
    }

    #[test]
    fn scenes_order_sends_the_order_and_the_order_it_read() {
        let api = OpsApi::answering(a_scenes_list_with_groups());
        let names = ["sleep".to_string(), "MOVIE".to_string(), "Gaming".to_string()];
        let text = text_of(scenes_order(&api, &names, false, false).unwrap());
        assert_eq!(text, "the scenes are in this order: Sleep, Movie, Gaming\n");
        let (method, path, body) = api.write();
        assert_eq!((method.as_str(), path.as_str()), ("PUT", canvas_proto::api::routes::SCENES));
        assert_eq!(body["order"], serde_json::json!(["sleep", "movie", "gaming"]));
        assert_eq!(body["expected_order"], serde_json::json!(["gaming", "movie", "sleep"]));
        assert!(body.get("scenes").is_none(), "an order write names no scene");
    }

    #[test]
    fn scenes_order_dry_run_prints_the_order_and_sends_nothing() {
        let api = OpsApi::answering(serde_json::Value::Null);
        let names = ["sleep".to_string(), "movie".to_string(), "gaming".to_string()];
        let text = text_of(scenes_order(&api, &names, true, false).unwrap());
        assert_eq!(text, "would order: Sleep, Movie, Gaming\n");
        assert_eq!(api.asked().len(), 1, "only the read: {:?}", api.asked());
        match scenes_order(&api, &names, true, true).unwrap() {
            Output::Json(value) => assert_eq!(value, serde_json::json!({ "saved": false, "order": ["sleep", "movie", "gaming"] })),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scenes_order_with_a_scene_left_out_or_repeated_refuses_and_prints_the_list() {
        let api = OpsApi::answering(serde_json::Value::Null);
        for names in [vec!["movie", "gaming"], vec!["movie", "gaming", "gaming", "sleep"]] {
            let names: Vec<String> = names.into_iter().map(String::from).collect();
            let error = scenes_order(&api, &names, false, false).expect_err("not every scene once");
            let message = error.to_string();
            assert!(message.contains("name every scene once"), "{message}");
            assert!(message.contains("gaming") && message.contains("Movie") && message.contains("sleep"), "{message}");
            assert_eq!(error.exit_code(), 1);
        }
        let unknown = scenes_order(&api, &["nope".to_string()], false, false).expect_err("no such scene");
        assert!(unknown.to_string().contains("no scene is named nope"), "{unknown}");
        assert!(api.asked().iter().all(|(method, _, _)| method == "GET"), "nothing was written: {:?}", api.asked());
    }

    #[test]
    fn a_stale_order_is_a_409_with_the_current_order_in_the_error() {
        let body = serde_json::json!({ "error": "conflict", "message": "the scenes were put in another order", "order": ["movie", "gaming", "sleep"] });
        let api = OpsApi::failing(409, body.clone());
        let names = ["sleep".to_string(), "movie".to_string(), "gaming".to_string()];
        let error = scenes_order(&api, &names, false, false).expect_err("stale");
        assert_eq!(error.exit_code(), 1);
        let text = error.to_string();
        assert!(text.contains("409") && text.contains("the order is now: movie, gaming, sleep"), "{text}");
        assert_eq!(error.json_body(), Some(&body), "--json prints this body to stdout");
    }

    #[test]
    fn group_move_sends_the_revs_it_read_and_says_what_moved() {
        let api = OpsApi::answering(a_moved_answer(true));
        let text = text_of(scenes_group_move(&api, &move_args("Desk", "Gaming", "movie", false, false), false).unwrap());
        assert_eq!(text, "moved Desk from Gaming to Movie: 2 devices taken from Everything; 1 group removed\n");
        let (method, path, body) = api.write();
        assert_eq!(method, "POST");
        assert_eq!(path, "/api/v1/scenes/gaming/groups/desk/move");
        assert_eq!(
            body,
            serde_json::json!({ "to": "movie", "copy": false, "expected_rev_from": 4, "expected_rev_to": 7 })
        );
    }

    #[test]
    fn group_move_copy_and_dry_run_words() {
        let api = OpsApi::answering(a_moved_answer(true));
        let copied = text_of(scenes_group_move(&api, &move_args("desk", "gaming", "movie", true, false), false).unwrap());
        assert!(copied.starts_with("copied Desk from Gaming to Movie"), "{copied}");
        assert_eq!(api.write().2["copy"], true);

        let api = OpsApi::answering(a_moved_answer(false));
        let probe = text_of(scenes_group_move(&api, &move_args("desk", "gaming", "movie", false, true), false).unwrap());
        assert!(probe.starts_with("would move Desk from Gaming to Movie: "), "{probe}");
        assert_eq!(api.write().2["dry_run"], true, "the daemon is asked to save nothing");

        let api = OpsApi::answering(a_moved_answer(false));
        let probe = text_of(scenes_group_move(&api, &move_args("desk", "gaming", "movie", true, true), false).unwrap());
        assert!(probe.starts_with("would copy Desk"), "{probe}");
    }

    #[test]
    fn group_move_with_nothing_taken_is_one_short_line() {
        let mut answer = a_moved_answer(true);
        answer["taken"] = serde_json::json!([]);
        answer["emptied"] = serde_json::json!([]);
        let api = OpsApi::answering(answer);
        let text = text_of(scenes_group_move(&api, &move_args("Wall", "gaming", "sleep", false, false), false).unwrap());
        assert_eq!(text, "moved Wall from Gaming to Sleep\n");
        let one = {
            let mut answer = a_moved_answer(true);
            answer["taken"][0]["lights"] = serde_json::json!(["a"]);
            OpsApi::answering(answer)
        };
        let text = text_of(scenes_group_move(&one, &move_args("Wall", "gaming", "sleep", false, false), false).unwrap());
        assert!(text.contains("1 device taken from Everything"), "singular: {text}");
    }

    #[test]
    fn group_move_json_is_the_daemons_answer() {
        let api = OpsApi::answering(a_moved_answer(true));
        match scenes_group_move(&api, &move_args("desk", "gaming", "movie", false, false), true).unwrap() {
            Output::Json(value) => assert_eq!(value, a_moved_answer(true)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn group_move_refuses_an_unknown_scene_group_or_an_ambiguous_one() {
        let api = OpsApi::answering(serde_json::Value::Null);
        let unknown_scene = scenes_group_move(&api, &move_args("desk", "gaming", "nowhere", false, false), false).unwrap_err();
        assert!(unknown_scene.to_string().contains("no scene is named nowhere"), "{unknown_scene}");
        let unknown_group = scenes_group_move(&api, &move_args("couch", "gaming", "movie", false, false), false).unwrap_err();
        let message = unknown_group.to_string();
        assert!(message.contains("Gaming has no group named couch") && message.contains("Desk, Wall"), "{message}");
        assert!(api.asked().iter().all(|(method, _, _)| method == "GET"), "nothing was sent: {:?}", api.asked());

        let twins = serde_json::json!({ "active": "a", "scenes": [
            { "id": "a", "name": "A", "rev": 1, "groups": [ { "id": "x", "name": "Same" }, { "id": "y", "name": "same" } ] },
            { "id": "b", "name": "B", "rev": 1, "groups": [] } ] });
        struct Twins(serde_json::Value);
        impl Api for Twins {
            fn call(&self, _: &str, _: &str, _: Option<&str>) -> Result<serde_json::Value, CliError> {
                Ok(self.0.clone())
            }
        }
        let error = scenes_group_move(&Twins(twins), &move_args("same", "a", "b", false, false), false).unwrap_err();
        assert!(error.to_string().contains("names 2 groups in A: x (Same), y (same)"), "{error}");
    }

    #[test]
    fn group_move_422_exits_1_with_the_daemons_sentence() {
        let body = serde_json::json!({ "error": "validation", "message": "A group goes to another scene." });
        let api = OpsApi::failing(422, body);
        let error = scenes_group_move(&api, &move_args("desk", "gaming", "gaming", false, false), false).unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert_eq!(error.to_string(), "the API returned 422: A group goes to another scene.");
    }

    #[test]
    fn a_409_keeps_the_body_and_prints_the_current_rev() {
        let body = serde_json::json!({
            "error": "conflict", "which": "to", "message": "this scene was changed in another window",
            "scene": { "id": "movie", "name": "Movie", "rev": 9, "groups": [] }
        });
        let api = OpsApi::failing(409, body.clone());
        let error = scenes_group_move(&api, &move_args("desk", "gaming", "movie", false, false), false).unwrap_err();
        assert_eq!(error.exit_code(), 1);
        let text = error.to_string();
        assert!(text.contains("409") && text.contains("scene movie is at rev 9 now"), "{text}");
        assert_eq!(error.json_body(), Some(&body));
        // An error without a body (the daemon is down, say) has none to print.
        assert_eq!(CliError::Other("x".into()).json_body(), None);
        assert_eq!(CliError::Api { status: 500, message: "boom".into(), body: None }.to_string(), "the API returned 500: boom");
    }

    /// A curve of four points, as the API sends a header's mode.
    fn a_curve() -> serde_json::Value {
        serde_json::json!({
            "mode": "curve",
            "source": { "type": "cpu" },
            "points": [
                { "temp_c": 30.0, "percent": 30 },
                { "temp_c": 60.0, "percent": 50 },
                { "temp_c": 80.0, "percent": 70 },
                { "temp_c": 90.0, "percent": 100 }
            ]
        })
    }

    /// One fan header of a fans status answer.
    fn a_header(
        id: &str,
        mode: serde_json::Value,
        on_bios: bool,
        available: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "label": id,
            "rpm": 1200,
            "pwm_percent": 40,
            "mode": mode,
            "on_bios": on_bios,
            "override_reason": null,
            "available": available
        })
    }

    /// A fans status answer with the given headers.
    fn a_fans_answer(headers: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({
            "chip": "NCT6687D-R @ 0xA20",
            "active_profile": "quiet",
            "floor_percent": 20,
            "headers": headers,
            "temps": [{ "label": "CPU Tctl", "celsius": 42.0, "source": { "type": "cpu" } }],
            "profiles": ["quiet"]
        })
    }

    /// The MODE column of the row of one header.
    fn the_mode_cell(text: &str, id: &str) -> String {
        let row = text
            .lines()
            .find(|line| line.split_whitespace().next() == Some(id))
            .unwrap_or_else(|| panic!("no row for {id} in:\n{text}"));
        row.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// A header that is not on this board (issue #153): the chip has the channel, the
    /// board has no connector for it. The service never writes it, so nothing was asked
    /// of it and the BIOS is not refusing anything.
    #[test]
    fn a_header_that_is_not_on_this_board_says_so_and_asks_for_nothing() {
        let answer = a_fans_answer(vec![a_header("sys_fan5", a_curve(), true, false)]);
        let text = fans_status_text(&answer);
        assert_eq!(
            the_mode_cell(&text, "sys_fan5"),
            "sys_fan5 1200 40% not on this board"
        );
        assert!(
            !text.contains("asked"),
            "nothing was asked of a header that is not on this board:\n{text}"
        );
    }

    /// The JSON answer is the API's own, so it carries `available` as it is.
    #[test]
    fn the_json_answer_of_the_fans_is_the_api_s_own() {
        let answer = a_fans_answer(vec![a_header("sys_fan5", a_curve(), true, false)]);
        let output = fans_status(&FakeApi(answer.clone()), true).expect("the fans answer");
        match output {
            Output::Json(value) => {
                assert_eq!(value, answer);
                assert_eq!(value["headers"][0]["available"], serde_json::json!(false));
            }
            Output::Text(text) => panic!("the JSON answer is the API's own, not a table:\n{text}"),
        }
    }

    /// A header that is on the board reads as before: the BIOS on its own, the BIOS with
    /// what was asked, and the asked mode when the chip is doing it.
    #[test]
    fn the_rows_of_the_headers_that_are_on_the_board_do_not_change() {
        let bios = a_header("cpu_fan", serde_json::json!({ "mode": "bios" }), true, true);
        let asked = a_header("sys_fan1", a_curve(), true, true);
        let curve = a_header("sys_fan2", a_curve(), false, true);
        let text = fans_status_text(&a_fans_answer(vec![bios, asked, curve]));
        assert_eq!(the_mode_cell(&text, "cpu_fan"), "cpu_fan 1200 40% BIOS");
        assert_eq!(
            the_mode_cell(&text, "sys_fan1"),
            "sys_fan1 1200 40% BIOS (asked curve (4 points))"
        );
        assert_eq!(
            the_mode_cell(&text, "sys_fan2"),
            "sys_fan2 1200 40% curve (4 points)"
        );
    }

    /// A header that is on the board and was asked for a fixed speed reads as that speed,
    /// and a header that was handed back says why.
    #[test]
    fn a_fixed_mode_and_a_handback_read_as_before() {
        let fixed = a_header(
            "pump_fan",
            serde_json::json!({ "mode": "fixed", "percent": 55 }),
            false,
            true,
        );
        let mut handback = a_header("sys_fan3", a_curve(), true, true);
        handback["override_reason"] = serde_json::json!("the sensor was lost");
        let text = fans_status_text(&a_fans_answer(vec![fixed, handback]));
        assert_eq!(
            the_mode_cell(&text, "pump_fan"),
            "pump_fan 1200 40% fixed 55%"
        );
        assert_eq!(
            the_mode_cell(&text, "sys_fan3"),
            "sys_fan3 1200 40% BIOS (asked curve (4 points); the sensor was lost)"
        );
    }

    /// A header that was on the board before #128 and has no `available` field in the
    /// answer is on the board: the field's default is `true`.
    #[test]
    fn a_header_without_an_availability_field_is_on_the_board() {
        let mut header = a_header("cpu_fan", serde_json::json!({ "mode": "bios" }), true, true);
        header
            .as_object_mut()
            .expect("an object")
            .remove("available");
        let text = fans_status_text(&a_fans_answer(vec![header]));
        assert_eq!(the_mode_cell(&text, "cpu_fan"), "cpu_fan 1200 40% BIOS");
    }

    /// The table's header and the chip, the profile and the floor are as they were.
    #[test]
    fn the_fans_status_text_still_leads_with_the_chip_and_the_profile() {
        let answer = a_fans_answer(vec![a_header(
            "cpu_fan",
            serde_json::json!({ "mode": "bios" }),
            true,
            true,
        )]);
        let text = fans_status_text(&answer);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "chip          NCT6687D-R @ 0xA20");
        assert_eq!(lines[1], "profile       quiet");
        assert_eq!(lines[2], "floor         20%");
        assert_eq!(lines[3], "HEADER         RPM        DUTY     MODE");
        assert_eq!(lines[5], "CPU Tctl       42.0 C");
    }

    /// An answer with no headers says so, as it did.
    #[test]
    fn an_answer_with_no_headers_says_so() {
        let text = fans_status_text(&a_fans_answer(Vec::new()));
        assert!(text.contains("no fan headers"), "{text}");
    }

    // ------------------------------------------------ the scenes CLI and the run record (#377)

    /// A scratch `CANVAS_RGB_HOME` for one test, removed when the test ends. The variable is
    /// process-wide, so the tests that read a record take one at a time, and the value it had
    /// before is put back.
    struct TempHome {
        dir: PathBuf,
        previous: Option<std::ffi::OsString>,
        _one_at_a_time: std::sync::MutexGuard<'static, ()>,
    }

    impl TempHome {
        fn new(tag: &str) -> Self {
            static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let one_at_a_time = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!("canvas-rgb-runs-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("the scratch home is made");
            let previous = std::env::var_os(crate::paths::HOME_ENV);
            std::env::set_var(crate::paths::HOME_ENV, &dir);
            Self { dir, previous, _one_at_a_time: one_at_a_time }
        }

        fn path(&self) -> &std::path::Path {
            &self.dir
        }

        /// Writes a file under the home (making its folder) and answers its path.
        fn write(&self, name: &str, text: &str) -> String {
            let path = self.dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("the folder is made");
            }
            std::fs::write(&path, text).expect("the file is written");
            path.to_string_lossy().into_owned()
        }

        /// Every file under the home, so a dry run can be shown to have written nothing.
        fn files(&self) -> Vec<PathBuf> {
            fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
                let Ok(entries) = std::fs::read_dir(dir) else { return };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, out);
                    } else {
                        out.push(path);
                    }
                }
            }
            let mut out = Vec::new();
            walk(&self.dir, &mut out);
            out.sort();
            out
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(crate::paths::HOME_ENV, value),
                None => std::env::remove_var(crate::paths::HOME_ENV),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// An API that answers the scenes list with one fixed answer and a write with another,
    /// recording every request. Any other read (the lights, say) answers the write's reply.
    struct RunApi {
        list: serde_json::Value,
        reply: Result<serde_json::Value, (u16, serde_json::Value)>,
        /// What `GET STATUS` answers: by default this command line's own version.
        status: serde_json::Value,
        /// Runs when a request other than a read arrives, before it is answered.
        on_write: Option<Box<dyn Fn()>>,
        /// When set, a request other than a read fails with this error, after `on_write` ran:
        /// the daemon saw the request, and the caller learns nothing of what it did.
        write_error: Option<Box<dyn Fn() -> CliError>>,
        asked: std::sync::Mutex<Vec<(String, String, Option<String>)>>,
        status_reads: std::sync::atomic::AtomicUsize,
    }

    impl RunApi {
        fn new(list: serde_json::Value, reply: Result<serde_json::Value, (u16, serde_json::Value)>) -> Self {
            Self {
                list,
                reply,
                status: serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }),
                on_write: None,
                write_error: None,
                asked: std::sync::Mutex::new(Vec::new()),
                status_reads: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn answering(list: serde_json::Value, reply: serde_json::Value) -> Self {
            Self::new(list, Ok(reply))
        }

        fn failing(list: serde_json::Value, status: u16, body: serde_json::Value) -> Self {
            Self::new(list, Err((status, body)))
        }

        /// The daemon's answer to `GET STATUS`.
        fn with_status(mut self, status: serde_json::Value) -> Self {
            self.status = status;
            self
        }

        /// Something to do when a write arrives, before the daemon answers it.
        fn on_write(mut self, hook: impl Fn() + 'static) -> Self {
            self.on_write = Some(Box::new(hook));
            self
        }

        /// Makes every write fail with this error once the daemon has "seen" it.
        fn write_fails_with(mut self, make: impl Fn() -> CliError + 'static) -> Self {
            self.write_error = Some(Box::new(make));
            self
        }

        /// How many times the version was asked.
        fn status_reads(&self) -> usize {
            self.status_reads.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Every request except the version read, in order.
        fn asked(&self) -> Vec<(String, String, Option<String>)> {
            self.asked.lock().unwrap().clone()
        }

        /// The one write the command sent, after the one read it made.
        fn write(&self) -> (String, String, Option<String>) {
            let asked = self.asked();
            assert_eq!(asked.len(), 2, "one read, one write: {asked:?}");
            assert_eq!(asked[0].0, "GET", "the read comes first: {asked:?}");
            asked[1].clone()
        }
    }

    impl Api for RunApi {
        fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, CliError> {
            if method == "GET" && path == canvas_proto::api::routes::STATUS {
                self.status_reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Ok(self.status.clone());
            }
            self.asked
                .lock()
                .unwrap()
                .push((method.to_string(), path.to_string(), body.map(str::to_string)));
            if method == "GET" && path == canvas_proto::api::routes::SCENES {
                return Ok(self.list.clone());
            }
            if method != "GET" {
                if let Some(hook) = &self.on_write {
                    hook();
                }
                if let Some(make) = &self.write_error {
                    return Err(make());
                }
            }
            match &self.reply {
                Ok(value) => Ok(value.clone()),
                Err((status, body)) => Err(CliError::Api {
                    status: *status,
                    message: body.get("message").and_then(|m| m.as_str()).unwrap_or_default().to_string(),
                    body: Some(body.clone()),
                }),
            }
        }
    }

    /// The scenes the run tests read: Gaming is active (rev 4), Movie is at rev 7 with one group,
    /// Sleep has no groups. Each group is a whole `Group`, because an edit reads the scene whole.
    fn run_scenes() -> serde_json::Value {
        serde_json::json!({
            "active": "gaming",
            "scenes": [
                { "id": "gaming", "name": "Gaming", "rev": 4, "groups": [
                    { "id": "desk", "name": "Desk", "members": ["fake:a#z0"], "plays": { "type": "off" } } ] },
                { "id": "movie", "name": "Movie", "rev": 7, "groups": [
                    { "id": "lamp", "name": "Lamp", "members": ["fake:b#z0"], "plays": { "type": "off" } } ] },
                { "id": "sleep", "name": "Sleep", "rev": 2, "groups": [] },
            ],
            "group_states": []
        })
    }

    /// The scene the daemon answers to a create or an edit.
    fn a_saved_scene() -> serde_json::Value {
        serde_json::json!({
            "id": "night", "name": "Night", "rev": 1,
            "groups": [
                { "id": "fans", "name": "Fans", "members": ["fake:a#z0"], "plays": { "type": "off" } }
            ]
        })
    }

    /// Two lights, one connected and one not, as `GET LIGHTS` answers them.
    fn a_lights_answer() -> serde_json::Value {
        serde_json::json!({ "lights": [
            { "light": { "id": "fake:a#z0" }, "led_count": 12, "connected": true, "label": "Desk strip" },
            { "light": { "id": "fake:b#z0" }, "led_count": 3, "connected": false, "label": "Case fan" },
        ]})
    }

    /// A groups file under the home: one group, named `Fans`, with a brightness of 70.
    fn a_groups_file(home: &TempHome) -> String {
        home.write(
            "groups.json",
            r#"[{"name":"Fans","members":["fake:a#z0"],"plays":{"type":"off"},"brightness":70}]"#,
        )
    }

    /// The record an earlier create of run `agent-1` would have written.
    fn a_record(scenes: serde_json::Value, prior_active: &str) -> String {
        serde_json::json!({
            "run": "agent-1",
            "prior_active": prior_active,
            "before": run_scenes(),
            "scenes": scenes,
        })
        .to_string()
    }

    /// The record's file under a home.
    fn record_at(home: &TempHome) -> PathBuf {
        home.path().join("agent-runs").join("agent-1.json")
    }

    /// A record with a pending write list, as a crashed run would leave it.
    fn a_record_pending(scenes: serde_json::Value, prior_active: &str, pending: serde_json::Value) -> String {
        let mut record: serde_json::Value = serde_json::from_str(&a_record(scenes, prior_active)).unwrap();
        record["pending"] = pending;
        record.to_string()
    }

    /// The record on disk, parsed.
    fn record_json(home: &TempHome) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(record_at(home)).expect("the record is there"))
            .expect("the record is JSON")
    }

    /// A hook that copies the record's text at the moment a write arrives.
    fn record_seen_at_write(
        path: PathBuf,
        seen: std::sync::Arc<std::sync::Mutex<String>>,
    ) -> impl Fn() + 'static {
        move || *seen.lock().unwrap() = std::fs::read_to_string(&path).unwrap_or_default()
    }

    /// A hook that makes the record's path a folder holding a file, so the next record write fails.
    fn break_the_record_at_write(path: PathBuf) -> impl Fn() + 'static {
        move || {
            let _ = std::fs::remove_file(&path);
            std::fs::create_dir_all(&path).expect("the folder is made");
            std::fs::write(path.join("keep"), "x").expect("the file is written");
        }
    }

    #[test]
    fn scenes_create_sends_the_groups_and_writes_the_run_record() {
        let home = TempHome::new("create");
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let args = CreateArgs {
            name: "Night".into(),
            from: Some(a_groups_file(&home)),
            run: Some("agent-1".into()),
            dry_run: false,
        };
        assert_eq!(text_of(scenes_create(&api, &args, false).unwrap()), "created night: 1 group\n");

        let (method, path, body) = api.write();
        assert_eq!((method.as_str(), path.as_str()), ("POST", canvas_proto::api::routes::SCENES));
        let body: serde_json::Value = serde_json::from_str(body.as_deref().expect("a body")).unwrap();
        assert_eq!(body["name"], "Night");
        assert_eq!(body["groups"][0]["name"], "Fans");
        assert_eq!(body["groups"][0]["members"], serde_json::json!(["fake:a#z0"]));
        assert_eq!(body["groups"][0]["brightness"], 70);
        assert!(body.get("dry_run").is_none(), "a real create sends no dry_run: {body}");

        let record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(record_at(&home)).expect("the record is written")).unwrap();
        assert_eq!(record["run"], "agent-1");
        assert_eq!(record["prior_active"], "gaming", "the active scene before the run: {record}");
        assert_eq!(record["before"], run_scenes(), "the list at the run's first write: {record}");
        assert_eq!(
            record["scenes"],
            serde_json::json!([{ "id": "night", "name": "Night", "rev": 1 }]),
            "{record}"
        );
        assert!(record.get("pending").is_none(), "the pending create is cleared: {record}");
    }

    #[test]
    fn a_second_create_by_the_same_run_keeps_its_first_before() {
        let home = TempHome::new("create-twice");
        let first = a_record(serde_json::json!([{ "id": "night", "name": "Night", "rev": 3 }]), "movie");
        home.write("agent-runs/agent-1.json", &first);
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let args = CreateArgs {
            name: "Night".into(),
            from: Some(a_groups_file(&home)),
            run: Some("agent-1".into()),
            dry_run: false,
        };
        scenes_create(&api, &args, false).unwrap();
        let record: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(record_at(&home)).unwrap()).unwrap();
        assert_eq!(record["prior_active"], "movie", "the record keeps the scene the run started from");
        assert_eq!(
            record["scenes"],
            serde_json::json!([{ "id": "night", "name": "Night", "rev": 1 }]),
            "the scene is noted once, at its latest rev: {record}"
        );
    }

    #[test]
    fn scenes_create_refuses_a_name_another_scene_has_whatever_its_case() {
        let home = TempHome::new("clash");
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let args =
            CreateArgs { name: " GAMING ".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        let error = scenes_create(&api, &args, false).expect_err("the name is taken");
        assert_eq!(
            error.to_string(),
            "a scene named \"GAMING\" already exists (id gaming); pick another name"
        );
        assert_eq!(error.exit_code(), 1);
        assert_eq!(api.asked().len(), 1, "only the list was read: {:?}", api.asked());
        assert!(home.files().is_empty(), "nothing was written: {:?}", home.files());
    }

    #[test]
    fn a_dry_run_writes_no_record_and_no_scene_copy() {
        let home = TempHome::new("dry-run");
        let api =
            RunApi::answering(run_scenes(), serde_json::json!({ "saved": false, "scene": a_saved_scene() }));
        let args = CreateArgs {
            name: "Night".into(),
            from: Some(a_groups_file(&home)),
            run: Some("agent-1".into()),
            dry_run: true,
        };
        assert_eq!(
            text_of(scenes_create(&api, &args, false).unwrap()),
            "would create night: 1 group (nothing saved)\n"
        );
        let (_, _, body) = api.write();
        let body: serde_json::Value = serde_json::from_str(body.as_deref().expect("a body")).unwrap();
        assert_eq!(body["dry_run"], true, "the daemon runs the checks and saves nothing");
        assert_eq!(
            home.files(),
            vec![home.path().join("groups.json")],
            "the only file is the one the test wrote: a dry run saves nothing"
        );
        match scenes_create(&api, &args, true).unwrap() {
            Output::Json(value) => {
                assert_eq!(value["saved"], false);
                assert_eq!(value["run"], "agent-1");
                assert_eq!(value["scene"]["id"], "night");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scenes_create_without_a_run_writes_no_record() {
        let home = TempHome::new("no-run");
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let args = CreateArgs { name: "Night".into(), from: None, run: None, dry_run: false };
        assert_eq!(text_of(scenes_create(&api, &args, false).unwrap()), "created night: 1 group\n");
        let (_, _, body) = api.write();
        let body: serde_json::Value = serde_json::from_str(body.as_deref().expect("a body")).unwrap();
        assert!(body.get("groups").is_none(), "no --from, no groups: {body}");
        assert!(home.files().is_empty(), "no --run, no record: {:?}", home.files());
    }

    #[test]
    fn the_json_answer_of_a_create_is_one_document_with_the_run_and_the_scene() {
        let home = TempHome::new("create-json");
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let args = CreateArgs { name: "Night".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        match scenes_create(&api, &args, true).unwrap() {
            Output::Json(value) => {
                assert_eq!(value["saved"], true);
                assert_eq!(value["run"], "agent-1");
                assert_eq!(value["scene"]["id"], "night");
                assert_eq!(value["scene"]["rev"], 1);
                assert_eq!(value["scene"]["groups"][0]["id"], "fans");
                assert_eq!(value["scene"]["groups"][0]["plays"], serde_json::json!({ "type": "off" }));
            }
            other => panic!("{other:?}"),
        }
        assert!(record_at(&home).is_file(), "a real create writes the record");
    }

    #[test]
    fn scenes_edit_sends_the_rev_it_read_and_updates_the_record() {
        let home = TempHome::new("edit");
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming"),
        );
        let api = RunApi::answering(
            run_scenes(),
            serde_json::json!({
                "id": "movie", "name": "Movie", "rev": 8,
                "groups": [
                    { "id": "lamp", "name": "Lamp", "members": ["fake:b#z0"], "plays": { "type": "off" } },
                    { "id": "wall", "name": "Wall", "members": ["fake:c#z0"], "plays": { "type": "off" } },
                ]
            }),
        );
        let groups = home.write(
            "groups.json",
            r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}},{"name":"Wall","members":["fake:c#z0"],"plays":{"type":"off"},"brightness":40}]"#,
        );
        let args = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: false };
        assert_eq!(text_of(scenes_edit(&api, &args, false).unwrap()), "saved movie: 2 groups\n");

        let (method, path, body) = api.write();
        assert_eq!((method.as_str(), path.as_str()), ("PUT", "/api/v1/scenes/movie"));
        let body: serde_json::Value = serde_json::from_str(body.as_deref().expect("a body")).unwrap();
        assert_eq!(body["expected_rev"], 7, "the rev it read goes with the write: {body}");
        assert_eq!(body["scene"]["id"], "movie");
        assert_eq!(body["scene"]["groups"][0]["id"], "lamp", "a group of the same name keeps its id");
        assert_eq!(body["scene"]["groups"][1]["id"], "wall", "a new group gets its name's id");
        assert_eq!(body["scene"]["groups"][1]["brightness"], 40);

        // The scene as it was is copied first, so the edit can be undone.
        let copy = std::fs::read_to_string(home.path().join("agent-runs/agent-1/movie.rev7.json"))
            .expect("the copy is written before the call");
        assert!(copy.contains("\"Lamp\""), "the copy is the whole scene: {copy}");

        let record: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(record_at(&home)).unwrap()).unwrap();
        assert_eq!(
            record["scenes"],
            serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 8 }]),
            "the record follows the scene's rev: {record}"
        );
    }

    #[test]
    fn a_dry_run_of_an_edit_sends_no_copy_and_writes_no_record() {
        let home = TempHome::new("edit-dry-run");
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);
        let api = RunApi::answering(
            run_scenes(),
            serde_json::json!({ "saved": false, "scene": {
                "id": "movie", "name": "Movie", "rev": 8, "groups": [] } }),
        );
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        let args = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: true };
        assert_eq!(text_of(scenes_edit(&api, &args, false).unwrap()), "would save movie: 0 groups (nothing saved)\n");
        let (method, path, body) = api.write();
        assert_eq!((method.as_str(), path.as_str()), ("PUT", "/api/v1/scenes/movie"));
        let body: serde_json::Value = serde_json::from_str(body.as_deref().expect("a body")).unwrap();
        assert_eq!(body["dry_run"], true, "the edit's dry run tells the daemon to save nothing: {body}");
        assert_eq!(
            std::fs::read_to_string(record_at(&home)).unwrap(),
            record,
            "a dry run leaves the record as it was"
        );
        assert!(
            !home.path().join("agent-runs/agent-1").exists(),
            "a dry run writes no copy: {:?}",
            home.files()
        );
    }

    #[test]
    fn a_stale_rev_is_a_409_and_the_record_does_not_change() {
        let home = TempHome::new("stale");
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);
        let api = RunApi::failing(
            run_scenes(),
            409,
            serde_json::json!({
                "error": "conflict", "message": "this scene was changed in another window",
                "scene": { "id": "movie", "name": "Movie", "rev": 9, "groups": [] }
            }),
        );
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        let args = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: false };
        let error = scenes_edit(&api, &args, false).expect_err("the rev is stale");
        assert_eq!(error.exit_code(), 1);
        let text = error.to_string();
        assert!(text.contains("409"), "{text}");
        assert!(text.contains("scene movie is at rev 9 now"), "the current rev is shown: {text}");
        assert_eq!(
            record_json(&home),
            serde_json::from_str::<serde_json::Value>(&record).unwrap(),
            "the daemon refused the write, so the record is as it was and holds no pending entry"
        );
        assert!(
            home.path().join("agent-runs/agent-1/movie.rev7.json").is_file(),
            "the copy was taken before the write, so the scene as it was is kept"
        );
    }

    #[test]
    fn an_edit_or_a_delete_of_a_scene_the_run_did_not_make_is_refused() {
        let home = TempHome::new("not-made");
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        let api = RunApi::answering(run_scenes(), serde_json::Value::Null);
        let edit = EditArgs { scene: "sleep".into(), from: groups, run: "agent-1".into(), dry_run: false };
        let error = scenes_edit(&api, &edit, false).expect_err("the run did not make sleep");
        assert_eq!(error.to_string(), "sleep was not made by run agent-1; nothing changed");
        assert_eq!(error.exit_code(), 1);
        let delete = DeleteArgs { scene: "sleep".into(), run: "agent-1".into(), force: true, dry_run: false };
        let error = scenes_delete(&api, &delete, false).expect_err("the run did not make sleep");
        assert_eq!(error.to_string(), "sleep was not made by run agent-1; nothing changed");
        assert!(
            api.asked().iter().all(|(method, _, _)| method == "GET"),
            "nothing was sent: {:?}",
            api.asked()
        );
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record);
    }

    #[test]
    fn a_missing_or_unreadable_run_record_refuses_an_edit_and_a_delete() {
        let home = TempHome::new("no-record");
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        let api = RunApi::answering(run_scenes(), serde_json::Value::Null);
        let edit = EditArgs { scene: "movie".into(), from: groups, run: "agent-9".into(), dry_run: false };
        assert_eq!(
            scenes_edit(&api, &edit, false).unwrap_err().to_string(),
            "no record of run agent-9; nothing changed"
        );
        let delete = DeleteArgs { scene: "movie".into(), run: "agent-9".into(), force: false, dry_run: false };
        assert_eq!(
            scenes_delete(&api, &delete, false).unwrap_err().to_string(),
            "no record of run agent-9; nothing changed"
        );
        assert!(api.asked().is_empty(), "nothing was read or sent: {:?}", api.asked());

        // A record that is there and cannot be read refuses the same way.
        home.write("agent-runs/agent-9.json", "{ this is not a record");
        assert_eq!(
            scenes_delete(&api, &delete, false).unwrap_err().to_string(),
            "no record of run agent-9; nothing changed"
        );
        assert!(api.asked().is_empty(), "still nothing was read or sent: {:?}", api.asked());
    }

    #[test]
    fn a_delete_of_the_active_scene_names_the_one_to_switch_to_first() {
        let home = TempHome::new("active");
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "gaming", "name": "Gaming", "rev": 4 }]), "movie"),
        );
        let api = RunApi::answering(run_scenes(), serde_json::Value::Null);
        // --force skips only the rev check, never this one.
        let args = DeleteArgs { scene: "gaming".into(), run: "agent-1".into(), force: true, dry_run: false };
        let error = scenes_delete(&api, &args, false).expect_err("the active scene is refused");
        assert_eq!(
            error.to_string(),
            "Gaming is the active scene. Switch first: canvas-rgb scenes set \"movie\""
        );
        assert_eq!(error.exit_code(), 1);
        assert!(
            api.asked().iter().all(|(method, _, _)| method == "GET"),
            "nothing was sent: {:?}",
            api.asked()
        );
    }

    #[test]
    fn a_scene_that_changed_since_the_run_made_it_needs_force() {
        let home = TempHome::new("changed");
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 3 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);

        let api = RunApi::answering(run_scenes(), serde_json::Value::Null);
        let args = DeleteArgs { scene: "movie".into(), run: "agent-1".into(), force: false, dry_run: false };
        let error = scenes_delete(&api, &args, false).expect_err("the rev changed");
        assert_eq!(
            error.to_string(),
            "Movie changed since run agent-1 made it (rev 7, was 3); nothing deleted. Use --force to delete it anyway."
        );
        assert_eq!(error.exit_code(), 1);
        assert_eq!(api.asked().len(), 1, "nothing was sent: {:?}", api.asked());
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record);

        let api = RunApi::answering(run_scenes(), serde_json::Value::Null);
        let args = DeleteArgs { scene: "movie".into(), run: "agent-1".into(), force: true, dry_run: false };
        assert_eq!(text_of(scenes_delete(&api, &args, false).unwrap()), "deleted movie\n");
        let (method, path, body) = api.write();
        assert_eq!((method.as_str(), path.as_str()), ("DELETE", "/api/v1/scenes/movie"));
        assert!(body.is_none(), "a delete sends no body");
        let copy = std::fs::read_to_string(home.path().join("agent-runs/agent-1/movie.rev7.json"))
            .expect("the copy is written before the delete");
        let copy: serde_json::Value = serde_json::from_str(&copy).unwrap();
        assert_eq!(copy["rev"], 7, "the copy is the scene it is deleting: {copy}");
        let record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(record_at(&home)).unwrap()).unwrap();
        assert_eq!(record["scenes"], serde_json::json!([]), "the scene is not the run's any more: {record}");
    }

    #[test]
    fn a_delete_that_fails_still_leaves_the_copy_and_the_record() {
        let home = TempHome::new("delete-fails");
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);
        let api = RunApi::failing(run_scenes(), 500, serde_json::json!({ "error": "internal", "message": "the save failed" }));
        let args = DeleteArgs { scene: "movie".into(), run: "agent-1".into(), force: false, dry_run: false };
        let error = scenes_delete(&api, &args, false).expect_err("the daemon failed");
        assert_eq!(error.exit_code(), 1);
        let copy = std::fs::read_to_string(home.path().join("agent-runs/agent-1/movie.rev7.json"))
            .expect("the copy is written before the call");
        assert!(copy.contains("\"Movie\""), "the copy is the whole scene: {copy}");
        let after = record_json(&home);
        assert_eq!(
            after["scenes"],
            serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]),
            "the record still lists the scene: {after}"
        );
        assert_eq!(
            after["pending"],
            serde_json::json!([{ "op": "delete", "scene": "movie" }]),
            "a 5xx leaves the outcome unknown, so the delete stays pending: {after}"
        );
        let _ = record;
    }

    #[test]
    fn a_scene_copy_that_cannot_be_written_sends_nothing() {
        let home = TempHome::new("copy-fails");
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);
        // A file where the run's folder must be: the copy cannot be written.
        home.write("agent-runs/agent-1", "not a folder");
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        let args = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: false };
        let error = scenes_edit(&api, &args, false).expect_err("the copy cannot be written");
        assert!(error.to_string().contains("could not save the scene"), "{error}");
        assert_eq!(error.exit_code(), 1);
        assert_eq!(api.asked().len(), 1, "only the read: the daemon was never called: {:?}", api.asked());
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record);
    }

    #[test]
    fn a_bad_run_id_is_refused_before_anything_is_read_or_written() {
        let home = TempHome::new("bad-id");
        let too_long = "a".repeat(65);
        for run in [
            "../x", "", too_long.as_str(), ".", ".x", "x.", "con", "NUL", "com1", "lpt9.json",
            "com0", "LPT0", "conin$", "conout$",
        ] {
            let api = RunApi::answering(run_scenes(), a_saved_scene());
            let create = CreateArgs { name: "Night".into(), from: None, run: Some(run.to_string()), dry_run: false };
            let error = scenes_create(&api, &create, false).expect_err("not a run id");
            assert!(error.to_string().contains("is not a run id"), "{run:?}: {error}");
            assert_eq!(error.exit_code(), 1);
            let edit = EditArgs { scene: "movie".into(), from: "groups.json".into(), run: run.to_string(), dry_run: false };
            assert!(
                scenes_edit(&api, &edit, false).unwrap_err().to_string().contains("is not a run id"),
                "{run:?} on an edit"
            );
            let delete = DeleteArgs { scene: "movie".into(), run: run.to_string(), force: false, dry_run: false };
            assert!(
                scenes_delete(&api, &delete, false).unwrap_err().to_string().contains("is not a run id"),
                "{run:?} on a delete"
            );
            assert!(api.asked().is_empty(), "{run:?}: nothing was sent: {:?}", api.asked());
        }
        assert!(home.files().is_empty(), "nothing was written: {:?}", home.files());
    }

    #[test]
    fn an_older_daemon_is_sent_nothing_for_a_create_from_a_file_a_dry_run_or_an_edit() {
        let home = TempHome::new("old-daemon");
        let own = env!("CARGO_PKG_VERSION");
        let groups = a_groups_file(&home);
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);
        let files = home.files();
        for which in 0..4 {
            let api = RunApi::answering(run_scenes(), a_saved_scene())
                .with_status(serde_json::json!({ "version": "0.0.1" }));
            let create = |from: Option<String>, dry_run: bool| CreateArgs {
                name: "Night".into(),
                from,
                run: Some("agent-1".into()),
                dry_run,
            };
            let edit = |dry_run: bool| EditArgs {
                scene: "movie".into(),
                from: groups.clone(),
                run: "agent-1".into(),
                dry_run,
            };
            let (what, result) = match which {
                0 => ("create --from", scenes_create(&api, &create(Some(groups.clone()), false), false)),
                1 => ("create --dry-run", scenes_create(&api, &create(None, true), false)),
                2 => ("edit", scenes_edit(&api, &edit(false), false)),
                _ => ("edit --dry-run", scenes_edit(&api, &edit(true), false)),
            };
            let error = result.expect_err(what);
            let text = error.to_string();
            assert!(text.contains("0.0.1") && text.contains(own), "{what}: both versions are named: {text}");
            assert!(text.contains("restart canvas-rgb"), "{what}: {text}");
            assert_eq!(error.exit_code(), 1, "{what}");
            assert!(api.asked().is_empty(), "{what}: no request but the version read: {:?}", api.asked());
            assert_eq!(api.status_reads(), 1, "{what}");
            assert_eq!(home.files(), files, "{what}: nothing was written");
        }
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record);
    }

    #[test]
    fn a_plain_create_never_asks_the_version_and_a_newer_daemon_passes_the_gate() {
        let home = TempHome::new("gate-open");
        let api = RunApi::answering(run_scenes(), a_saved_scene())
            .with_status(serde_json::json!({ "version": "0.0.1" }));
        let args = CreateArgs { name: "Night".into(), from: None, run: None, dry_run: false };
        scenes_create(&api, &args, false).unwrap();
        assert_eq!(api.status_reads(), 0, "a plain create sends nothing an old daemon misreads");

        let api = RunApi::answering(run_scenes(), a_saved_scene())
            .with_status(serde_json::json!({ "version": "99.0.0" }));
        let args = CreateArgs { name: "Night".into(), from: Some(a_groups_file(&home)), run: None, dry_run: false };
        scenes_create(&api, &args, false).unwrap();
        assert_eq!(api.status_reads(), 1);
        api.write();
    }

    #[test]
    fn versions_compare_by_their_three_numbers_and_an_unreadable_one_is_refused() {
        let ask = |reported: serde_json::Value, own: &str| {
            let api = RunApi::answering(run_scenes(), a_saved_scene()).with_status(reported);
            check_daemon_version(&api, own)
        };
        assert!(ask(serde_json::json!({ "version": "0.10.0" }), "0.9.0").is_ok(), "0.10.0 is newer than 0.9.0");
        assert!(ask(serde_json::json!({ "version": "0.9.0" }), "0.10.0").is_err(), "0.9.0 is older than 0.10.0");
        assert!(ask(serde_json::json!({ "version": "1.2.3" }), "1.2.3").is_ok());
        assert!(ask(serde_json::json!({ "version": "v1.2.3-beta+7" }), "1.2.3").is_ok());
        let error = ask(serde_json::json!({}), "1.2.3").unwrap_err().to_string();
        assert!(error.contains("no version") && error.contains("1.2.3") && error.contains("restart canvas-rgb"), "{error}");
        for bad in ["", "abc", "1.2", "1.2.3.4", "1.x.3"] {
            let error = ask(serde_json::json!({ "version": bad }), "1.2.3").unwrap_err().to_string();
            assert!(error.contains("cannot be read") && error.contains("1.2.3"), "{bad:?}: {error}");
        }
    }

    #[test]
    fn a_dry_run_answer_without_saved_false_is_an_error_and_a_real_write_needs_no_saved_field() {
        let home = TempHome::new("saved-field");
        let groups = a_groups_file(&home);
        // A daemon that ignored dry_run answers the plain scene: it wrote.
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let create = CreateArgs { name: "Night".into(), from: Some(groups.clone()), run: None, dry_run: true };
        let error = scenes_create(&api, &create, false).expect_err("a dry run needs saved: false");
        assert!(error.to_string().contains("the daemon did not say whether it saved"), "{error}");
        assert_eq!(error.exit_code(), 1);
        // A dry run that says it saved is an error too.
        let api = RunApi::answering(run_scenes(), serde_json::json!({ "saved": true, "scene": a_saved_scene() }));
        let error = scenes_create(&api, &create, false).expect_err("saved: true on a dry run");
        assert!(error.to_string().contains("the daemon says it saved the scene"), "{error}");
        // The same two for an edit's dry run.
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming"),
        );
        let edit = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: true };
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let error = scenes_edit(&api, &edit, false).expect_err("a dry run needs saved: false");
        assert!(error.to_string().contains("the daemon did not say whether it saved"), "{error}");
        let api = RunApi::answering(run_scenes(), serde_json::json!({ "saved": true, "scene": a_saved_scene() }));
        let error = scenes_edit(&api, &edit, false).expect_err("saved: true on a dry run");
        assert!(error.to_string().contains("the daemon says it saved the scene"), "{error}");
        // A real write answers the plain scene, as the spine says: that is the normal answer.
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let real = CreateArgs { name: "Night".into(), from: None, run: None, dry_run: false };
        assert_eq!(text_of(scenes_create(&api, &real, false).unwrap()), "created night: 1 group\n");
    }

    #[test]
    fn each_write_is_pending_in_the_record_while_its_request_is_on_the_way() {
        let home = TempHome::new("pending");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let during = |seen: &std::sync::Arc<std::sync::Mutex<String>>| -> serde_json::Value {
            serde_json::from_str(&seen.lock().unwrap()).expect("the record is on disk when the request goes out")
        };

        // A create.
        let api = RunApi::answering(run_scenes(), a_saved_scene())
            .on_write(record_seen_at_write(record_at(&home), seen.clone()));
        let args = CreateArgs { name: "Night".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        scenes_create(&api, &args, false).unwrap();
        assert_eq!(during(&seen)["pending"], serde_json::json!([{ "op": "create", "scene": "Night" }]));
        assert!(record_json(&home).get("pending").is_none(), "cleared after the record's own update");

        // An edit.
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming"),
        );
        let moved = serde_json::json!({ "id": "movie", "name": "Movie", "rev": 8, "groups": [] });
        let api = RunApi::answering(run_scenes(), moved)
            .on_write(record_seen_at_write(record_at(&home), seen.clone()));
        let edit = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: false };
        scenes_edit(&api, &edit, false).unwrap();
        assert_eq!(during(&seen)["pending"], serde_json::json!([{ "op": "edit", "scene": "movie" }]));
        assert!(record_json(&home).get("pending").is_none());

        // A delete.
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming"),
        );
        let api = RunApi::answering(run_scenes(), serde_json::Value::Null)
            .on_write(record_seen_at_write(record_at(&home), seen.clone()));
        let delete = DeleteArgs { scene: "movie".into(), run: "agent-1".into(), force: false, dry_run: false };
        scenes_delete(&api, &delete, false).unwrap();
        assert_eq!(during(&seen)["pending"], serde_json::json!([{ "op": "delete", "scene": "movie" }]));
        assert!(record_json(&home).get("pending").is_none());
    }

    #[test]
    fn a_create_the_daemon_refused_leaves_no_pending_and_one_it_may_have_run_keeps_it() {
        let home = TempHome::new("pending-refused");
        let args = CreateArgs { name: "Night".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        let api = RunApi::failing(run_scenes(), 422, serde_json::json!({ "error": "validation", "message": "no" }));
        scenes_create(&api, &args, false).expect_err("refused");
        assert!(record_json(&home).get("pending").is_none(), "a 422 wrote nothing");
        let api = RunApi::failing(run_scenes(), 503, serde_json::json!({ "error": "unavailable", "message": "later" }));
        scenes_create(&api, &args, false).expect_err("unavailable");
        assert_eq!(
            record_json(&home)["pending"],
            serde_json::json!([{ "op": "create", "scene": "Night" }]),
            "a 5xx may have written: the record keeps the trace"
        );
    }

    #[test]
    fn a_create_whose_pending_entry_cannot_be_saved_sends_nothing() {
        let home = TempHome::new("pending-fails");
        // A folder where the record's temporary file goes: the record cannot be written.
        home.write("agent-runs/.agent-1.json.tmp/keep", "x");
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        let args = CreateArgs { name: "Night".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        let error = scenes_create(&api, &args, false).expect_err("the record cannot be written");
        assert!(error.to_string().ends_with("nothing was sent"), "{error}");
        assert_eq!(error.exit_code(), 1);
        assert!(
            api.asked().iter().all(|(method, _, _)| method == "GET"),
            "no write was sent: {:?}",
            api.asked()
        );
    }

    #[test]
    fn a_record_that_cannot_be_written_after_the_daemon_wrote_says_the_scene_was_made() {
        let home = TempHome::new("record-fails-after");
        // The daemon's answer arrives, and the record's file has become a folder by then.
        let api = RunApi::answering(run_scenes(), a_saved_scene())
            .on_write(break_the_record_at_write(record_at(&home)));
        let args = CreateArgs { name: "Night".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        let error = scenes_create(&api, &args, false).expect_err("the record cannot be written");
        let text = error.to_string();
        assert!(text.starts_with("the scene night was created, but its run record could not be written"), "{text}");
        assert_eq!(error.exit_code(), 1);
        api.write();
        drop(home);

        let home = TempHome::new("record-fails-after-edit");
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming"),
        );
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        let moved = serde_json::json!({ "id": "movie", "name": "Movie", "rev": 8, "groups": [] });
        let api = RunApi::answering(run_scenes(), moved).on_write(break_the_record_at_write(record_at(&home)));
        let edit = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: false };
        let text = scenes_edit(&api, &edit, false).expect_err("the record cannot be written").to_string();
        assert!(text.starts_with("the scene movie was saved, but its run record could not be written"), "{text}");
    }

    #[test]
    fn a_delete_dry_run_sends_no_delete_and_writes_no_copy() {
        let home = TempHome::new("delete-dry-run");
        let record = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        home.write("agent-runs/agent-1.json", &record);
        let files = home.files();
        let api = RunApi::answering(run_scenes(), serde_json::Value::Null);
        let args = DeleteArgs { scene: "movie".into(), run: "agent-1".into(), force: false, dry_run: true };
        assert_eq!(text_of(scenes_delete(&api, &args, false).unwrap()), "would delete movie\n");
        assert!(
            api.asked().iter().all(|(method, _, _)| method == "GET"),
            "no DELETE was sent: {:?}",
            api.asked()
        );
        assert_eq!(home.files(), files, "no copy was written");
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record, "the record is as it was");
        match scenes_delete(&api, &args, true).unwrap() {
            Output::Json(value) => assert_eq!(value["saved"], false),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_groups_file_that_is_missing_bad_or_too_big_gives_one_sentence_and_sends_nothing() {
        let home = TempHome::new("bad-from");
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming"),
        );
        let missing = home.path().join("nope.json").to_string_lossy().into_owned();
        let not_groups = home.write("bad.json", "{ not json");
        let big = home.write("big.json", &" ".repeat(1024 * 1024 + 1));
        let at_the_limit = home.write("limit.json", &format!("[]{}", " ".repeat(1024 * 1024 - 2)));
        for (file, expect) in [
            (missing.clone(), "could not read"),
            (not_groups.clone(), "is not a list of groups"),
            (big.clone(), "over 1 MiB"),
        ] {
            let api = RunApi::answering(run_scenes(), a_saved_scene());
            let create = CreateArgs {
                name: "Night".into(),
                from: Some(file.clone()),
                run: Some("agent-1".into()),
                dry_run: false,
            };
            let text = scenes_create(&api, &create, false).expect_err("a bad file").to_string();
            assert!(text.contains(&file) && text.contains(expect), "{text}");
            assert!(!text.contains('\n'), "one sentence: {text}");
            assert!(api.asked().is_empty() && api.status_reads() == 0, "create sent nothing: {:?}", api.asked());
            let edit = EditArgs { scene: "movie".into(), from: file.clone(), run: "agent-1".into(), dry_run: false };
            let text = scenes_edit(&api, &edit, false).expect_err("a bad file").to_string();
            assert!(text.contains(expect), "{text}");
            assert!(api.asked().is_empty() && api.status_reads() == 0, "edit sent nothing: {:?}", api.asked());
        }
        // A file of exactly 1 MiB is read.
        assert_eq!(read_groups(&at_the_limit).expect("1 MiB is allowed").len(), 0);
    }

    #[test]
    fn a_scene_the_record_does_not_list_is_never_deleted_whatever_its_pending_create_says() {
        let home = TempHome::new("no-adoption");
        let mut list = run_scenes();
        list["scenes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "id": "night", "name": "Night", "rev": 1, "groups": [] }));
        let night = DeleteArgs { scene: "night".into(), run: "agent-1".into(), force: false, dry_run: false };
        let refusal = "night was not recorded as made by run agent-1, and a create of \"Night\" by this run has no known outcome, so nothing was deleted; delete it in the app if it is the run's.";

        // A pending create of this name, the scene at rev 1, not in the first list: still refused.
        // `--force` does not change that, and neither does `--json`.
        let record = a_record_pending(
            serde_json::json!([]),
            "gaming",
            serde_json::json!([{ "op": "create", "scene": "Night" }]),
        );
        home.write("agent-runs/agent-1.json", &record);
        let files = home.files();
        for (force, json) in [(false, false), (true, false), (false, true)] {
            let api = RunApi::answering(list.clone(), serde_json::Value::Null);
            let args = DeleteArgs { scene: "night".into(), run: "agent-1".into(), force, dry_run: false };
            let error = scenes_delete(&api, &args, json).expect_err("a scene the record does not list");
            assert_eq!(error.to_string(), refusal, "force {force}, json {json}");
            assert!(!error.to_string().contains('\n'), "one sentence");
            assert!(
                api.asked().iter().all(|(method, _, _)| method == "GET"),
                "the fake got no delete request: {:?}",
                api.asked()
            );
            assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record, "the record is as it was");
            assert_eq!(home.files(), files, "no copy was written");
        }

        // The pending name matches without regard to case, and a changed rev makes no difference.
        let mut changed = list.clone();
        changed["scenes"][3]["rev"] = serde_json::json!(2);
        home.write(
            "agent-runs/agent-1.json",
            &a_record_pending(
                serde_json::json!([]),
                "gaming",
                serde_json::json!([{ "op": "create", "scene": "NIGHT" }]),
            ),
        );
        let api = RunApi::answering(changed, serde_json::Value::Null);
        assert_eq!(scenes_delete(&api, &night, false).unwrap_err().to_string(), refusal);
        assert!(api.asked().iter().all(|(method, _, _)| method == "GET"));

        // A pending create of another name: the plain refusal, with the pending write named.
        home.write(
            "agent-runs/agent-1.json",
            &a_record_pending(
                serde_json::json!([]),
                "gaming",
                serde_json::json!([{ "op": "create", "scene": "Other" }, { "op": "delete", "scene": "movie" }]),
            ),
        );
        let api = RunApi::answering(list.clone(), serde_json::Value::Null);
        assert_eq!(
            scenes_delete(&api, &night, false).unwrap_err().to_string(),
            "night was not made by run agent-1; nothing changed. Run agent-1 holds writes with no known outcome: create Other, delete movie."
        );
        assert!(api.asked().iter().all(|(method, _, _)| method == "GET"));

        // No pending write: the plain refusal, with nothing added.
        home.write("agent-runs/agent-1.json", &a_record(serde_json::json!([]), "gaming"));
        let api = RunApi::answering(list.clone(), serde_json::Value::Null);
        assert_eq!(
            scenes_delete(&api, &night, false).unwrap_err().to_string(),
            "night was not made by run agent-1; nothing changed"
        );
        assert!(api.asked().iter().all(|(method, _, _)| method == "GET"));
    }

    #[test]
    fn an_edit_of_a_scene_the_run_did_not_make_names_the_writes_the_run_holds() {
        let home = TempHome::new("edit-names-pending");
        let groups = a_groups_file(&home);
        let edit = EditArgs { scene: "sleep".into(), from: groups, run: "agent-1".into(), dry_run: false };
        home.write(
            "agent-runs/agent-1.json",
            &a_record_pending(
                serde_json::json!([]),
                "gaming",
                serde_json::json!([{ "op": "create", "scene": "Night" }]),
            ),
        );
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        assert_eq!(
            scenes_edit(&api, &edit, false).unwrap_err().to_string(),
            "sleep was not made by run agent-1; nothing changed. Run agent-1 holds writes with no known outcome: create Night."
        );
        assert!(api.asked().iter().all(|(method, _, _)| method == "GET"), "{:?}", api.asked());
        // With nothing pending, the sentence is as it was.
        home.write("agent-runs/agent-1.json", &a_record(serde_json::json!([]), "gaming"));
        let api = RunApi::answering(run_scenes(), a_saved_scene());
        assert_eq!(
            scenes_edit(&api, &edit, false).unwrap_err().to_string(),
            "sleep was not made by run agent-1; nothing changed"
        );
    }

    #[test]
    fn a_failure_the_daemon_may_have_seen_keeps_the_pending_entry_and_a_certain_refusal_clears_it() {
        let home = TempHome::new("pending-other");
        let made = a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming");
        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        let create = CreateArgs { name: "Night".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        let edit = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: false };
        let delete = DeleteArgs { scene: "movie".into(), run: "agent-1".into(), force: false, dry_run: false };

        // An error the transport raised after the request went out (a reset, say) is `Other`:
        // the write may have landed, so create, edit and delete all keep their entry.
        let reset = || CliError::Other("connection reset by peer".to_string());
        let api = RunApi::answering(run_scenes(), a_saved_scene()).write_fails_with(reset);
        scenes_create(&api, &create, false).expect_err("reset");
        assert_eq!(
            record_json(&home)["pending"],
            serde_json::json!([{ "op": "create", "scene": "Night" }]),
            "Other after the write arrived keeps the create pending"
        );
        home.write("agent-runs/agent-1.json", &made);
        let api = RunApi::answering(run_scenes(), a_saved_scene()).write_fails_with(reset);
        scenes_edit(&api, &edit, false).expect_err("reset");
        assert_eq!(record_json(&home)["pending"], serde_json::json!([{ "op": "edit", "scene": "movie" }]));
        home.write("agent-runs/agent-1.json", &made);
        let api = RunApi::answering(run_scenes(), serde_json::Value::Null).write_fails_with(reset);
        scenes_delete(&api, &delete, false).expect_err("reset");
        assert_eq!(record_json(&home)["pending"], serde_json::json!([{ "op": "delete", "scene": "movie" }]));

        // No answer in time keeps it too.
        home.write("agent-runs/agent-1.json", &made);
        let slow = || CliError::NoAnswer("slow".to_string());
        let api = RunApi::answering(run_scenes(), serde_json::Value::Null).write_fails_with(slow);
        scenes_delete(&api, &delete, false).expect_err("no answer");
        assert_eq!(record_json(&home)["pending"], serde_json::json!([{ "op": "delete", "scene": "movie" }]));

        // A definite refusal clears it: the daemon not running, not trusted, or a 4xx.
        for (label, make) in [
            ("not running", (|| CliError::NotRunning("port".to_string())) as fn() -> CliError),
            ("not trusted", || CliError::NotTrusted("someone else".to_string())),
        ] {
            home.write("agent-runs/agent-1.json", &made);
            let api = RunApi::answering(run_scenes(), serde_json::Value::Null).write_fails_with(make);
            scenes_delete(&api, &delete, false).expect_err(label);
            assert!(record_json(&home).get("pending").is_none(), "{label} clears the pending delete");
        }
        home.write("agent-runs/agent-1.json", &made);
        let api = RunApi::failing(run_scenes(), 499, serde_json::json!({ "message": "no" }));
        scenes_delete(&api, &delete, false).expect_err("499");
        assert!(record_json(&home).get("pending").is_none(), "a status under 500 clears it");
        home.write("agent-runs/agent-1.json", &made);
        let api = RunApi::failing(run_scenes(), 500, serde_json::json!({ "message": "no" }));
        scenes_delete(&api, &delete, false).expect_err("500");
        assert_eq!(record_json(&home)["pending"], serde_json::json!([{ "op": "delete", "scene": "movie" }]));
    }

    #[test]
    fn a_real_write_answered_saved_false_clears_its_pending_entry() {
        let home = TempHome::new("saved-false");
        let not_saved = serde_json::json!({ "saved": false, "scene": a_saved_scene() });
        let create = CreateArgs { name: "Night".into(), from: None, run: Some("agent-1".into()), dry_run: false };
        let api = RunApi::answering(run_scenes(), not_saved.clone());
        let error = scenes_create(&api, &create, false).expect_err("the daemon says it did not save");
        assert_eq!(error.to_string(), "the daemon says it did not save the scene");
        assert!(record_json(&home).get("pending").is_none(), "create: nothing was saved, so nothing is pending");

        let groups = home.write("groups.json", r#"[{"name":"Lamp","members":["fake:b#z0"],"plays":{"type":"off"}}]"#);
        home.write(
            "agent-runs/agent-1.json",
            &a_record(serde_json::json!([{ "id": "movie", "name": "Movie", "rev": 7 }]), "gaming"),
        );
        let edit = EditArgs { scene: "movie".into(), from: groups, run: "agent-1".into(), dry_run: false };
        let api = RunApi::answering(run_scenes(), not_saved);
        let error = scenes_edit(&api, &edit, false).expect_err("the daemon says it did not save");
        assert_eq!(error.to_string(), "the daemon says it did not save the scene");
        assert!(record_json(&home).get("pending").is_none(), "edit: nothing was saved, so nothing is pending");

        // An answer that is no scene says nothing about whether it saved: the entry stays.
        let api = RunApi::answering(run_scenes(), serde_json::json!({ "unexpected": 1 }));
        scenes_create(&api, &create, false).expect_err("not a scene");
        assert_eq!(
            record_json(&home)["pending"],
            serde_json::json!([{ "op": "create", "scene": "Night" }]),
            "only an explicit saved: false clears it"
        );
    }

    #[test]
    fn under_json_every_failure_of_a_scene_write_is_one_document() {
        let create = Command::Scenes(ScenesCommand::Create(CreateArgs {
            name: "Night".into(),
            from: None,
            run: Some("agent-1".into()),
            dry_run: false,
        }));
        let other = CliError::Other("a scene named \"Night\" already exists (id night); pick another name".into());
        assert_eq!(
            failure_document(&create, &other),
            Some(serde_json::json!({
                "saved": false,
                "run": "agent-1",
                "error": "a scene named \"Night\" already exists (id night); pick another name",
            }))
        );
        let api = CliError::Api {
            status: 409,
            message: "this scene was changed in another window".into(),
            body: Some(serde_json::json!({ "scene": { "id": "movie", "rev": 9 } })),
        };
        let edit = Command::Scenes(ScenesCommand::Edit(EditArgs {
            scene: "movie".into(),
            from: "g.json".into(),
            run: "agent-2".into(),
            dry_run: false,
        }));
        assert_eq!(
            failure_document(&edit, &api),
            Some(serde_json::json!({
                "saved": false,
                "run": "agent-2",
                "error": api.to_string(),
            }))
        );
        assert_eq!(api.exit_code(), 1);
        assert_eq!(other.exit_code(), 1);
        // No run named: null. A delete names one. Other commands keep their own output.
        let bare = Command::Scenes(ScenesCommand::Create(CreateArgs {
            name: "x".into(),
            from: None,
            run: None,
            dry_run: false,
        }));
        assert_eq!(failure_document(&bare, &other).unwrap()["run"], serde_json::Value::Null);
        let delete = Command::Scenes(ScenesCommand::Delete(DeleteArgs {
            scene: "x".into(),
            run: "agent-3".into(),
            force: false,
            dry_run: false,
        }));
        assert_eq!(failure_document(&delete, &other).unwrap()["run"], "agent-3");
        assert_eq!(failure_document(&Command::Lights(LightsCommand::List), &other), None);
        assert_eq!(failure_document(&Command::Status, &other), None);
    }

    #[test]
    fn lights_list_prints_the_id_the_label_the_leds_and_whether_it_is_connected() {
        let api = RunApi::answering(serde_json::Value::Null, a_lights_answer());
        let text = text_of(lights_list(&api, false).expect("the lights"));
        let lines: Vec<Vec<&str>> = text.lines().map(|line| line.split_whitespace().collect()).collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[0], ["ID", "LABEL", "LEDS", "CONNECTED"], "{text}");
        assert_eq!(lines[1], ["fake:a#z0", "Desk", "strip", "12", "yes"], "{text}");
        assert_eq!(lines[2], ["fake:b#z0", "Case", "fan", "3", "no"], "{text}");
        match lights_list(&api, true).expect("the lights") {
            Output::Json(value) => assert_eq!(value, a_lights_answer()),
            other => panic!("{other:?}"),
        }
        assert_eq!(lights_list_text(&serde_json::json!({ "lights": [] })), "no lights\n");
    }

    // ---- effects and presets (#377 T3) ----

    /// The effect most of these tests are about.
    const INK: &str = "html:ink";

    /// `html:ink` as `GET /effects/{id}` answers it: two properties, one built-in preset and one
    /// of the user's.
    fn ink_detail() -> serde_json::Value {
        serde_json::json!({
            "schema": {
                "id": "html:ink",
                "name": "Ink",
                "properties": [
                    { "key": "speed", "label": "Speed",
                      "kind": { "type": "number", "min": 0.0, "max": 10.0, "step": 1.0, "default": 5.0 } },
                    { "key": "tint", "label": "Tint", "kind": { "type": "color", "default": "#112233" } }
                ],
                "presets": [ { "name": "Calm", "props": { "speed": { "type": "number", "value": 2.0 } } } ]
            },
            "favorite": false,
            "user_presets": [ { "name": "Night", "props": { "speed": { "type": "number", "value": 7.0 } } } ],
            "props": {}
        })
    }

    /// A daemon for the effects commands: it answers each (method, path) it was told about, the
    /// scenes list for the scenes route, and `404` for anything else, and records every request.
    /// What a fake route answers: a value, or a status and a body.
    type Reply = Result<serde_json::Value, (u16, serde_json::Value)>;

    struct FxApi {
        scenes: serde_json::Value,
        routes: Vec<(String, String, Reply)>,
        /// Runs when a request other than a read arrives, before it is answered.
        on_write: Option<Box<dyn Fn()>>,
        /// When set, a request other than a read fails with this error, after `on_write` ran.
        write_error: Option<Box<dyn Fn() -> CliError>>,
        asked: std::sync::Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl FxApi {
        fn new(scenes: serde_json::Value) -> Self {
            Self {
                scenes,
                routes: Vec::new(),
                on_write: None,
                write_error: None,
                asked: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn route(mut self, method: &str, path: &str, answer: serde_json::Value) -> Self {
            self.routes.push((method.to_string(), path.to_string(), Ok(answer)));
            self
        }

        fn refuse(mut self, method: &str, path: &str, status: u16, message: &str) -> Self {
            self.routes.push((
                method.to_string(),
                path.to_string(),
                Err((status, serde_json::json!({ "message": message }))),
            ));
            self
        }

        fn on_write(mut self, hook: impl Fn() + 'static) -> Self {
            self.on_write = Some(Box::new(hook));
            self
        }

        fn write_fails_with(mut self, make: impl Fn() -> CliError + 'static) -> Self {
            self.write_error = Some(Box::new(make));
            self
        }

        fn asked(&self) -> Vec<(String, String, Option<String>)> {
            self.asked.lock().unwrap().clone()
        }

        /// Every request that is not a read.
        fn writes(&self) -> Vec<(String, String, Option<String>)> {
            self.asked().into_iter().filter(|(method, _, _)| method != "GET").collect()
        }

        /// How many times a path was read.
        fn reads(&self, path: &str) -> usize {
            self.asked().iter().filter(|(method, at, _)| method == "GET" && at == path).count()
        }
    }

    impl Api for FxApi {
        fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, CliError> {
            self.asked
                .lock()
                .unwrap()
                .push((method.to_string(), path.to_string(), body.map(str::to_string)));
            if method != "GET" {
                if let Some(hook) = &self.on_write {
                    hook();
                }
                if let Some(make) = &self.write_error {
                    return Err(make());
                }
            }
            if method == "GET" && path == canvas_proto::api::routes::SCENES {
                return Ok(self.scenes.clone());
            }
            for (at_method, at_path, answer) in &self.routes {
                if at_method == method && at_path == path {
                    return match answer {
                        Ok(value) => Ok(value.clone()),
                        Err((status, body)) => Err(CliError::Api {
                            status: *status,
                            message: body.get("message").and_then(|m| m.as_str()).unwrap_or_default().to_string(),
                            body: Some(body.clone()),
                        }),
                    };
                }
            }
            // No stacks unless a test routes some: the effects commands read the list to see
            // whether a saved stack layers or masks an effect.
            if method == "GET" && path == canvas_proto::api::routes::STACKS {
                return Ok(serde_json::json!({ "stacks": [] }));
            }
            Err(CliError::Api {
                status: 404,
                message: format!("the fake daemon has no route for {method} {path}"),
                body: None,
            })
        }
    }

    /// A failure after which the caller cannot tell whether the daemon acted.
    fn unknown_outcome(label: &str) -> CliError {
        match label {
            "no answer" => CliError::NoAnswer("no answer".into()),
            _ => CliError::Other("connection reset".into()),
        }
    }

    fn ink_route() -> String {
        effect_route(canvas_proto::api::routes::EFFECT, INK)
    }

    fn ink_source_route() -> String {
        effect_route(canvas_proto::api::routes::EFFECT_SOURCE, INK)
    }

    fn ink_presets_route() -> String {
        effect_route(canvas_proto::api::routes::EFFECT_PRESETS, INK)
    }

    fn json_of(output: Output) -> serde_json::Value {
        match output {
            Output::Json(value) => value,
            other => panic!("{other:?}"),
        }
    }

    /// The record of run `agent-1` with extra top-level fields (`presets`, `effects`, `pending`).
    fn run_record_with(extra: serde_json::Value) -> String {
        let mut record: serde_json::Value =
            serde_json::from_str(&a_record(serde_json::json!([]), "gaming")).expect("the record is JSON");
        for (key, value) in extra.as_object().expect("an object") {
            record[key.as_str()] = value.clone();
        }
        record.to_string()
    }

    /// The record of a run that saved `Night` of `html:ink` and imported `html:ink`.
    fn record_with_ink_and_night() -> String {
        run_record_with(serde_json::json!({
            "presets": [{ "effect": "html:ink", "name": "Night" }],
            "effects": [{ "id": "html:ink" }],
        }))
    }

    /// Scenes where Gaming and Movie each play `effect`, and Sleep plays nothing.
    fn scenes_playing(effect: &str) -> serde_json::Value {
        serde_json::json!({
            "active": "gaming",
            "scenes": [
                { "id": "gaming", "name": "Gaming", "rev": 4, "groups": [
                    { "id": "desk", "name": "Desk", "members": ["fake:a#z0"],
                      "plays": { "type": "effect", "effect": effect, "props": {} } } ] },
                { "id": "movie", "name": "Movie", "rev": 7, "groups": [
                    { "id": "lamp", "name": "Lamp", "members": ["fake:b#z0"],
                      "plays": { "type": "effect", "effect": effect, "props": {} } } ] },
                { "id": "sleep", "name": "Sleep", "rev": 2, "groups": [
                    { "id": "bed", "name": "Bed", "members": [], "plays": { "type": "off" } } ] },
            ],
            "group_states": []
        })
    }

    fn save_args(props: &[&str], run: Option<&str>) -> PresetSaveArgs {
        PresetSaveArgs {
            effect: INK.into(),
            name: "Night".into(),
            props: props.iter().map(|p| p.to_string()).collect(),
            run: run.map(str::to_string),
        }
    }

    fn preset_delete_args(name: &str, run: &str, dry_run: bool) -> PresetDeleteArgs {
        PresetDeleteArgs { effect: INK.into(), name: name.into(), run: run.into(), dry_run }
    }

    fn effect_delete_args(run: &str, keep_presets: bool, dry_run: bool) -> EffectDeleteArgs {
        EffectDeleteArgs { effect: INK.into(), keep_presets, run: run.into(), dry_run }
    }

    fn import_args(file: &str, replace: bool, run: Option<&str>) -> ImportArgs {
        ImportArgs { file: file.into(), replace, run: run.map(str::to_string) }
    }

    /// The names of the files in a folder, sorted; empty when the folder is not there.
    fn file_names_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    /// What was on disk at the moment a write arrived: the record's text and the file names in
    /// one folder of the run.
    #[derive(Clone, Default)]
    struct Seen {
        record: String,
        files: Vec<String>,
    }

    /// A hook that notes the record and the files of a folder when a write arrives.
    fn seen_at_write(
        record: PathBuf,
        folder: PathBuf,
        seen: std::sync::Arc<std::sync::Mutex<Seen>>,
    ) -> impl Fn() + 'static {
        move || {
            *seen.lock().unwrap() = Seen {
                record: std::fs::read_to_string(&record).unwrap_or_default(),
                files: file_names_in(&folder),
            };
        }
    }

    fn seen_pending(seen: &Seen) -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(&seen.record)
            .map(|record| record["pending"].clone())
            .unwrap_or(serde_json::Value::Null)
    }

    fn run_folder(home: &TempHome, folder: &str) -> PathBuf {
        home.path().join("agent-runs").join("agent-1").join(folder)
    }

    #[test]
    fn the_effects_commands_are_at_the_spines_names() {
        use clap::CommandFactory;
        let root = Cli::command();
        for path in [
            canvas_proto::cli::EFFECTS_SOURCE,
            canvas_proto::cli::EFFECTS_DELETE,
            canvas_proto::cli::EFFECTS_PRESET_LIST,
            canvas_proto::cli::EFFECTS_PRESET_SAVE,
            canvas_proto::cli::EFFECTS_PRESET_DELETE,
        ] {
            let mut at = &root;
            for step in path {
                at = at.find_subcommand(step).unwrap_or_else(|| panic!("no command {step} on the path {path:?}"));
            }
        }
        let cli = Cli::try_parse_from([
            "canvas-rgb", "effects", "preset", "save", INK, "Night", "--prop", "speed=7", "--prop",
            "tint=#ff0000", "--run", "agent-1",
        ])
        .unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Preset(PresetCommand::Save(args))) => {
                assert_eq!((args.effect.as_str(), args.name.as_str()), (INK, "Night"));
                assert_eq!(args.props, ["speed=7", "tint=#ff0000"], "--prop repeats");
                assert_eq!(args.run.as_deref(), Some("agent-1"));
            }
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from(["canvas-rgb", "effects", "preset", "save", INK, "Night"]).unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Preset(PresetCommand::Save(args))) => {
                assert!(args.run.is_none() && args.props.is_empty(), "a save may leave --run and --prop off");
            }
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from([
            "canvas-rgb", "effects", "delete", INK, "--keep-presets", "--run", "agent-1", "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Delete(args)) => {
                assert_eq!((args.effect.as_str(), args.run.as_str()), (INK, "agent-1"));
                assert!(args.keep_presets && args.dry_run);
            }
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from(["canvas-rgb", "effects", "preset", "delete", INK, "Night", "--run", "agent-1"])
            .unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Preset(PresetCommand::Delete(args))) => {
                assert_eq!((args.name.as_str(), args.run.as_str(), args.dry_run), ("Night", "agent-1", false));
            }
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from(["canvas-rgb", "effects", "preset", "list", INK]).unwrap();
        assert!(matches!(cli.command, Command::Effects(EffectsCommand::Preset(PresetCommand::List(_)))));
        let cli = Cli::try_parse_from(["canvas-rgb", "effects", "source", INK, "--out", "ink.html"]).unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Source(args)) => assert_eq!(args.out.as_deref(), Some("ink.html")),
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from(["canvas-rgb", "effects", "import", "a.html", "--run", "agent-1"]).unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Import(args)) => {
                assert_eq!((args.run.as_deref(), args.replace), (Some("agent-1"), false));
            }
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from(["canvas-rgb", "effects", "check", "a.html", "--prop", "a=1", "--prop", "b=2"])
            .unwrap();
        match cli.command {
            Command::Effects(EffectsCommand::Check(args)) => assert_eq!(args.props, ["a=1", "b=2"]),
            other => panic!("{other:?}"),
        }
        // --run is required on both deletes: a run's guard cannot be left off by forgetting a flag.
        assert!(
            Cli::try_parse_from(["canvas-rgb", "effects", "delete", INK]).is_err(),
            "an effect delete names the run"
        );
        assert!(
            Cli::try_parse_from(["canvas-rgb", "effects", "preset", "delete", INK, "Night"]).is_err(),
            "a preset delete names the run"
        );
    }

    #[test]
    fn prop_arguments_split_at_the_first_equals_and_a_bad_one_is_one_plain_sentence() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<String>>();
        assert_eq!(
            parse_prop_args(&args(&["tint=#ff0000", "label=a=b"])).unwrap(),
            vec![("tint".to_string(), "#ff0000".to_string()), ("label".to_string(), "a=b".to_string())],
            "a value keeps its own ="
        );
        assert!(parse_prop_args(&[]).unwrap().is_empty());
        let missing = parse_prop_args(&args(&["speed"])).unwrap_err();
        assert!(missing.contains("is not a key=value pair"), "a missing = is refused: {missing}");
        let twice = parse_prop_args(&args(&["speed=1", "tint=#ffffff", "speed=2"])).unwrap_err();
        assert_eq!(twice, "--prop speed is given twice; give each property once", "a repeated key is refused");
        let nameless = parse_prop_args(&args(&["=5"])).unwrap_err();
        assert!(nameless.contains("no property name"), "{nameless}");
    }

    #[test]
    fn a_path_segment_is_percent_encoded_so_a_name_cannot_change_the_route() {
        assert_eq!(path_segment("html:my effect"), "html%3Amy%20effect");
        assert_eq!(path_segment("Night / 2"), "Night%20%2F%202");
        assert_eq!(path_segment("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(path_segment("é"), "%C3%A9", "each UTF-8 byte is encoded");
        assert_eq!(
            preset_route("html:ink", "Night / 2"),
            "/api/v1/effects/html%3Aink/presets/Night%20%2F%202"
        );
        assert_eq!(ink_presets_route(), "/api/v1/effects/html%3Aink/presets");
    }

    #[test]
    fn a_wrong_property_is_refused_before_anything_is_read_or_written() {
        let home = TempHome::new("fx-badprop");
        let fresh = || FxApi::new(run_scenes()).route("GET", &ink_route(), ink_detail());

        let api = fresh();
        let error = preset_save(&api, &save_args(&["speed"], Some("agent-1")), false).unwrap_err();
        assert!(error.to_string().contains("is not a key=value pair"), "{error}");
        assert!(api.asked().is_empty(), "a pair with no = sends nothing, not even a read: {:?}", api.asked());

        let api = fresh();
        let error = preset_save(&api, &save_args(&["speed=1", "speed=2"], Some("agent-1")), false).unwrap_err();
        assert!(error.to_string().contains("--prop speed is given twice"), "{error}");
        assert!(api.asked().is_empty(), "a repeated key sends nothing: {:?}", api.asked());

        for (pair, needle) in [
            ("bogus=1", "bogus is not a property of this effect"),
            ("speed=99", "--prop speed"),
            ("tint=blue", "--prop tint"),
        ] {
            let api = fresh();
            let error = preset_save(&api, &save_args(&[pair], Some("agent-1")), false).unwrap_err();
            assert!(error.to_string().contains(needle), "{pair}: {error}");
            assert_eq!(
                api.asked(),
                vec![("GET".to_string(), ink_route(), None)],
                "{pair}: only the effect was read; nothing was written"
            );
        }
        assert!(home.files().is_empty(), "no record was started: {:?}", home.files());
    }

    /// NIT 9 of the #412 review: a schema kind this CLI cannot read is its own sentence, not "is
    /// not a property of this effect".
    #[test]
    fn a_schema_kind_this_cli_cannot_read_is_not_reported_as_a_missing_property() {
        let home = TempHome::new("fx-badkind");
        let mut detail = ink_detail();
        detail["schema"]["properties"][0]["kind"] = serde_json::json!({ "type": "sparkle" });
        let api = FxApi::new(run_scenes()).route("GET", &ink_route(), detail);
        let error = preset_save(&api, &save_args(&["speed=2"], Some("agent-1")), false).unwrap_err();
        assert!(
            error.to_string().contains("carries a kind this command cannot read"),
            "the message names the schema, not the key: {error}"
        );
        assert!(!error.to_string().contains("is not a property"), "{error}");
        assert!(api.writes().is_empty(), "{:?}", api.asked());
        assert!(home.files().is_empty(), "nothing was written: {:?}", home.files());
    }

    #[test]
    fn preset_save_posts_the_typed_props_and_the_record_holds_it_pending_while_the_request_is_on_the_way() {
        let home = TempHome::new("fx-save");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_route(), ink_detail())
            .route("POST", &ink_presets_route(), serde_json::json!({ "user_presets": [] }))
            .on_write(seen_at_write(record_at(&home), run_folder(&home, "presets"), seen.clone()));
        let text = text_of(
            preset_save(&api, &save_args(&["speed=7", "tint=#ff0000"], Some("agent-1")), false).unwrap(),
        );
        assert_eq!(text, "saved preset \"Night\" of html:ink\n");

        let writes = api.writes();
        assert_eq!(writes.len(), 1, "one write: {writes:?}");
        assert_eq!((writes[0].0.as_str(), writes[0].1.as_str()), ("POST", ink_presets_route().as_str()));
        let body: serde_json::Value = serde_json::from_str(writes[0].2.as_deref().expect("a body")).unwrap();
        assert_eq!(body["name"], "Night");
        assert_eq!(body["props"]["speed"], serde_json::json!({ "type": "number", "value": 7.0 }));
        assert_eq!(body["props"]["tint"], serde_json::json!({ "type": "color", "value": "#ff0000" }));

        assert_eq!(
            seen_pending(&seen.lock().unwrap()),
            serde_json::json!([{ "op": "preset-save", "scene": "html:ink/Night" }]),
            "the record held the save as pending when the request went out"
        );
        let record = record_json(&home);
        assert_eq!(record["presets"], serde_json::json!([{ "effect": "html:ink", "name": "Night" }]), "{record}");
        assert!(record.get("pending").is_none(), "the pending save is cleared: {record}");
        assert_eq!(record["prior_active"], "gaming");
        assert_eq!(record["before"], run_scenes(), "the list at the run's first write");
    }

    #[test]
    fn preset_save_without_a_run_writes_no_record_and_reads_no_scenes() {
        let home = TempHome::new("fx-save-norun");
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_route(), ink_detail())
            .route("POST", &ink_presets_route(), serde_json::json!({ "user_presets": [] }));
        let document = json_of(preset_save(&api, &save_args(&["speed=7"], None), true).unwrap());
        assert_eq!(document["saved"], true);
        assert_eq!(document["run"], serde_json::Value::Null);
        assert_eq!((document["effect"].as_str(), document["preset"].as_str()), (Some("html:ink"), Some("Night")));
        assert_eq!(api.reads(canvas_proto::api::routes::SCENES), 0, "no run, no list of scenes");
        assert!(home.files().is_empty(), "no record, no copy: {:?}", home.files());
    }

    #[test]
    fn a_bad_run_id_is_refused_by_every_effects_command_before_anything_is_read_or_sent() {
        let home = TempHome::new("fx-badrun");
        for bad in ["../x", "", "con", "a b"] {
            let api = FxApi::new(run_scenes());
            let errors = [
                preset_save(&api, &save_args(&["speed=1"], Some(bad)), false).unwrap_err(),
                preset_delete(&api, &preset_delete_args("Night", bad, false), false).unwrap_err(),
                effects_delete(&api, &effect_delete_args(bad, false, false), false).unwrap_err(),
                // A file that is not there: the run id is checked first, so the error is about the id.
                effects_import(&api, &import_args("missing.html", false, Some(bad)), false).unwrap_err(),
            ];
            for error in errors {
                assert!(error.to_string().contains("is not a run id"), "{bad:?}: {error}");
            }
            assert!(api.asked().is_empty(), "{bad:?}: the fake was never called: {:?}", api.asked());
        }
        assert!(home.files().is_empty(), "nothing was written: {:?}", home.files());
    }

    #[test]
    fn a_preset_the_run_did_not_save_is_refused_and_nothing_is_sent_or_copied() {
        let home = TempHome::new("fx-pdel-notmine");
        home.write("agent-runs/agent-1.json", &run_record_with(serde_json::json!({})));
        let before = home.files();
        let api = FxApi::new(run_scenes()).route("GET", &ink_route(), ink_detail());
        let error = preset_delete(&api, &preset_delete_args("Night", "agent-1", false), false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the preset \"Night\" of html:ink was not made by run agent-1; nothing changed",
            "a preset the record does not list is refused"
        );
        assert!(api.writes().is_empty(), "no delete was sent: {:?}", api.asked());
        assert_eq!(home.files(), before, "no copy was taken");

        // The same refusal names the writes the run holds with no known outcome.
        home.write(
            "agent-runs/agent-1.json",
            &run_record_with(serde_json::json!({ "pending": [{ "op": "preset-save", "scene": "html:ink/Night" }] })),
        );
        let error = preset_delete(&api, &preset_delete_args("night", "agent-1", false), false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the preset \"Night\" of html:ink was not made by run agent-1; nothing changed. Run agent-1 holds writes with no known outcome: preset-save html:ink/Night."
        );
        assert!(api.writes().is_empty());

        // A preset that is not in the library at all is its own sentence.
        let error = preset_delete(&api, &preset_delete_args("Nope", "agent-1", false), false).unwrap_err();
        assert_eq!(error.to_string(), "html:ink has no preset of yours named \"Nope\"; nothing changed");
        // A missing record refuses the same way the scenes commands do.
        let error = preset_delete(&api, &preset_delete_args("Night", "agent-9", false), false).unwrap_err();
        assert_eq!(error.to_string(), "no record of run agent-9; nothing changed");
    }

    #[test]
    fn a_preset_delete_copies_the_preset_whole_first_then_deletes_it_and_forgets_it() {
        let home = TempHome::new("fx-pdel");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_route(), ink_detail())
            .route("DELETE", &preset_route("html:ink", "Night"), serde_json::json!({ "user_presets": [] }))
            .on_write(seen_at_write(record_at(&home), run_folder(&home, "presets"), seen.clone()));
        // The name matches without regard to case, and the stored name goes in the route.
        let text = text_of(preset_delete(&api, &preset_delete_args("night", "agent-1", false), false).unwrap());
        assert_eq!(text, "deleted preset \"Night\" of html:ink\n");
        let writes = api.writes();
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert_eq!(
            (writes[0].0.as_str(), writes[0].1.as_str()),
            ("DELETE", "/api/v1/effects/html%3Aink/presets/Night")
        );

        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.files.len(), 1, "the copy was on disk when the delete went out: {:?}", seen.files);
        assert_eq!(
            seen_pending(&seen),
            serde_json::json!([{ "op": "preset-delete", "scene": "html:ink/Night" }]),
            "the record held the delete as pending when the request went out"
        );
        let copy: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(run_folder(&home, "presets").join(&seen.files[0])).expect("the copy is there"),
        )
        .unwrap();
        assert_eq!(copy["effect"], "html:ink");
        assert_eq!(copy["preset"], ink_detail()["user_presets"][0], "the whole preset, props and all");

        let record = record_json(&home);
        assert!(record.get("presets").is_none(), "the preset is forgotten: {record}");
        assert!(record.get("pending").is_none(), "{record}");
        assert_eq!(record["effects"], serde_json::json!([{ "id": "html:ink" }]), "the effect is untouched");
    }

    #[test]
    fn a_preset_copy_that_cannot_be_written_sends_nothing() {
        let home = TempHome::new("fx-pdel-nocopy");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        // A file where the copy's folder must be.
        home.write("agent-runs/agent-1/presets", "in the way");
        let record_before = std::fs::read_to_string(record_at(&home)).unwrap();
        let api = FxApi::new(run_scenes()).route("GET", &ink_route(), ink_detail());
        let error = preset_delete(&api, &preset_delete_args("Night", "agent-1", false), false).unwrap_err();
        assert!(error.to_string().contains("could not save the preset"), "{error}");
        assert!(api.writes().is_empty(), "a copy that cannot be written sends nothing: {:?}", api.asked());
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record_before, "no pending entry");
    }

    #[test]
    fn a_dry_run_of_a_delete_writes_nothing_and_sends_no_write() {
        let home = TempHome::new("fx-dry");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let before = home.files();
        let record_before = std::fs::read_to_string(record_at(&home)).unwrap();

        let api = FxApi::new(run_scenes()).route("GET", &ink_route(), ink_detail());
        let text = text_of(preset_delete(&api, &preset_delete_args("Night", "agent-1", true), false).unwrap());
        assert_eq!(text, "would delete preset \"Night\" of html:ink\n");
        let document = json_of(preset_delete(&api, &preset_delete_args("Night", "agent-1", true), true).unwrap());
        assert_eq!(document["saved"], false);
        assert!(api.writes().is_empty(), "a dry run sends no write: {:?}", api.asked());

        let api = FxApi::new(run_scenes());
        let text = text_of(effects_delete(&api, &effect_delete_args("agent-1", false, true), false).unwrap());
        assert_eq!(text, "would delete html:ink\n");
        assert_eq!(
            api.asked(),
            vec![
                ("GET".to_string(), canvas_proto::api::routes::SCENES.to_string(), None),
                ("GET".to_string(), canvas_proto::api::routes::STACKS.to_string(), None),
            ],
            "a dry run only reads the scenes and the stacks: no source, no copy, no delete"
        );
        assert_eq!(home.files(), before, "no record rewrite, no copy: the whole home is as it was");
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record_before);
    }

    #[test]
    fn an_effect_the_run_did_not_import_is_refused_without_asking_the_daemon_anything() {
        let home = TempHome::new("fx-edel-notmine");
        home.write("agent-runs/agent-1.json", &run_record_with(serde_json::json!({})));
        let before = home.files();
        let api = FxApi::new(run_scenes());
        for effect in ["html:ink", "waves"] {
            let args = EffectDeleteArgs { effect: effect.into(), keep_presets: false, run: "agent-1".into(), dry_run: false };
            let error = effects_delete(&api, &args, false).unwrap_err();
            assert_eq!(error.to_string(), format!("{effect} was not made by run agent-1; nothing changed"));
        }
        assert!(api.asked().is_empty(), "not made by the run: no request at all: {:?}", api.asked());
        assert_eq!(home.files(), before);
    }

    #[test]
    fn an_effect_a_scene_plays_is_never_deleted_and_the_scenes_are_named() {
        let home = TempHome::new("fx-edel-played");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let before = home.files();
        for keep_presets in [false, true] {
            let api = FxApi::new(scenes_playing("html:ink"));
            let error = effects_delete(&api, &effect_delete_args("agent-1", keep_presets, false), false).unwrap_err();
            assert_eq!(
                error.to_string(),
                "html:ink is played by the scenes \"Gaming\", \"Movie\"; nothing deleted. Move those groups to another effect first.",
                "no flag lets a played effect go"
            );
            assert_eq!(
                api.asked(),
                vec![("GET".to_string(), canvas_proto::api::routes::SCENES.to_string(), None)],
                "no source read, no delete"
            );
        }
        // The id matches without regard to case, and one scene is one scene.
        let one = serde_json::json!({ "active": "x", "scenes": [
            { "id": "x", "name": "Solo", "rev": 1, "groups": [
                { "id": "g", "name": "G", "members": [], "plays": { "type": "effect", "effect": "HTML:INK", "props": {} } } ] } ] });
        let api = FxApi::new(one);
        let error = effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap_err();
        assert!(error.to_string().contains("is played by the scene \"Solo\";"), "{error}");
        assert_eq!(home.files(), before, "no copy was taken");
    }

    /// SHOULD-FIX 2 of the #412 review: a saved stack that uses the effect as a layer, or only as
    /// a layer's mask, blocks the delete too, and the stacks are named the way the scenes are.
    #[test]
    fn an_effect_a_stack_layers_or_masks_is_never_deleted_and_the_stacks_are_named() {
        let home = TempHome::new("fx-edel-stacked");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let before = home.files();
        let listed = || {
            serde_json::json!({ "stacks": [
                { "id": "stack:Sunset", "name": "Sunset", "layers": 1, "html_layers": 1, "missing": [] } ] })
        };
        let sunset = effect_route(canvas_proto::api::routes::STACK, "stack:Sunset");
        // One layer names the effect outright; one names it only as its mask, in another case.
        for detail in [
            serde_json::json!({ "stack": { "id": "stack:Sunset", "layers": [
                { "id": "l1", "name": "Ink", "effect": INK } ] }, "layers": [] }),
            serde_json::json!({ "stack": { "id": "stack:Sunset", "layers": [
                { "id": "l1", "name": "Heat", "effect": "fire",
                  "mask": { "effect": "HTML:INK", "amount": 100 } } ] }, "layers": [] }),
        ] {
            let api = FxApi::new(run_scenes())
                .route("GET", canvas_proto::api::routes::STACKS, listed())
                .route("GET", &sunset, detail);
            let error = effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap_err();
            assert_eq!(
                error.to_string(),
                "html:ink is a layer or a mask of the stack \"Sunset\"; nothing deleted. Change those layers to another effect first.",
                "a stack that uses the effect is named, and no flag forces it"
            );
            assert!(api.writes().is_empty(), "no DELETE is sent: {:?}", api.asked());
            assert_eq!(api.reads(&ink_source_route()), 0, "the effect is not even copied");
            assert_eq!(home.files(), before, "no copy was taken");
        }
        // Two stacks, both naming it: the plural, and both names.
        let listed = || {
            serde_json::json!({ "stacks": [
                { "id": "stack:Sunset", "name": "Sunset", "layers": 1, "html_layers": 0, "missing": [] },
                { "id": "stack:Dusk", "name": "Dusk", "layers": 2, "html_layers": 1, "missing": [] } ] })
        };
        let api = FxApi::new(run_scenes())
            .route("GET", canvas_proto::api::routes::STACKS, listed())
            .route(
                "GET",
                &sunset,
                serde_json::json!({ "stack": { "id": "stack:Sunset", "layers": [
                    { "id": "l1", "effect": INK } ] }, "layers": [] }),
            )
            .route(
                "GET",
                &effect_route(canvas_proto::api::routes::STACK, "stack:Dusk"),
                serde_json::json!({ "stack": { "id": "stack:Dusk", "layers": [
                    { "id": "l2", "effect": "fire" },
                    { "id": "l1", "effect": INK } ] }, "layers": [] }),
            );
        let error = effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "html:ink is a layer or a mask of the stacks \"Sunset\", \"Dusk\"; nothing deleted. Change those layers to another effect first."
        );
        assert!(api.writes().is_empty(), "{:?}", api.asked());
        // A stack that names another effect leaves the delete alone: it goes through to the copy.
        let api = FxApi::new(run_scenes())
            .route(
                "GET",
                canvas_proto::api::routes::STACKS,
                serde_json::json!({ "stacks": [
                    { "id": "stack:Sunset", "name": "Sunset", "layers": 1, "html_layers": 0, "missing": [] } ] }),
            )
            .route(
                "GET",
                &sunset,
                serde_json::json!({ "stack": { "id": "stack:Sunset", "layers": [
                    { "id": "l1", "effect": "fire" } ] }, "layers": [] }),
            )
            .route("GET", &ink_source_route(), serde_json::json!({ "source": "<html>ink</html>", "file_name": "ink.html" }))
            .route("GET", &ink_route(), ink_detail())
            .route("DELETE", &ink_route(), serde_json::json!({ "ok": true, "id": "html:ink", "presets_removed": 1 }));
        let text = text_of(effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap());
        assert_eq!(text, "deleted html:ink (1 preset removed)\n", "another effect's stack is not in the way");
    }

    #[test]
    fn an_effect_delete_copies_the_source_and_the_presets_first_then_deletes_and_forgets_them() {
        let home = TempHome::new("fx-edel");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_source_route(), serde_json::json!({ "source": "<html>ink</html>", "file_name": "ink.html" }))
            .route("GET", &ink_route(), ink_detail())
            .route(
                "DELETE",
                &ink_route(),
                serde_json::json!({ "ok": true, "id": "html:ink", "presets_removed": 1 }),
            )
            .on_write(seen_at_write(record_at(&home), run_folder(&home, "effects"), seen.clone()));
        let text = text_of(effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap());
        assert_eq!(text, "deleted html:ink (1 preset removed)\n");
        let writes = api.writes();
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert_eq!((writes[0].0.as_str(), writes[0].1.as_str()), ("DELETE", "/api/v1/effects/html%3Aink"));

        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.files.len(), 1, "the copy was on disk when the delete went out: {:?}", seen.files);
        assert_eq!(
            seen_pending(&seen),
            serde_json::json!([{ "op": "effect-delete", "scene": "html:ink" }]),
            "the record held the delete as pending when the request went out"
        );
        let copy: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(run_folder(&home, "effects").join(&seen.files[0])).expect("the copy is there"),
        )
        .unwrap();
        assert_eq!(copy["id"], "html:ink");
        assert_eq!(copy["source"], "<html>ink</html>");
        assert_eq!(copy["file_name"], "ink.html");
        assert_eq!(copy["user_presets"], ink_detail()["user_presets"], "the presets go with the effect, so they are copied");

        let record = record_json(&home);
        assert!(record.get("effects").is_none(), "the effect is forgotten: {record}");
        assert!(record.get("presets").is_none(), "its presets went with it: {record}");
        assert!(record.get("pending").is_none(), "{record}");

        // --keep-presets: the query, and the record keeps the presets the library kept.
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_source_route(), serde_json::json!({ "source": "<html>ink</html>", "file_name": "ink.html" }))
            .route("GET", &ink_route(), ink_detail())
            .route(
                "DELETE",
                &format!("{}?keep_presets=true", ink_route()),
                serde_json::json!({ "ok": true, "id": "html:ink", "presets_removed": 0 }),
            );
        let text = text_of(effects_delete(&api, &effect_delete_args("agent-1", true, false), false).unwrap());
        assert_eq!(text, "deleted html:ink\n");
        assert_eq!(api.writes()[0].1, "/api/v1/effects/html%3Aink?keep_presets=true");
        let record = record_json(&home);
        assert_eq!(record["presets"], serde_json::json!([{ "effect": "html:ink", "name": "Night" }]), "{record}");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let document = json_of(effects_delete(&api, &effect_delete_args("agent-1", true, false), true).unwrap());
        assert_eq!(document["deleted"], "html:ink");
        assert_eq!((document["saved"].as_bool(), document["run"].as_str()), (Some(true), Some("agent-1")));
    }

    #[test]
    fn an_effect_copy_that_cannot_be_written_or_has_no_source_sends_nothing() {
        let home = TempHome::new("fx-edel-nocopy");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        home.write("agent-runs/agent-1/effects", "in the way");
        let record_before = std::fs::read_to_string(record_at(&home)).unwrap();
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_source_route(), serde_json::json!({ "source": "<html>ink</html>", "file_name": "ink.html" }))
            .route("GET", &ink_route(), ink_detail());
        let error = effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap_err();
        assert!(error.to_string().contains("could not save the effect"), "{error}");
        assert!(api.writes().is_empty(), "a copy that cannot be written sends nothing: {:?}", api.asked());
        assert_eq!(std::fs::read_to_string(record_at(&home)).unwrap(), record_before, "no pending entry");

        // An answer with no source is no copy either.
        let api = FxApi::new(run_scenes()).route("GET", &ink_source_route(), serde_json::json!({ "file_name": "ink.html" }));
        let error = effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap_err();
        assert!(error.to_string().contains("the daemon sent no source for html:ink"), "{error}");
        assert!(api.writes().is_empty(), "{:?}", api.asked());
    }

    /// SHOULD-FIX 5 of the #412 review: a missing or non-array `user_presets` is a copy that is not
    /// the effect's whole content, so it is an error and the delete never goes out.
    #[test]
    fn an_effect_answer_without_a_readable_preset_list_is_refused_and_nothing_is_deleted() {
        let home = TempHome::new("fx-edel-nopresets");
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let record_before = std::fs::read_to_string(record_at(&home)).unwrap();
        let mut missing = ink_detail();
        missing.as_object_mut().expect("an object").remove("user_presets");
        for answer in [
            missing,
            serde_json::json!({ "user_presets": null }),
            serde_json::json!({ "user_presets": { "Night": 1 } }),
        ] {
            let api = FxApi::new(run_scenes())
                .route("GET", &ink_source_route(), serde_json::json!({ "source": "<html>ink</html>", "file_name": "ink.html" }))
                .route("GET", &ink_route(), answer.clone());
            let error = effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap_err();
            assert_eq!(
                error.to_string(),
                "could not read the effect's presets; nothing deleted",
                "a delete never drops presets it could not copy: {answer}"
            );
            assert!(api.writes().is_empty(), "the DELETE is never sent: {:?}", api.asked());
            assert_eq!(
                std::fs::read_to_string(record_at(&home)).unwrap(),
                record_before,
                "the record is untouched: no pending entry"
            );
            assert!(!run_folder(&home, "effects").exists(), "no half copy is left behind");
        }
        // The list command refuses the same answer instead of showing an effect with no presets.
        let api = FxApi::new(run_scenes()).route("GET", &ink_route(), serde_json::json!({ "schema": { "id": INK } }));
        let error = preset_list(&api, &PresetListArgs { effect: INK.into() }, false).unwrap_err();
        assert_eq!(error.to_string(), "could not read the effect's presets; nothing changed");
    }

    #[test]
    fn an_import_with_a_run_is_pending_while_it_is_sent_and_the_effect_is_noted() {
        let home = TempHome::new("fx-import");
        let file = home.write("ink.html", "<html>ink</html>");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
        let answer = serde_json::json!({ "effect": { "id": "html:ink", "name": "Ink" }, "replaced": false });
        let api = FxApi::new(run_scenes())
            .route("POST", canvas_proto::api::routes::EFFECTS_IMPORT, answer)
            .on_write(seen_at_write(record_at(&home), run_folder(&home, "effects"), seen.clone()));
        let text = text_of(effects_import(&api, &import_args(&file, false, Some("agent-1")), false).unwrap());
        assert_eq!(text, "imported html:ink (Ink)\n");
        let writes = api.writes();
        assert_eq!(writes.len(), 1, "{writes:?}");
        let body: serde_json::Value = serde_json::from_str(writes[0].2.as_deref().expect("a body")).unwrap();
        assert_eq!(body, serde_json::json!({ "file_name": "ink.html", "html": "<html>ink</html>", "replace": false }));
        assert_eq!(
            seen_pending(&seen.lock().unwrap()),
            serde_json::json!([{ "op": "import", "scene": "ink.html" }]),
            "the record held the import as pending when the request went out"
        );
        let record = record_json(&home);
        assert_eq!(record["effects"], serde_json::json!([{ "id": "html:ink" }]), "{record}");
        assert!(record.get("pending").is_none(), "{record}");
        assert_eq!(record["before"], run_scenes());
        assert!(seen.lock().unwrap().files.is_empty(), "a plain import copies nothing");
    }

    #[test]
    fn an_import_without_a_run_writes_no_record_and_answers_one_document() {
        let home = TempHome::new("fx-import-norun");
        let file = home.write("ink.html", "<html>ink</html>");
        let before = home.files();
        let answer = serde_json::json!({ "effect": { "id": "html:ink", "name": "Ink" }, "replaced": true });
        let api = FxApi::new(run_scenes()).route("POST", canvas_proto::api::routes::EFFECTS_IMPORT, answer);
        let text = text_of(effects_import(&api, &import_args(&file, true, None), false).unwrap());
        assert_eq!(text, "imported html:ink (Ink); it replaced the file of the same name\n");
        let document = json_of(effects_import(&api, &import_args(&file, true, None), true).unwrap());
        assert_eq!(document["saved"], true);
        assert_eq!(document["run"], serde_json::Value::Null);
        assert_eq!(document["effect"]["id"], "html:ink");
        assert_eq!(document["replaced"], true);
        assert_eq!(api.reads(canvas_proto::api::routes::SCENES), 0, "no run, no list of scenes");
        assert_eq!(home.files(), before, "no record, no copy");
    }

    #[test]
    fn an_import_file_over_the_cap_is_refused_in_one_sentence_before_any_request() {
        let home = TempHome::new("fx-import-big");
        let big = home.write("big.html", &"x".repeat(canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES + 1));
        let api = FxApi::new(run_scenes());
        let error = effects_import(&api, &import_args(&big, false, Some("agent-1")), false).unwrap_err();
        assert!(error.to_string().contains("is over"), "{error}");
        assert!(!error.to_string().contains('\n'), "one sentence: {error}");
        assert!(api.asked().is_empty(), "read before any request: {:?}", api.asked());
        let error = effects_import(&api, &import_args("/no/such/file.html", false, None), false).unwrap_err();
        assert!(error.to_string().contains("could not read"), "{error}");
        assert!(api.asked().is_empty());
        let exact = home.write("exact.html", &"x".repeat(canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES));
        let answer = serde_json::json!({ "effect": { "id": "html:exact", "name": "Exact" }, "replaced": false });
        let api = FxApi::new(run_scenes()).route("POST", canvas_proto::api::routes::EFFECTS_IMPORT, answer);
        effects_import(&api, &import_args(&exact, false, None), false).expect("a file at the cap is accepted");
    }

    #[test]
    fn a_replace_with_a_run_overwrites_only_an_effect_the_run_imported_and_copies_it_first() {
        let home = TempHome::new("fx-replace");
        let file = home.write("ink.html", "<html>new</html>");
        let listed = serde_json::json!({ "effects": [{ "id": "html:ink", "name": "Ink" }], "active": null, "failed": [] });

        // The effect is in the library and the run did not import it: refused, nothing copied.
        home.write("agent-runs/agent-1.json", &run_record_with(serde_json::json!({})));
        let before = home.files();
        let api = FxApi::new(run_scenes()).route("GET", canvas_proto::api::routes::EFFECTS, listed.clone());
        let error = effects_import(&api, &import_args(&file, true, Some("agent-1")), false).unwrap_err();
        assert_eq!(error.to_string(), "html:ink was not made by run agent-1; nothing changed");
        assert!(api.writes().is_empty(), "{:?}", api.asked());
        assert_eq!(home.files(), before, "no copy");

        // A run with no record is refused too.
        let error = effects_import(&api, &import_args(&file, true, Some("agent-8")), false).unwrap_err();
        assert_eq!(error.to_string(), "html:ink was not made by run agent-8; nothing changed");
        assert!(api.writes().is_empty());

        // The run imported it: its source is copied first, then the import goes out with replace.
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
        let answer = serde_json::json!({ "effect": { "id": "html:ink", "name": "Ink" }, "replaced": true });
        let api = FxApi::new(run_scenes())
            .route("GET", canvas_proto::api::routes::EFFECTS, listed.clone())
            .route("GET", &ink_source_route(), serde_json::json!({ "source": "<html>old</html>", "file_name": "ink.html" }))
            .route("POST", canvas_proto::api::routes::EFFECTS_IMPORT, answer.clone())
            .on_write(seen_at_write(record_at(&home), run_folder(&home, "effects"), seen.clone()));
        effects_import(&api, &import_args(&file, true, Some("agent-1")), false).unwrap();
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.files.len(), 1, "the old source was on disk when the import went out: {:?}", seen.files);
        let copy: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(run_folder(&home, "effects").join(&seen.files[0])).unwrap(),
        )
        .unwrap();
        assert_eq!(copy["source"], "<html>old</html>");
        let body: serde_json::Value = serde_json::from_str(api.writes()[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(body["replace"], true);

        // A copy that cannot be written sends nothing.
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let api = FxApi::new(run_scenes())
            .route("GET", canvas_proto::api::routes::EFFECTS, listed)
            .route("GET", &ink_source_route(), serde_json::json!({ "source": "<html>old</html>", "file_name": "ink.html" }))
            .route("POST", canvas_proto::api::routes::EFFECTS_IMPORT, answer);
        std::fs::remove_dir_all(run_folder(&home, "effects")).unwrap();
        home.write("agent-runs/agent-1/effects", "in the way");
        let error = effects_import(&api, &import_args(&file, true, Some("agent-1")), false).unwrap_err();
        assert!(error.to_string().contains("could not save the effect"), "{error}");
        assert!(api.writes().is_empty(), "a copy that cannot be written sends nothing: {:?}", api.asked());
    }

    #[test]
    fn a_replace_of_a_name_the_library_does_not_have_is_a_plain_create() {
        let home = TempHome::new("fx-replace-new");
        let file = home.write("fresh.html", "<html>fresh</html>");
        let answer = serde_json::json!({ "effect": { "id": "html:fresh", "name": "Fresh" }, "replaced": false });
        let api = FxApi::new(run_scenes())
            .route("GET", canvas_proto::api::routes::EFFECTS, serde_json::json!({ "effects": [], "active": null, "failed": [] }))
            .route("POST", canvas_proto::api::routes::EFFECTS_IMPORT, answer);
        effects_import(&api, &import_args(&file, true, Some("agent-1")), false).unwrap();
        assert_eq!(api.writes().len(), 1, "{:?}", api.asked());
        assert!(!run_folder(&home, "effects").exists(), "nothing existed, so nothing was copied");
        assert_eq!(record_json(&home)["effects"], serde_json::json!([{ "id": "html:fresh" }]));
    }

    #[test]
    fn a_write_the_daemon_may_have_seen_keeps_its_pending_entry_and_a_certain_refusal_clears_it() {
        let home = TempHome::new("fx-pending");
        let file = home.write("ink.html", "<html>ink</html>");
        let ink_save = || save_args(&["speed=7"], Some("agent-1"));
        // An error the transport raised mid-request, and a request that got no answer: either way
        // the daemon may have acted.
        for label in ["other", "no answer"] {
            home.write("agent-runs/agent-1.json", &record_with_ink_and_night());

            let api = FxApi::new(run_scenes()).route("GET", &ink_route(), ink_detail()).write_fails_with(move || unknown_outcome(label));
            preset_save(&api, &ink_save(), false).unwrap_err();
            assert_eq!(
                record_json(&home)["pending"],
                serde_json::json!([{ "op": "preset-save", "scene": "html:ink/Night" }]),
                "{label}: a save that may have landed stays pending"
            );

            home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
            let api = FxApi::new(run_scenes()).route("GET", &ink_route(), ink_detail()).write_fails_with(move || unknown_outcome(label));
            preset_delete(&api, &preset_delete_args("Night", "agent-1", false), false).unwrap_err();
            let record = record_json(&home);
            assert_eq!(
                record["pending"],
                serde_json::json!([{ "op": "preset-delete", "scene": "html:ink/Night" }]),
                "{label}: a delete that may have landed stays pending"
            );
            assert_eq!(record["presets"].as_array().map(|p| p.len()), Some(1), "{label}: the preset is still listed");

            home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
            let api = FxApi::new(run_scenes())
                .route("GET", &ink_source_route(), serde_json::json!({ "source": "s", "file_name": "ink.html" }))
                .route("GET", &ink_route(), ink_detail())
                .write_fails_with(move || unknown_outcome(label));
            effects_delete(&api, &effect_delete_args("agent-1", false, false), false).unwrap_err();
            assert_eq!(
                record_json(&home)["pending"],
                serde_json::json!([{ "op": "effect-delete", "scene": "html:ink" }]),
                "{label}: an effect delete that may have landed stays pending"
            );

            home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
            let api = FxApi::new(run_scenes()).write_fails_with(move || unknown_outcome(label));
            effects_import(&api, &import_args(&file, false, Some("agent-1")), false).unwrap_err();
            assert_eq!(
                record_json(&home)["pending"],
                serde_json::json!([{ "op": "import", "scene": "ink.html" }]),
                "{label}: an import that may have landed stays pending"
            );
        }

        // A refusal the daemon made, certainly, leaves nothing pending, and the lists stay as they were.
        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_route(), ink_detail())
            .refuse("POST", &ink_presets_route(), 409, "a preset of that name exists");
        let error = preset_save(&api, &ink_save(), false).unwrap_err();
        assert!(error.to_string().contains("409"), "{error}");
        assert!(record_json(&home).get("pending").is_none(), "a 409 is a certain refusal");

        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let api = FxApi::new(run_scenes())
            .route("GET", &ink_route(), ink_detail())
            .refuse("DELETE", &preset_route("html:ink", "Night"), 404, "no such preset");
        preset_delete(&api, &preset_delete_args("Night", "agent-1", false), false).unwrap_err();
        let record = record_json(&home);
        assert!(record.get("pending").is_none(), "{record}");
        assert_eq!(record["presets"].as_array().map(|p| p.len()), Some(1));

        home.write("agent-runs/agent-1.json", &record_with_ink_and_night());
        let api = FxApi::new(run_scenes())
            .route("POST", canvas_proto::api::routes::EFFECTS_IMPORT, serde_json::json!({ "effect": {}, "replaced": false }));
        let error = effects_import(&api, &import_args(&file, false, Some("agent-1")), false).unwrap_err();
        assert!(error.to_string().contains("named no effect"), "{error}");
        assert_eq!(
            record_json(&home)["pending"],
            serde_json::json!([{ "op": "import", "scene": "ink.html" }]),
            "an answer with no id leaves the import pending"
        );
    }

    #[test]
    fn effects_source_prints_the_source_or_writes_it_to_a_new_file_only() {
        let home = TempHome::new("fx-source");
        let answer = serde_json::json!({ "source": "<html>ink</html>", "file_name": "ink.html" });
        let api = FxApi::new(run_scenes()).route("GET", &ink_source_route(), answer.clone());
        let print = SourceArgs { effect: INK.into(), out: None };
        assert_eq!(text_of(effects_source(&api, &print, false).unwrap()), "<html>ink</html>");
        assert_eq!(json_of(effects_source(&api, &print, true).unwrap()), answer);

        let out = home.path().join("copy.html").to_string_lossy().into_owned();
        let write = SourceArgs { effect: INK.into(), out: Some(out.clone()) };
        assert_eq!(
            text_of(effects_source(&api, &write, false).unwrap()),
            format!("wrote 16 bytes of html:ink to {out}\n")
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "<html>ink</html>");
        let error = effects_source(&api, &write, false).unwrap_err();
        assert!(error.to_string().contains("already exists"), "an existing file is not written over: {error}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "<html>ink</html>");
        let document = json_of(
            effects_source(&api, &SourceArgs { effect: INK.into(), out: Some(format!("{out}.2")) }, true).unwrap(),
        );
        assert_eq!((document["id"].as_str(), document["bytes"].as_u64()), (Some("html:ink"), Some(16)));

        // A stock effect the daemon has no source for: its refusal passes through.
        let api = FxApi::new(run_scenes()).refuse("GET", &ink_source_route(), 404, "waves has no source");
        let error = effects_source(&api, &print, false).unwrap_err();
        assert!(error.to_string().contains("404"), "{error}");
    }

    #[test]
    fn preset_list_shows_the_built_in_presets_and_the_users_with_their_props() {
        let api = FxApi::new(run_scenes()).route("GET", &ink_route(), ink_detail());
        let args = PresetListArgs { effect: INK.into() };
        let text = text_of(preset_list(&api, &args, false).unwrap());
        let lines: Vec<Vec<&str>> = text.lines().map(|line| line.split_whitespace().collect()).collect();
        assert_eq!(lines, vec![vec!["built-in", "Calm", "speed=2"], vec!["yours", "Night", "speed=7"]], "{text}");
        let document = json_of(preset_list(&api, &args, true).unwrap());
        assert_eq!(document["effect"], "html:ink");
        assert_eq!(document["presets"], ink_detail()["schema"]["presets"]);
        assert_eq!(document["user_presets"], ink_detail()["user_presets"]);
        assert_eq!(preset_list_text(&serde_json::json!([]), &serde_json::json!([])), "no presets\n");
        assert!(api.writes().is_empty());
    }

    /// An image of the effect's frame 60 as bytes, drawn with these `--prop` arguments.
    fn check_png(tag: &str, props: &[&str]) -> Result<Vec<u8>, String> {
        let file = TempFile::new(&format!("props-{tag}"), MOVING);
        let out = std::env::temp_dir().join(format!("canvas-rgb-check-{}-{tag}.png", std::process::id()));
        let args: Vec<String> = props.iter().map(|p| p.to_string()).collect();
        let result = check_effect_with(&file.path(), Some(&out.to_string_lossy()), &args);
        let bytes = std::fs::read(&out).unwrap_or_default();
        let _ = std::fs::remove_file(&out);
        result.map(|_| bytes)
    }

    #[test]
    fn a_check_with_props_draws_with_them_and_refuses_a_bad_one_before_reading_the_file() {
        let plain = check_png("plain", &[]).expect("the effect loads");
        let green = check_png("green", &["tint=#00ff00"]).expect("the effect loads with a prop");
        assert!(!plain.is_empty() && !green.is_empty(), "both drew a picture");
        assert_ne!(plain, green, "tint=#00ff00 changes the picture: the prop reached the effect");
        assert_eq!(plain, check_png("plain2", &[]).expect("loads"), "the same props draw the same picture");

        let unknown = check_png("unknown", &["bogus=1"]).unwrap_err();
        assert_eq!(unknown, "bogus is not a property of this effect");
        let range = check_png("range", &["speed=500"]).unwrap_err();
        assert!(range.contains("--prop speed"), "{range}");
        let twice = check_png("twice", &["speed=1", "speed=2"]).unwrap_err();
        assert!(twice.contains("--prop speed is given twice"), "{twice}");

        // The pairs are checked before the file is read: the error is not about the file.
        let error = check_effect_with("/no/such/file.html", None, &["speed".to_string()]).unwrap_err();
        assert!(error.contains("is not a key=value pair"), "{error}");
        let error = check_effect_with("/no/such/file.html", None, &[]).unwrap_err();
        assert!(error.contains("could not read"), "{error}");
    }

    #[test]
    fn under_json_every_failure_of_an_effects_command_is_one_document() {
        let other = CliError::Other("html:ink was not made by run agent-1; nothing changed".into());
        let commands = [
            (Command::Effects(EffectsCommand::Delete(effect_delete_args("agent-1", false, false))), Some("agent-1")),
            (
                Command::Effects(EffectsCommand::Preset(PresetCommand::Delete(preset_delete_args("N", "agent-2", false)))),
                Some("agent-2"),
            ),
            (Command::Effects(EffectsCommand::Preset(PresetCommand::Save(save_args(&[], Some("agent-3"))))), Some("agent-3")),
            (Command::Effects(EffectsCommand::Preset(PresetCommand::Save(save_args(&[], None)))), None),
            (Command::Effects(EffectsCommand::Import(import_args("a.html", false, Some("agent-4")))), Some("agent-4")),
            (Command::Effects(EffectsCommand::Import(import_args("a.html", false, None))), None),
            (Command::Effects(EffectsCommand::Preset(PresetCommand::List(PresetListArgs { effect: INK.into() }))), None),
            (Command::Effects(EffectsCommand::Source(SourceArgs { effect: INK.into(), out: None })), None),
            (
                Command::Effects(EffectsCommand::Check(CheckArgs {
                    file: "a.html".into(),
                    png: None,
                    preview: None,
                    props: vec![],
                })),
                None,
            ),
        ];
        for (command, run) in &commands {
            assert_eq!(
                failure_document(command, &other),
                Some(serde_json::json!({ "saved": false, "run": run, "error": other.to_string() })),
                "{command:?}"
            );
        }
        // A daemon refusal reads the same way: one document, the status in its text.
        let refused = CliError::Api { status: 409, message: "a preset of that name exists".into(), body: None };
        let document = failure_document(&commands[2].0, &refused).unwrap();
        assert_eq!(document["error"], refused.to_string());
        assert_eq!(refused.exit_code(), 1);
        // Commands outside this set keep their own output.
        assert_eq!(failure_document(&Command::Effects(EffectsCommand::List), &other), None);
    }
}
