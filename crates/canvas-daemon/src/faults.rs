//! Hardware notes (#580): the words the daemon sends, their dismissal keys, and when a key is
//! cleared. One place owns all three so the editor never has to guess which note a key belongs to.
//!
//! A fault is built only from a definite bad state (R3): OpenRGB `Down`, or an NCT open that was
//! tried and returned a problem. Connecting, unknown and unchecked states are silent. A dismissed
//! fault stays in `Status.faults` (R14) so the strip still counts it; only the page notes hide.

use canvas_proto::api::{Fault, FaultKind, LinkState};
use canvas_proto::fans::{ChipProblem, FansStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::state::App;

/// A 64-bit FNV-1a hash of some text: a stable key that does not depend on the build. Moved here
/// from `agent_run` (R7) so a note's key and a run's copy name use the same rule.
pub fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The wire code of a kind. Explicit, so a change in how the enum is printed cannot silently
/// rewrite every dismissal key.
fn kind_code(kind: FaultKind) -> &'static str {
    match kind {
        FaultKind::FanChip => "fan_chip",
        FaultKind::OpenRgbDown => "open_rgb_down",
    }
}

/// The dismissal key: hex FNV-1a over (kind, device id, cause code), with NUL between the parts.
/// The device id goes into the hash and never into the stored text (R7). A new cause gets a new
/// key, so a dismissed note comes back when its cause changes; a version in the words does not.
pub fn key(kind: FaultKind, device: Option<&str>, cause_code: &str) -> String {
    let text = format!("{}\0{}\0{cause_code}", kind_code(kind), device.unwrap_or(""));
    format!("{:016x}", fnv1a(&text))
}

/// The plain words of one note, kept together so the builder stays readable.
struct NoteWords<'a> {
    affected: &'a str,
    cause: Option<&'a str>,
    fix: &'a str,
    ignore_if: Option<&'a str>,
}

fn note(
    kind: FaultKind,
    device: Option<String>,
    cause_code: &str,
    words: NoteWords<'_>,
    dismissed: &[String],
) -> Fault {
    let key = key(kind, device.as_deref(), cause_code);
    Fault {
        kind,
        key: key.clone(),
        device,
        affected: words.affected.into(),
        cause: words.cause.map(str::to_string),
        fix: words.fix.into(),
        ignore_if: words.ignore_if.map(str::to_string),
        dismissed: dismissed.iter().any(|k| k == &key),
    }
}

/// The fan note from the NCT half's typed answer (R1, R18). A missing field or an unchecked
/// service produces no note; a healthy read produces no note.
fn fan_note(fans: &FansStatus, dismissed: &[String]) -> Option<Fault> {
    if !fans.chip_checked {
        return None;
    }
    let problem = fans.chip_problem?;
    Some(match problem {
        ChipProblem::PawnIoUnavailable => note(
            FaultKind::FanChip,
            None,
            "pawn_io_unavailable",
            NoteWords {
                affected: "Fan headers can't be driven.",
                cause: Some("The PawnIO driver, which canvas-rgb needs to reach the fan chip, is missing or did not load."),
                fix: "Install PawnIO, then restart canvas-rgb.",
                ignore_if: Some("Your motherboard sets the fans and you do not need canvas-rgb to."),
            },
            dismissed,
        ),
        ChipProblem::ChipNotFound => note(
            FaultKind::FanChip,
            None,
            "chip_not_found",
            NoteWords {
                affected: "Fan headers can't be driven.",
                cause: Some("canvas-rgb found no supported fan chip on this board."),
                fix: "GPU fans can still be set from canvas-rgb.",
                ignore_if: Some("Your motherboard has no NCT6687D chip; GPU fans are still controlled."),
            },
            dismissed,
        ),
    })
}

/// Every note the current state proves. Order is stable: fans first, then OpenRGB.
pub fn build(openrgb: LinkState, fans: Option<&FansStatus>, dismissed: &[String]) -> Vec<Fault> {
    let mut out = Vec::new();
    if let Some(fans) = fans {
        if let Some(fault) = fan_note(fans, dismissed) {
            out.push(fault);
        }
    }
    if openrgb == LinkState::Down {
        out.push(note(
            FaultKind::OpenRgbDown,
            None,
            "down",
            NoteWords {
                affected: "Lights reached through OpenRGB are not available.",
                cause: Some("OpenRGB is not running, or its port is busy or refused."),
                fix: "Start OpenRGB.",
                ignore_if: Some("You have no lights that need OpenRGB."),
            },
            dismissed,
        ));
    }
    out
}

