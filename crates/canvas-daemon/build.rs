//! Build-time facts for Help & About (#574): the short commit and the UTC build date.
//!
//! Both are optional. A build without git (a source archive, a vendor drop) still compiles: no
//! environment variable is emitted, and the daemon reports the field as absent. Failure here
//! never fails the build.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // A commit changes HEAD (or the packed refs) without touching this crate's sources.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");

    if let Some(commit) = git(&["rev-parse", "--short", "HEAD"]) {
        println!("cargo:rustc-env=CANVAS_BUILD_COMMIT={commit}");
    }
    println!("cargo:rustc-env=CANVAS_BUILD_DATE={}", utc_date());
}

/// Runs git quietly, returning trimmed stdout only on success.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Today's UTC date as `YYYY-MM-DD`, from the system clock (civil date from days since epoch).
fn utc_date() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_has_the_shape_a_reader_sees() {
        let date = utc_date();
        assert_eq!(date.len(), 10);
        assert!(date.chars().nth(4) == Some('-') && date.chars().nth(7) == Some('-'));
        assert!(date[..4].chars().all(|c| c.is_ascii_digit()));
    }
}
