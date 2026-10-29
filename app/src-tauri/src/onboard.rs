//! When the tray opens the editor for the onboarding.
//!
//! Two paths open the window:
//!
//! - **`--onboard`** (the installer's launch, also through the single-instance callback): open
//!   at once, without waiting for a poll. It does not count toward the auto-open limit.
//! - **A poll** that reports `onboarding_complete == false`: open once per tray run, and only
//!   while the tray's own counter is below [`AUTO_OPEN_LIMIT`]. Each such open adds one to the
//!   counter, a small file in the tray's app-data folder.

use std::path::{Path, PathBuf};

/// The command-line flag the installer passes.
pub const ONBOARD_ARG: &str = "--onboard";

/// How many times the poll may open the window by itself, over all tray runs.
pub const AUTO_OPEN_LIMIT: u32 = 3;

/// True when the arguments hold `--onboard`.
pub fn has_onboard_arg<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().any(|a| a.as_ref() == ONBOARD_ARG)
}

/// `%LOCALAPPDATA%\canvas-rgb\tray\`: the tray's own folder.
pub fn tray_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(canvas_proto::consts::APP_NAME)
        .join("tray")
}

fn counter_path(dir: &Path) -> PathBuf {
    dir.join("onboard-auto-opens")
}

/// How many times the poll has opened the window. A missing or unreadable file is 0.
pub fn auto_open_count(dir: &Path) -> u32 {
    std::fs::read_to_string(counter_path(dir))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// Adds one to the counter and returns the new value. `Err` when the file cannot be written.
pub fn bump_auto_open_count(dir: &Path) -> std::io::Result<u32> {
    let next = auto_open_count(dir).saturating_add(1);
    std::fs::create_dir_all(dir)?;
    std::fs::write(counter_path(dir), next.to_string())?;
    Ok(next)
}

/// The poll rule: the daemon answered, onboarding is not complete, this run has not opened
/// the window yet, and the counter is below the limit.
pub fn should_auto_open(
    daemon: bool,
    onboarding_complete: bool,
    opened_this_run: bool,
    count: u32,
) -> bool {
    daemon && !onboarding_complete && !opened_this_run && count < AUTO_OPEN_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canvas-tray-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_flag_is_found_among_the_arguments() {
        assert!(has_onboard_arg(&["canvas-rgb.exe", "--onboard"]));
        assert!(!has_onboard_arg(&["canvas-rgb.exe"]));
        assert!(!has_onboard_arg(&["canvas-rgb.exe", "--onboarding"]));
    }

    #[test]
    fn the_counter_starts_at_zero_and_counts_up() {
        let dir = temp_dir("count");
        assert_eq!(auto_open_count(&dir), 0);
        assert_eq!(bump_auto_open_count(&dir).unwrap(), 1);
        assert_eq!(bump_auto_open_count(&dir).unwrap(), 2);
        assert_eq!(auto_open_count(&dir), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_garbled_counter_file_reads_as_zero() {
        let dir = temp_dir("garbled");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(counter_path(&dir), "not a number").unwrap();
        assert_eq!(auto_open_count(&dir), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_poll_opens_only_on_the_first_three_runs() {
        assert!(should_auto_open(true, false, false, 0));
        assert!(should_auto_open(true, false, false, 2));
        assert!(!should_auto_open(true, false, false, 3), "the fourth run never opens");
    }

    #[test]
    fn the_poll_waits_for_the_daemon_and_respects_complete_and_once_per_run() {
        assert!(!should_auto_open(false, false, false, 0), "no daemon answer yet");
        assert!(!should_auto_open(true, true, false, 0), "onboarding is complete");
        assert!(!should_auto_open(true, false, true, 0), "already opened this run");
    }
}