/// Clears one dismissal key after an observed healthy state, and writes `config.toml` only when
/// the key was actually stored (R4, R16). A connecting service, an empty fans cache and an
/// unchecked chip never call this, so a restart cannot revive a note the owner hid.
pub async fn clear_if_healthy(app: &Arc<App>, dismissed_key: &str) {
    // C6: a healthy poll that has no key to clear must not take the writer lock or clone the
    // store at all. This read is the whole cost of the common case.
    if !app.store.read().await.config.dismissed_faults.iter().any(|k| k == dismissed_key) {
        return;
    }
    let wrote = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&wrote);
    let key = dismissed_key.to_string();
    let result = crate::store_io::store_save(
        app,
        move |store| {
            if store.config.dismissed_faults.iter().any(|k| k == &key) {
                store.config.dismissed_faults.retain(|k| k != &key);
                flag.store(true, Ordering::SeqCst);
            }
            Ok(())
        },
        move |store| {
            if !wrote.load(Ordering::SeqCst) {
                return Ok(());
            }
            store
                .save_config()
                .map_err(|e| crate::api::ApiFailure::internal(format!("could not save the config: {e}")))
        },
    )
    .await;
    if let Err(e) = result {
        tracing::warn!(error = %e.message, "a dismissal key could not be cleared");
    }
}

/// A fan answer with a checked, healthy chip clears every fan note key. An older service
/// (`chip_checked: false`) and a still-bad chip clear nothing (R4, R13).
pub async fn clear_fan_if_healthy(app: &Arc<App>, status: &FansStatus) {
    if !status.chip_checked || status.chip_problem.is_some() {
        return;
    }
    for cause in ["pawn_io_unavailable", "chip_not_found"] {
        clear_if_healthy(app, &key(FaultKind::FanChip, None, cause)).await;
    }
}

/// An observed OpenRGB connection clears the down note's key. `Connecting` leaves it alone.
pub async fn clear_openrgb_if_healthy(app: &Arc<App>, link: LinkState) {
    if link != LinkState::Connected {
        return;
    }
    clear_if_healthy(app, &key(FaultKind::OpenRgbDown, None, "down")).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_stable_and_changes_with_the_cause() {
        let first = key(FaultKind::OpenRgbDown, None, "down");
        assert_eq!(first, key(FaultKind::OpenRgbDown, None, "down"));
        assert_ne!(first, key(FaultKind::OpenRgbDown, None, "busy"));
        assert_ne!(first, key(FaultKind::FanChip, None, "down"));
        assert_ne!(first, key(FaultKind::OpenRgbDown, Some("a device"), "down"));
        assert_eq!(first.len(), 16);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn a_fault_appears_only_on_a_definite_bad_state_r3() {
        let dismissed = Vec::new();
        // Connecting and the service being down are not OpenRGB faults.
        assert!(build(LinkState::Connecting, None, &dismissed).is_empty());
        assert!(build(LinkState::Down, None, &dismissed).len() == 1);
        // An unchecked or empty fans answer is silent, even with a problem value.
        let mut unchecked = crate::fakes::fake_fans_status();
        unchecked.chip_checked = false;
        unchecked.chip_problem = Some(ChipProblem::PawnIoUnavailable);
        assert!(build(LinkState::Connected, Some(&unchecked), &dismissed).is_empty());
        // A checked, healthy chip is silent.
        assert!(build(LinkState::Connected, Some(&crate::fakes::fake_fans_status()), &dismissed).is_empty());
        // Both bad states together give two notes, and the strip counts both.
        let mut bad = crate::fakes::fake_fans_status();
        bad.chip_problem = Some(ChipProblem::ChipNotFound);
        assert_eq!(build(LinkState::Down, Some(&bad), &dismissed).len(), 2);
    }

    #[test]
    fn the_fan_note_says_what_the_answer_proves_r18() {
        let mut bad = crate::fakes::fake_fans_status();
        bad.chip_problem = Some(ChipProblem::PawnIoUnavailable);
        let faults = build(LinkState::Connected, Some(&bad), &[]);
        assert_eq!(faults[0].kind, FaultKind::FanChip);
        assert!(faults[0].cause.as_deref().unwrap_or("").contains("is missing or did not load"));
        assert_eq!(faults[0].fix, "Install PawnIO, then restart canvas-rgb.");
        bad.chip_problem = Some(ChipProblem::ChipNotFound);
        let faults = build(LinkState::Connected, Some(&bad), &[]);
        assert_eq!(faults[0].affected, "Fan headers can't be driven.");
        assert!(faults[0].cause.as_deref().unwrap_or("").contains("no supported fan chip on this board"));
        assert!(faults[0].ignore_if.as_deref().unwrap_or("").contains("GPU fans are still controlled"));
    }

    #[test]
    fn a_dismissed_fault_stays_in_the_list_r14() {
        let mut bad = crate::fakes::fake_fans_status();
        bad.chip_problem = Some(ChipProblem::PawnIoUnavailable);
        let first = build(LinkState::Down, Some(&bad), &[]);
        let keys: Vec<String> = first.iter().map(|f| f.key.clone()).collect();
        let again = build(LinkState::Down, Some(&bad), &keys);
        assert_eq!(again.len(), 2);
        assert!(again.iter().all(|f| f.dismissed));
    }
}
