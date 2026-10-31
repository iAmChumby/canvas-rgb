//! `canvas-rgb-cli.exe`: the command line client.
//!
//! It is installed as `canvas-rgb.cmd` on PATH, because the tray owns the name
//! `canvas-rgb.exe`.

use canvas_daemon::cli::{
    Cli, CliError, Command, ConfigCommand, DevicesCommand, DiagCommand, EffectsCommand, FansCommand,
    OutputCommand, PeripheralsCommand,
};use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code() as u8)
        }
    }
}

/// Runs the command.
fn run(cli: &Cli) -> Result<(), CliError> {
    match &cli.command {
        Command::Status => status(cli),
        Command::Devices(DevicesCommand::List) => devices(cli),
        Command::Effects(EffectsCommand::List) => effects_list(cli),
        Command::Effects(EffectsCommand::Activate(args)) => effects_activate(cli, args),
        Command::Effects(EffectsCommand::Import(args)) => effects_import(cli, args),
        Command::Effects(EffectsCommand::Check(args)) => effects_check(cli, args),
        Command::Output(OutputCommand::Pause) => output(cli, "paused"),
        Command::Output(OutputCommand::Resume) => output(cli, "running"),
        Command::Fans(FansCommand::Status) => fans_status(cli),
        Command::Fans(FansCommand::Profile { name }) => fans_profile(cli, name),
        Command::Peripherals(PeripheralsCommand::List) => peripherals_list(cli),
        Command::Peripherals(PeripheralsCommand::Show { id }) => peripherals_show(cli, id),
        Command::Peripherals(PeripheralsCommand::Access { id, reads, writes }) => {
            peripherals_access(cli, id, reads.map(|s| s.is_on()), writes.map(|s| s.is_on()))
        }
        Command::Diag(DiagCommand::Readback) => diag_readback(cli),
        Command::Diag(DiagCommand::DumpFrames(args)) => diag_dump_frames(args),
        Command::Diag(DiagCommand::Bundle { out }) => diag_bundle(out),
        Command::Diag(DiagCommand::FailSensor { source, clear }) => {
            diag_fail_sensor(cli, source, *clear)
        }
        Command::Config(ConfigCommand::Export { file }) => config_export(cli, file),
        Command::Config(ConfigCommand::Import { file }) => config_import(cli, file),
        Command::Update(args) => update(cli, args),
    }
}

/// The client, or an error.
fn client(cli: &Cli) -> Result<canvas_daemon::cli::Client, CliError> {
    canvas_daemon::cli::Client::new(cli.api.as_deref())
}

/// Prints a value as JSON or as a table.
fn print_json(_cli: &Cli, value: &serde_json::Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("could not format the answer: {e}"),
    }
}

