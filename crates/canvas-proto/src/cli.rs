//! CLI command names that tools, the installer and agentmode call. The CLI binary is
//! `canvas-rgb-cli.exe`, invoked as `canvas-rgb` through a `.cmd` shim on PATH.
//! Each command talks to the daemon API with the token file, and prints JSON with `--json`.

/// `canvas-rgb status`
pub const STATUS: &[&str] = &["status"];
/// `canvas-rgb devices list`
pub const DEVICES_LIST: &[&str] = &["devices", "list"];
/// `canvas-rgb effects list`
pub const EFFECTS_LIST: &[&str] = &["effects", "list"];
/// `canvas-rgb effects activate <Name|id> [--<prop> <value>]...`, e.g.
/// `canvas-rgb effects activate Solid --color #ff0000`. Names match case-insensitively.
pub const EFFECTS_ACTIVATE: &[&str] = &["effects", "activate"];
/// `canvas-rgb output pause|resume`
pub const OUTPUT_PAUSE: &[&str] = &["output", "pause"];
pub const OUTPUT_RESUME: &[&str] = &["output", "resume"];
/// `canvas-rgb fans status`
pub const FANS_STATUS: &[&str] = &["fans", "status"];
/// `canvas-rgb fans profile <name>`
pub const FANS_PROFILE: &[&str] = &["fans", "profile"];
/// `canvas-rgb diag readback`: colors per device as OpenRGB reports them.
pub const DIAG_READBACK: &[&str] = &["diag", "readback"];
/// `canvas-rgb diag dump-frames --effect <id> --frames <n> --out <dir>`: renders headless
/// (no devices), writing raw RGB frames for `tools/poison-compare`.
pub const DIAG_DUMP_FRAMES: &[&str] = &["diag", "dump-frames"];
/// `canvas-rgb diag bundle [--out <zip>]`: zips logs and config (G8).
pub const DIAG_BUNDLE: &[&str] = &["diag", "bundle"];
/// `canvas-rgb diag fail-sensor <cpu|gpu|board:<name>> [--clear]`
pub const DIAG_FAIL_SENSOR: &[&str] = &["diag", "fail-sensor"];
/// `canvas-rgb config export <file>` / `canvas-rgb config import <file>` (G10)
pub const CONFIG_EXPORT: &[&str] = &["config", "export"];
pub const CONFIG_IMPORT: &[&str] = &["config", "import"];
/// `canvas-rgb update --file <installer> --sig <installer.sig>`: asks the service to install a
/// signed build as SYSTEM, with no permission prompt (#43). Prints the signed version it started.
pub const UPDATE: &[&str] = &["update"];
