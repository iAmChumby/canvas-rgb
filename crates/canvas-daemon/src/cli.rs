//! The `canvas-rgb` CLI.
//!
//! Every command talks to the daemon's API with the token file, prints a readable table
//! by default and JSON with `--json`, and exits non-zero with the API's error message
//! when something fails. A refused connection prints "daemon not running".

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;

use canvas_proto::peripheral::PeripheralAccess;
use canvas_proto::traits::Effect;

use crate::peer;
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
    /// The effect library.
    #[command(subcommand)]
    Effects(EffectsCommand),
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
/// The kinds are: color, number, bool, choice and hue.
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
    #[error("the daemon is not running (nothing answers on {0})")]
    NotRunning(String),
    /// The daemon is not running, or the port is held by another program: the API token
    /// is not sent to it.
    #[error("{0}")]
    NotTrusted(String),
    #[error("the API returned {status}: {message}")]
    Api { status: u16, message: String },
    #[error("{0}")]
    Other(String),
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
            CliError::NotRunning(_) | CliError::NotTrusted(_) => 2,
            CliError::Api { .. } => 1,
            CliError::Other(_) => 1,
        }
    }
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
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { base, token, agent })
    }

    /// Sends a request and returns the body.
    pub fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<serde_json::Value, CliError> {
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
                return Err(CliError::Other(format!("unsupported HTTP method {other}")));
            }
        };

        match response {
            Ok(mut response) => {
                let status = response.status().as_u16();
                let text = response.body_mut().read_to_string().unwrap_or_default();
                if status >= 400 {
                    // The API answers with `{ "error": ..., "message": ... }`.
                    let message = serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
                        .unwrap_or(text);
                    return Err(CliError::Api { status, message });
                }
                if text.trim().is_empty() {
                    return Ok(serde_json::Value::Null);
                }
                serde_json::from_str(&text).or(Ok(serde_json::Value::String(text)))
            }
            // The connection failed: the daemon is not running.
            Err(_) => Err(CliError::NotRunning(self.base.clone())),
        }
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
        let report = check_effect(&file.path(), None).expect("the effect loads");        assert_eq!(report.name, "Moving");
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
}