/// `canvas-rgb status`.
fn status(cli: &Cli) -> Result<(), CliError> {
    let client = client(cli)?;
    let status = client.request("GET", canvas_proto::api::routes::STATUS, None)?;
    if cli.json {
        print_json(cli, &status);
        return Ok(());
    }

    println!("canvas-rgb {}", status.get("version").and_then(|v| v.as_str()).unwrap_or("?"));
    println!(
        "  power        {}",
        status.get("power").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "  brightness   {}",
        status.get("brightness").and_then(|v| v.as_u64()).unwrap_or(0)
    );
    println!(
        "  fps          {} target, {} measured",
        status.get("fps_target").and_then(|v| v.as_u64()).unwrap_or(0),
        status
            .get("fps_measured")
            .and_then(|v| v.as_f64())
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "?".into())
    );
    println!(
        "  frames       {} rendered{}",
        status.get("frames_rendered").and_then(|v| v.as_u64()).unwrap_or(0),
        if status.get("parked").and_then(|v| v.as_bool()).unwrap_or(false) {
            " (parked)"
        } else {
            ""
        }
    );
    println!(
        "  devices      {} connected",
        status.get("devices_connected").and_then(|v| v.as_u64()).unwrap_or(0)
    );
    println!(
        "  preview      {} subscriber(s)",
        status.get("preview_subscribers").and_then(|v| v.as_u64()).unwrap_or(0)
    );
    println!(
        "  openrgb      {}",
        status.get("openrgb").and_then(|v| v.as_str()).unwrap_or("?")
    );
    let service = status.get("service").and_then(|v| v.as_str()).unwrap_or("?");
    println!("  service      {service}");
    if let Some(down) = status.get("service_down_ms").and_then(|v| v.as_u64()) {
        println!("               down for {down} ms");
    }
    println!(
        "  layout       {}",
        status.get("active_layout").and_then(|v| v.as_str()).unwrap_or("none")
    );
    println!(
        "  effect       {}",
        status.get("active_effect").and_then(|v| v.as_str()).unwrap_or("none")
    );
    if status.get("show_zone_panel").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("  zone panel   open (size your ARGB headers)");
    }

    // The conflict banner.
    if let Some(conflicts) = status.get("conflicts").and_then(|v| v.as_array()) {
        for conflict in conflicts {
            // The spine tags the conflict with `kind`.
            if conflict.get("kind").and_then(|v| v.as_str()) == Some("signal_rgb") {
                let process = conflict.get("process").and_then(|v| v.as_bool()).unwrap_or(false);
                let service = conflict.get("service").and_then(|v| v.as_bool()).unwrap_or(false);
                let mut parts = Vec::new();
                if process {
                    parts.push(canvas_proto::consts::SIGNALRGB_PROCESS.to_string());
                }
                if service {
                    parts.push(format!("{} service", canvas_proto::consts::SIGNALRGB_SERVICE));
                }
                println!(
                    "  CONFLICT     SignalRGB is running ({}): it will fight canvas-rgb for the devices",
                    parts.join(", ")
                );
            }
        }
    }
    Ok(())
}

