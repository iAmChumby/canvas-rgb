//! The `canvas-rgb` CLI.
//!
//! Every command talks to the daemon's API with the token file, prints a readable table
//! by default and JSON with `--json`, and exits non-zero with the API's error message
//! when something fails. A refused connection prints "daemon not running".

use clap::{Args, Parser, Subcommand};
use std::collections::BTreeMap;

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
    /// Diagnostics.
    #[command(subcommand)]
    Diag(DiagCommand),
    /// The configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
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
    /// List the 12 effects.
    List,
    /// Activate an effect.
    Activate(ActivateArgs),
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

/// A CLI error.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("the daemon is not running (nothing answers on {0})")]
    NotRunning(String),
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
            CliError::NotRunning(_) => 2,
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
    pub fn new(base: Option<&str>) -> Result<Self, CliError> {
        let base = base
            .map(str::to_string)
            .unwrap_or_else(canvas_proto::consts::api_base_url);
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

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::props::PropKind;

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
}
