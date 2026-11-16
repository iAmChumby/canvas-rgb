//! #6: no console window after logon.
//!
//! The daemon used to be a console program, so every launcher that did not pass
//! `CREATE_NO_WINDOW` (the old HKCU Run value started by Explorer, a shell) gave it a terminal
//! window, and closing that window ended the daemon. The fix names the Windows convention in the
//! entry point: a release `canvas-rgbd.exe` is a windows-subsystem program (`#![windows_subsystem]`
//! in `src/main.rs`), so no launcher can give it a console, while `canvas-rgb-cli.exe` keeps its
//! console for scripts.
//!
//! The measurement is the PE header of the built binaries, not the source: `Subsystem` in the
//! optional header is 3 (`IMAGE_SUBSYSTEM_WINDOWS_CUI`, console) for a console program and 2
//! (`IMAGE_SUBSYSTEM_WINDOWS_GUI`) for a windows program. A debug build keeps the console on
//! purpose (`cargo run`, the logs on a terminal), so the release value is the one the installed
//! payload must have — the installer builds with `cargo build --release`
//! (`tools/build-installer.ps1`).

/// The source of the daemon's entry point: the attribute that picks the subsystem lives there.
const MAIN: &str = include_str!("../src/main.rs");

/// The source of the CLI's entry point: it must never ask for the windows subsystem.
const CLI: &str = include_str!("../src/bin/cli.rs");

/// `IMAGE_SUBSYSTEM_WINDOWS_GUI`: a windows program, no console, ever.
const WINDOWS_GUI: u16 = 2;

/// `IMAGE_SUBSYSTEM_WINDOWS_CUI`: a console program, one console window per launch.
const CONSOLE: u16 = 3;

/// The `Subsystem` field of an executable's PE optional header, read from the bytes on disk.
///
/// `Subsystem` sits at optional-header offset 68 in both PE32 and PE32+. The optional header
/// starts after the DOS stub's `e_lfanew` (offset 0x3C), the 4-byte `PE\0\0` signature and the
/// 20-byte COFF file header.
fn subsystem(bytes: &[u8]) -> u16 {
    assert_eq!(&bytes[0..2], b"MZ", "not a PE image: no MZ header");
    let lfanew = u32::from_le_bytes(bytes[0x3C..0x40].try_into().expect("four bytes")) as usize;
    assert_eq!(
        &bytes[lfanew..lfanew + 4],
        b"PE\0\0",
        "not a PE image: no PE signature at e_lfanew"
    );
    let optional = lfanew + 4 + 20;
    u16::from_le_bytes(bytes[optional + 68..optional + 70].try_into().expect("two bytes"))
}

/// The built binary Cargo made for this run, and its bytes.
fn built(name: &str) -> (String, Vec<u8>) {
    let path = match name {
        "canvas-rgbd" => env!("CARGO_BIN_EXE_canvas-rgbd").to_string(),
        "canvas-rgb-cli" => env!("CARGO_BIN_EXE_canvas-rgb-cli").to_string(),
        other => panic!("no binary named {other}"),
    };
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    (path, bytes)
}

/// The daemon's entry point asks Windows for a windows-subsystem release build (#6).
///
/// Without the attribute the release binary is a console program again, and the first launcher
/// that does not hide it puts a terminal window on the owner's desktop.
#[test]
fn the_daemon_entry_point_asks_for_a_windowless_release() {
    assert!(
        MAIN.contains("#![cfg_attr(not(debug_assertions), windows_subsystem = \"windows\")]"),
        "the daemon's entry point must be a windows-subsystem program in release (#6)"
    );
    assert!(
        MAIN.contains("logging::init(\"canvas-rgbd\")"),
        "with no console, the daemon's log file is its only record: it must still start the file log"
    );
}

/// The built daemon's PE header says what the entry point asked for (#6).
///
/// In release this reads `2` (windows GUI: no launcher can give it a console) and in debug `3`
/// (console, so `cargo run` and the tests keep their terminal). Run it with
/// `cargo test --release --test console_window` to measure the payload the installer ships.
#[test]
fn the_daemon_binary_has_the_subsystem_the_installer_ships() {
    let (path, bytes) = built("canvas-rgbd");
    let (profile, want) = if cfg!(debug_assertions) {
        ("debug", CONSOLE)
    } else {
        ("release", WINDOWS_GUI)
    };
    let got = subsystem(&bytes);
    assert_eq!(
        got, want,
        "canvas-rgbd.exe ({profile} build, {path}) has PE subsystem {got}; want {want} \
         (2 = windows GUI, no console; 3 = console)"
    );
}

/// The CLI keeps its console in every profile, so scripts still read its output (#6).
#[test]
fn the_cli_binary_keeps_its_console() {
    assert!(
        !CLI.contains("windows_subsystem"),
        "canvas-rgb-cli.exe must never ask for the windows subsystem: its output is a script's input"
    );
    let (path, bytes) = built("canvas-rgb-cli");
    let got = subsystem(&bytes);
    assert_eq!(
        got,
        CONSOLE,
        "canvas-rgb-cli.exe ({path}) has PE subsystem {got}; want {CONSOLE} (console)"
    );
}