/// `canvas-rgb devices list`.
fn devices(cli: &Cli) -> Result<(), CliError> {
    let client = client(cli)?;
    let answer = client.request("GET", canvas_proto::api::routes::DEVICES, None)?;
    if cli.json {
        print_json(cli, &answer);
        return Ok(());
    }
    let devices = answer
        .get("devices")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if devices.is_empty() {
        println!("no devices");
        return Ok(());
    }
    println!("{:<28} {:<12} {:<22} ID", "NAME", "KIND", "ZONES");
    for device in &devices {
        let name = device.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = device.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let id = device.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let zones = device
            .get("zones")
            .and_then(|v| v.as_array())
            .map(|zones| {
                zones
                    .iter()
                    .map(|z| {
                        let index = z.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                        let leds = z.get("led_count").and_then(|v| v.as_u64()).unwrap_or(0);
                        format!("#{index}:{leds}")
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let total: u64 = device
            .get("zones")
            .and_then(|v| v.as_array())
            .map(|zones| zones.iter().filter_map(|z| z.get("led_count").and_then(|v| v.as_u64())).sum())
            .unwrap_or(0);
        println!("{:<28} {:<12} {:<22} {} ({} LEDs)", name, kind, zones, id, total);
    }
    Ok(())
}

/// `canvas-rgb effects list`.
fn effects_list(cli: &Cli) -> Result<(), CliError> {
    let client = client(cli)?;
    let answer = client.request("GET", canvas_proto::api::routes::EFFECTS, None)?;
    if cli.json {
        print_json(cli, &answer);
        return Ok(());
    }
    let active = answer.get("active").and_then(|v| v.as_str()).unwrap_or("");
    let effects = answer
        .get("effects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for effect in &effects {
        let id = effect.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = effect.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let favorite = effect.get("favorite").and_then(|v| v.as_bool()).unwrap_or(false);
        let marker = if favorite { "*" } else { " " };
        let active_mark = if id == active { " (active)" } else { "" };
        println!("{marker} {:<20} {:<14}{active_mark}", id, name);
    }
    Ok(())
}

/// `canvas-rgb effects activate <Name|id> [--<prop> <value>]...`.
fn effects_activate(cli: &Cli, args: &canvas_daemon::cli::ActivateArgs) -> Result<(), CliError> {
    let client = client(cli)?;

    // The props are parsed against the effect's schema, so a bad value is refused
    // before it reaches the daemon.
    let detail = client.request(
        "GET",
        &format!("{}/{}", canvas_proto::api::routes::EFFECTS, args.effect),
        None,
    )?;
    let schema = detail.get("schema").cloned().unwrap_or(serde_json::Value::Null);
    let properties = schema.get("properties").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    let raw = canvas_daemon::cli::parse_activate_props(&args.props)?;
    let mut props = serde_json::Map::new();
    for (key, value) in raw {
        let Some(property) = properties.iter().find(|p| p.get("key").and_then(|v| v.as_str()) == Some(key.as_str()))
        else {
            return Err(CliError::Other(format!(
                "{key} is not a property of this effect"
            )));
        };
        let kind = property.get("kind").cloned().unwrap_or(serde_json::Value::Null);
        let kind: canvas_proto::props::PropKind = serde_json::from_value(kind)
            .map_err(|e| CliError::Other(format!("could not read the schema of {key}: {e}")))?;
        let parsed = canvas_daemon::cli::parse_prop_value(&kind, &value)
            .map_err(|e| CliError::Other(format!("--{key}: {e}")))?;
        props.insert(key, serde_json::to_value(parsed).map_err(|e| CliError::Other(e.to_string()))?);
    }

    let body = serde_json::json!({ "props": props }).to_string();
    let answer = client.request(
        "POST",
        &format!("{}/{}/activate", canvas_proto::api::routes::EFFECTS, args.effect),
        Some(&body),
    )?;
    if cli.json {
        print_json(cli, &answer);
    } else {
        println!("activated {}", args.effect);
    }
    Ok(())
}

/// `canvas-rgb effects import <file> [--replace]`.
///
/// The daemon writes the file into the effects folder. The token only goes to the
/// daemon: the client checks who holds the port first.
fn effects_import(cli: &Cli, args: &canvas_daemon::cli::ImportArgs) -> Result<(), CliError> {
    // The file is read first: a file that is not there is a local problem, and it is
    // reported without asking the daemon anything.
    let path = std::path::Path::new(&args.file);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| CliError::Other(format!("{} has no file name", args.file)))?;
    let html = std::fs::read_to_string(path)
        .map_err(|e| CliError::Other(format!("could not read {}: {e}", path.display())))?;
    // A file that is over the limit is refused here, so it is never posted.
    if html.len() > canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES {
        return Err(CliError::Other(format!(
            "{} is {} bytes; the limit is {}",
            path.display(),
            html.len(),
            canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES
        )));
    }
    let client = client(cli)?;
    let body = serde_json::json!({
        "file_name": name,
        "html": html,
        "replace": args.replace,
    })
    .to_string();
    let answer =
        client.request("POST", canvas_proto::api::routes::EFFECTS_IMPORT, Some(&body))?;
    if cli.json {
        print_json(cli, &answer);
        return Ok(());
    }
    let effect = answer.get("effect");
    let id = effect.and_then(|e| e.get("id")).and_then(|v| v.as_str()).unwrap_or("?");
    let name = effect.and_then(|e| e.get("name")).and_then(|v| v.as_str()).unwrap_or("?");
    let replaced = answer.get("replaced").and_then(|v| v.as_bool()).unwrap_or(false);
    if replaced {
        println!("imported {id} ({name}); it replaced the file of the same name");
    } else {
        println!("imported {id} ({name})");
    }
    Ok(())
}

/// `canvas-rgb effects check <file> [--png <path>]`.
///
/// No daemon is involved: the file is parsed and rendered here.
fn effects_check(cli: &Cli, args: &canvas_daemon::cli::CheckArgs) -> Result<(), CliError> {
    let report = canvas_daemon::cli::check_effect(&args.file, args.png.as_deref())
        .map_err(CliError::Other)?;

    let mut problems: Vec<&str> = Vec::new();
    if !report.moves {
        problems.push("every frame is the same: the effect does not move");
    }
    if report.flat {
        problems.push("the picture is one flat colour");
    }
    if !report.errors.is_empty() {
        problems.push("the script reported an error while the frames were rendered");
    }

    if cli.json {
        print_json(
            cli,
            &serde_json::json!({
                "name": report.name,
                "description": report.description,
                "properties": report.properties.iter().map(|p| serde_json::json!({
                    "key": p.key,
                    "type": p.kind,
                    "default": p.default,
                })).collect::<Vec<_>>(),
                "frames": report.frames,
                "mean_frame_ms": report.mean_frame_ms,
                "max_frame_ms": report.max_frame_ms,
                "moves": report.moves,
                "flat": report.flat,
                "png": report.png,
                "errors": report.errors,
                "problems": problems,
            }),
        );
    } else {
        println!("name        {}", report.name);
        if !report.description.is_empty() {
            println!("description {}", report.description);
        }
        if report.properties.is_empty() {
            println!("properties  none");
        } else {
            println!("properties");
            for property in &report.properties {
                println!(
                    "  {:<12} {:<8} {}",
                    property.key, property.kind, property.default
                );
            }
        }
        println!(
            "frames      {} rendered, {:.3} ms mean, {:.3} ms max",
            report.frames, report.mean_frame_ms, report.max_frame_ms
        );
        println!(
            "picture     {}, {}",
            if report.moves {
                "changes over time"
            } else {
                "the same in every frame"
            },
            if report.flat {
                "one flat colour"
            } else {
                "not one flat colour"
            }
        );
        if let Some(png) = &report.png {
            println!("png         {png}");
        }
        for error in &report.errors {
            println!("script      {error}");
        }
        for problem in &problems {
            println!("problem     {problem}");
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(CliError::Other("the effect is likely broken".to_string()))
    }
}

/// `canvas-rgb output pause|resume`.
fn output(cli: &Cli, power: &str) -> Result<(), CliError> {
    let client = client(cli)?;
    let body = serde_json::json!({ "power": power }).to_string();
    let answer = client.request("PATCH", canvas_proto::api::routes::OUTPUT, Some(&body))?;
    if cli.json {
        print_json(cli, &answer);
    } else {
        println!("output is {power}");
    }
    Ok(())
}

/// `canvas-rgb fans status`.
fn fans_status(cli: &Cli) -> Result<(), CliError> {
    let client = client(cli)?;
    let answer = client.request("GET", canvas_proto::api::routes::FANS, None)?;
    if cli.json {
        print_json(cli, &answer);
        return Ok(());
    }
    println!(
        "chip          {}",
        answer.get("chip").and_then(|v| v.as_str()).unwrap_or("not detected")
    );
    println!(
        "profile       {}",
        answer.get("active_profile").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "floor         {}%",
        answer.get("floor_percent").and_then(|v| v.as_u64()).unwrap_or(0)
    );
    let headers = answer
        .get("headers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if headers.is_empty() {
        println!("no fan headers");
    } else {
        println!("{:<14} {:<10} {:<8} MODE", "HEADER", "RPM", "DUTY");
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
            let asked = describe_mode(header.get("mode"));
            let on_bios = header.get("on_bios").and_then(|v| v.as_bool()).unwrap_or(false);
            let reason = header.get("override_reason").and_then(|v| v.as_str());
            // What the chip is doing comes first; what was asked is shown only when it differs.
            let mode = match (on_bios, asked.as_str(), reason) {
                (true, "BIOS", _) => "BIOS".to_string(),
                (true, _, Some(reason)) => format!("BIOS (asked {asked}; {reason})"),
                (true, _, None) => format!("BIOS (asked {asked})"),
                (false, _, Some(reason)) => format!("{asked} ({reason})"),
                (false, _, None) => asked,
            };
            println!("{:<14} {:<10} {:<8} {}", id, rpm, duty, mode);
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
        println!("{:<14} {celsius}", label);
    }
    Ok(())
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
            let points = mode.get("points").and_then(|v| v.as_array()).map(|p| p.len()).unwrap_or(0);
            format!("curve ({points} points)")
        }
        _ => "?".into(),
    }
}

/// `canvas-rgb fans profile <name>`.
fn fans_profile(cli: &Cli, name: &str) -> Result<(), CliError> {
    let client = client(cli)?;
    let body = serde_json::json!({ "name": name }).to_string();
    let answer = client.request("PATCH", canvas_proto::api::routes::FAN_PROFILE, Some(&body))?;
    if cli.json {
        print_json(cli, &answer);
    } else {
        println!("the fan profile is {name}");
    }
    Ok(())
}

/// `canvas-rgb peripherals list`. The peripherals the adapters see.
fn peripherals_list(cli: &Cli) -> Result<(), CliError> {
    let client = client(cli)?;
    match canvas_daemon::cli::peripherals_list(&client, cli.json)? {
        canvas_daemon::cli::Output::Json(value) => print_json(cli, &value),
        canvas_daemon::cli::Output::Text(text) => print!("{text}"),
    }
    Ok(())
}

/// `canvas-rgb peripherals show <id>`.
fn peripherals_show(cli: &Cli, id: &str) -> Result<(), CliError> {
    let client = client(cli)?;
    match canvas_daemon::cli::peripherals_show(&client, id, cli.json)? {
        canvas_daemon::cli::Output::Json(value) => print_json(cli, &value),
        canvas_daemon::cli::Output::Text(text) => print!("{text}"),
    }
    Ok(())
}

/// `canvas-rgb peripherals access <id> --reads on|off --writes on|off`.
fn peripherals_access(
    cli: &Cli,
    id: &str,
    reads: Option<bool>,
    writes: Option<bool>,
) -> Result<(), CliError> {
    let client = client(cli)?;
    match canvas_daemon::cli::peripherals_access(&client, id, reads, writes, cli.json)? {
        canvas_daemon::cli::Output::Json(value) => print_json(cli, &value),
        canvas_daemon::cli::Output::Text(text) => print!("{text}"),
    }
    Ok(())
}

/// `canvas-rgb diag readback`.
fn diag_readback(cli: &Cli) -> Result<(), CliError> {
    let client = client(cli)?;
    let answer = client.request("GET", canvas_proto::api::routes::DIAG_READBACK, None)?;
    if cli.json {
        print_json(cli, &answer);
        return Ok(());
    }
    let devices = answer
        .get("devices")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if devices.is_empty() {
        println!("no devices to read back");
        return Ok(());
    }
    for device in &devices {
        let name = device.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let id = device.get("device_id").and_then(|v| v.as_str()).unwrap_or("?");
        let colors = device
            .get("colors")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        println!("{name} ({id})");
        for (index, color) in colors.iter().enumerate() {
            println!("  {index:>4}  {}", color.as_str().unwrap_or("?"));
        }
    }
    Ok(())
}

/// `canvas-rgb diag dump-frames ...`.
///
/// Renders locally, with no daemon involved.
fn diag_dump_frames(args: &canvas_daemon::cli::DumpFramesArgs) -> Result<(), CliError> {
    let out = std::path::PathBuf::from(&args.out);
    std::fs::create_dir_all(&out).map_err(|e| CliError::Other(format!("could not create {}: {e}", out.display())))?;

    let mut effect = canvas_effects::create(&args.effect)
        .ok_or_else(|| CliError::Other(format!("no effect with the id {}", args.effect)))?;

    // The props.
    let mut props = canvas_proto::props::Props::new();
    let schema = effect.schema();
    for pair in &args.props {
        let (key, value) = canvas_daemon::cli::parse_prop_pair(pair)
            .map_err(CliError::Other)?;
        let property = schema
            .properties
            .iter()
            .find(|p| p.key == key)
            .ok_or_else(|| CliError::Other(format!("{key} is not a property of this effect")))?;
        let parsed = canvas_daemon::cli::parse_prop_value(&property.kind, &value)
            .map_err(|e| CliError::Other(format!("--prop {key}: {e}")))?;
        props.insert(key, parsed);
    }
    effect.init(&props);

    let mut canvas = canvas_proto::traits::Canvas::new(args.width, args.height);
    let step = 1.0 / args.fps.max(1) as f32;
    let mut time = 0.0f64;

    for frame_index in 0..args.frames {
        effect.update(step, &mut canvas, &canvas_proto::traits::Inputs { time, ..Default::default() });
        let mut rgb = Vec::with_capacity((args.width * args.height * 3) as usize);
        for pixel in &canvas.pixels {
            let c = canvas_proto::Rgb8::from_f32(*pixel);
            rgb.push(c.r);
            rgb.push(c.g);
            rgb.push(c.b);
        }
        let name = format!("frame_{frame_index:05}.rgb");
        std::fs::write(out.join(name), &rgb)
            .map_err(|e| CliError::Other(format!("could not write the frame: {e}")))?;
        time += step as f64;
    }

    let meta = serde_json::json!({
        "width": args.width,
        "height": args.height,
        "frames": args.frames,
        "fps": args.fps,
        "effect": args.effect,
    });
    std::fs::write(
        out.join("meta.json"),
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    )
    .map_err(|e| CliError::Other(format!("could not write meta.json: {e}")))?;

    println!("wrote {} frames to {}", args.frames, out.display());
    Ok(())
}

/// `canvas-rgb diag bundle [--out <zip>]`.
fn diag_bundle(out: &Option<String>) -> Result<(), CliError> {
    let target = match out {
        Some(path) => std::path::PathBuf::from(path),
        None => std::env::current_dir()
            .map_err(|e| CliError::Other(e.to_string()))?
            .join(format!("canvas-rgb-diag-{}.zip", canvas_daemon::storage::timestamp())),
    };
    let path = canvas_daemon::bundle::write_bundle(&target).map_err(|e| CliError::Other(e.to_string()))?;
    println!("wrote {}", path.display());
    Ok(())
}

/// `canvas-rgb diag fail-sensor <cpu|gpu|board:<name>> [--clear]`.
fn diag_fail_sensor(cli: &Cli, source: &str, clear: bool) -> Result<(), CliError> {
    let client = client(cli)?;
    let body = serde_json::json!({ "source": source, "failed": !clear }).to_string();
    let answer = client.request("POST", "/api/v1/diag/fail-sensor", Some(&body))?;
    if cli.json {
        print_json(cli, &answer);
    } else {
        if clear {
            println!("cleared the failure on {source}");
        } else {
            println!("{source} now looks failed");
        }
    }
    Ok(())
}

/// `canvas-rgb config export <file>`.
fn config_export(cli: &Cli, file: &str) -> Result<(), CliError> {
    let client = client(cli)?;
    let answer = client.request("GET", canvas_proto::api::routes::CONFIG_EXPORT, None)?;
    let text = serde_json::to_string_pretty(&answer)
        .map_err(|e| CliError::Other(format!("could not format the export: {e}")))?;
    std::fs::write(file, text)
        .map_err(|e| CliError::Other(format!("could not write {file}: {e}")))?;
    println!("wrote {file}");
    Ok(())
}

/// `canvas-rgb config import <file>`.
fn config_import(cli: &Cli, file: &str) -> Result<(), CliError> {
    let client = client(cli)?;
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError::Other(format!("could not read {file}: {e}")))?;
    // Check it parses before sending it, so the error is clear.
    serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| CliError::Other(format!("{file} is not valid JSON: {e}")))?;
    let answer = client.request("POST", canvas_proto::api::routes::CONFIG_IMPORT, Some(&text))?;
    if cli.json {
        print_json(cli, &answer);
    } else {
        println!("imported {file}");
    }
    Ok(())
}

/// `canvas-rgb update --file <installer> --sig <sig file>`.
///
/// The service runs setup as SYSTEM, so there is no permission prompt. Any refusal (a bad
/// signature, an older version, an install already running, the service down) exits non-zero
/// with the service's message.
fn update(cli: &Cli, args: &canvas_daemon::cli::UpdateArgs) -> Result<(), CliError> {
    let request = args.request()?;
    let client = client(cli)?;
    let body = serde_json::to_string(&request)
        .map_err(|e| CliError::Other(format!("could not build the request: {e}")))?;
    let answer = client.request("POST", canvas_proto::api::routes::UPDATE_INSTALL, Some(&body))?;
    if cli.json {
        print_json(cli, &answer);
    } else {
        let version = answer.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        println!("the service started the update to {version}");
    }
    Ok(())
}
