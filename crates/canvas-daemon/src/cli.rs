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

/// `canvas-rgb effects import <file> [--replace]`
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The HTML file to import.
    pub file: String,
    /// Overwrite an effect of the same name.
    #[arg(long)]
    pub replace: bool,
}

/// `canvas-rgb effects check <file> [--png <path>] [--preview <dir>]`
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
    let source = std::fs::read_to_string(path).map_err(|e| format!("could not read {path}: {e}"))?;
    check_source(&source, png)
}

/// Checks a SignalRGB-format HTML effect's source, with no daemon involved.
///
/// The source is parsed with the same runtime the daemon uses, then 120 frames are rendered on
/// the 320x200 canvas. The error is the parser's own message. `effects check` reads a file and
/// calls this; the editor's preview route renders a draft with the same runtime.
pub fn check_source(source: &str, png: Option<&str>) -> Result<CheckReport, String> {
    let mut effect = canvas_html::HtmlEffect::from_source("check", source, &canvas_proto::props::Props::new())
        .map_err(|e| e.to_string())?;
    let schema = effect.schema();
    effect.init(&schema.defaults());

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
    let source = std::fs::read_to_string(path).map_err(|e| format!("could not read {path}: {e}"))?;
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

/// What `--json` prints on stdout when `scenes create`, `scenes edit` or `scenes delete` fails:
/// one document, `{"saved": false, "run": <run or null>, "error": "<text>"}`, for every kind of
/// failure. Any other command answers `None` and keeps its own output.
pub fn failure_document(command: &Command, error: &CliError) -> Option<serde_json::Value> {
    let run = match command {
        Command::Scenes(ScenesCommand::Create(args)) => args.run.as_deref(),
        Command::Scenes(ScenesCommand::Edit(args)) => Some(args.run.as_str()),
        Command::Scenes(ScenesCommand::Delete(args)) => Some(args.run.as_str()),
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
    let (saved, scene) = scene_answer(&answer, args.dry_run)?;
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
            "{id} was not made by run {}; nothing changed",
            args.run
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
    let (saved, scene) = scene_answer(&answer, args.dry_run)?;
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
/// A scene the record does not list counts as the run's when the record holds a pending create
/// of that name, the scene is not in the list the run first read, and it is still at `rev` 1:
/// that is a create that was sent and never recorded.
pub fn scenes_delete(api: &dyn Api, args: &DeleteArgs, json: bool) -> Result<Output, CliError> {
    let mut record = crate::agent_run::load(&args.run)?;
    let list = api.call("GET", canvas_proto::api::routes::SCENES, None)?;
    let found = resolve_scene(&list, &args.scene, json)?;
    let id = str_of(&found, "id").to_string();
    let name = str_of(&found, "name").to_string();
    let mut adopted = None;
    let made = match record.made(&id).cloned() {
        Some(made) => made,
        None => match adoptable(&record, &found) {
            Some((made, pending_name)) => {
                adopted = Some(pending_name);
                made
            }
            None => {
                return Err(CliError::Other(format!(
                    "{id} was not made by run {}; nothing changed",
                    args.run
                )));
            }
        },
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
        if let Some(pending_name) = &adopted {
            record.finish("create", pending_name);
        }
        record.save().map_err(|e| {
            CliError::Other(format!(
                "the scene {id} was deleted, but its run record could not be written: {e}"
            ))
        })?;
    }
    let adoption = adopted.as_ref().map(|_| {
        format!(
            "{id} was made by run {}: its create was sent but never written to the record, so the run owns it.",
            args.run
        )
    });
    Ok(if json {
        let mut document = serde_json::json!({ "saved": !args.dry_run, "run": args.run, "deleted": id });
        if adoption.is_some() {
            document["adopted"] = serde_json::Value::Bool(true);
        }
        Output::Json(document)
    } else {
        let said = if args.dry_run { "would delete" } else { "deleted" };
        let lead = adoption.map(|sentence| format!("{sentence}\n")).unwrap_or_default();
        Output::Text(format!("{lead}{said} {id}\n"))
    })
}

/// The scene a record with a pending create may adopt: its entry is a `create` of the scene's
/// name (case aside), the scene was not in the list the run first read, and it is at `rev` 1,
/// where a create leaves it. Answers the scene as the run's, and the pending entry's name.
fn adoptable(
    record: &crate::agent_run::RunRecord,
    found: &serde_json::Value,
) -> Option<(crate::agent_run::MadeScene, String)> {
    let id = str_of(found, "id");
    let name = str_of(found, "name");
    let rev = found.get("rev").and_then(|v| v.as_u64())?;
    if rev != 1 || record.had_before(id) || id == "?" {
        return None;
    }
    let pending = record
        .pending
        .iter()
        .find(|p| p.op == "create" && p.scene.trim().eq_ignore_ascii_case(name.trim()))?;
    let made = crate::agent_run::MadeScene { id: id.to_string(), name: name.to_string(), rev };
    Some((made, pending.scene.clone()))
}

/// A write failed after its pending entry was saved. When the daemon refused it, or it never
/// went out, the write did not happen: the entry is removed (the record is saved again; if that
/// fails, the entry stays, which only says more). When the outcome is unknown (no answer, or a
/// `5xx`) the entry stays, so the record shows a write that may have landed.
fn settle_pending(record: &mut crate::agent_run::RunRecord, op: &str, scene: &str, error: &CliError) {
    let unknown = match error {
        CliError::NoAnswer(_) => true,
        CliError::Api { status, .. } => *status >= 500,
        _ => false,
    };
    if !unknown {
        record.finish(op, scene);
        let _ = record.save();
    }
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
    fn a_scene_the_run_created_but_never_recorded_can_be_deleted_by_that_run() {
        let home = TempHome::new("adopt");
        let mut list = run_scenes();
        list["scenes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "id": "night", "name": "Night", "rev": 1, "groups": [] }));
        let pending = serde_json::json!([{ "op": "create", "scene": "Night" }]);
        home.write("agent-runs/agent-1.json", &a_record_pending(serde_json::json!([]), "gaming", pending));
        let api = RunApi::answering(list.clone(), serde_json::Value::Null);
        let args = DeleteArgs { scene: "night".into(), run: "agent-1".into(), force: false, dry_run: false };
        let text = text_of(scenes_delete(&api, &args, false).unwrap());
        assert_eq!(
            text,
            "night was made by run agent-1: its create was sent but never written to the record, so the run owns it.\ndeleted night\n"
        );
        let (method, path, _) = api.write();
        assert_eq!((method.as_str(), path.as_str()), ("DELETE", "/api/v1/scenes/night"));
        assert!(record_json(&home).get("pending").is_none(), "the create is settled: {}", record_json(&home));

        // Not at rev 1: someone changed it, so it is not provably the run's.
        let mut changed = list.clone();
        changed["scenes"][3]["rev"] = serde_json::json!(2);
        home.write(
            "agent-runs/agent-1.json",
            &a_record_pending(serde_json::json!([]), "gaming", serde_json::json!([{ "op": "create", "scene": "Night" }])),
        );
        let api = RunApi::answering(changed, serde_json::Value::Null);
        let error = scenes_delete(&api, &args, false).expect_err("rev 2 is not adopted");
        assert_eq!(error.to_string(), "night was not made by run agent-1; nothing changed");

        // A scene that was in the list before the run is never adopted, whatever its name.
        let mut at_rev_one = run_scenes();
        at_rev_one["scenes"][2]["rev"] = serde_json::json!(1);
        let api = RunApi::answering(at_rev_one, serde_json::Value::Null);
        home.write(
            "agent-runs/agent-1.json",
            &a_record_pending(serde_json::json!([]), "gaming", serde_json::json!([{ "op": "create", "scene": "Sleep" }])),
        );
        let sleep = DeleteArgs { scene: "sleep".into(), run: "agent-1".into(), force: true, dry_run: false };
        assert_eq!(
            scenes_delete(&api, &sleep, false).unwrap_err().to_string(),
            "sleep was not made by run agent-1; nothing changed"
        );
        // And with no pending create, nothing is adopted.
        home.write("agent-runs/agent-1.json", &a_record(serde_json::json!([]), "gaming"));
        let api = RunApi::answering(list, serde_json::Value::Null);
        assert!(scenes_delete(&api, &args, false).is_err());
        assert!(api.asked().iter().all(|(method, _, _)| method == "GET"));
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
}
