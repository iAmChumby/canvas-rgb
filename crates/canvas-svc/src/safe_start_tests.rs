//! #577 sections 1-3: the verdict table, the streak, the persisted files and the mode.

use crate::safe_start::*;
use canvas_proto::api::{LastExit, SafeStartMode, SafeStartReason};
use std::fs;
use std::path::Path;

const CRASH_CODES: [i32; 6] = [
    101,
    0xC000_0409u32 as i32,
    0xC000_0005u32 as i32,
    0xC000_00FDu32 as i32,
    0x8000_0003u32 as i32,
    0xC000_0374u32 as i32,
];

// ---- section 1: every row of the table ----

#[test]
fn a_quit_notice_is_clean() {
    assert_eq!(verdict(EndCause::QuitNotice, None, Beat::AfterFirst), LastExit::Clean);
    assert_eq!(verdict(EndCause::QuitNotice, Some(0), Beat::BeforeFirst), LastExit::Clean);
}

#[test]
fn exit_zero_without_a_notice_is_clean() {
    assert_eq!(verdict(EndCause::Exited, Some(0), Beat::AfterFirst), LastExit::Clean);
    assert_eq!(verdict(EndCause::Exited, Some(0), Beat::BeforeFirst), LastExit::Clean);
}

#[test]
fn every_service_caused_kill_is_planned() {
    for cause in [EndCause::ServiceStop, EndCause::ServiceKill] {
        assert_eq!(verdict(cause, Some(1), Beat::AfterFirst), LastExit::Planned, "{cause:?}");
        assert_eq!(verdict(cause, Some(0), Beat::BeforeFirst), LastExit::Planned, "{cause:?}");
    }
}

#[test]
fn session_end_codes_and_the_session_cause_are_planned() {
    for code in [0x4001_0004u32 as i32, 0xC000_026Bu32 as i32] {
        assert_eq!(verdict(EndCause::Exited, Some(code), Beat::AfterFirst), LastExit::Planned);
    }
    assert_eq!(verdict(EndCause::SessionEnd, Some(1), Beat::AfterFirst), LastExit::Planned);
}

#[test]
fn a_stale_daemon_at_start_is_unknown() {
    assert_eq!(verdict(EndCause::StaleAtStart, Some(1), Beat::BeforeFirst), LastExit::Unknown);
}

#[test]
fn a_missed_steady_heartbeat_is_unclean_and_a_missed_first_one_is_a_startup_failure() {
    assert_eq!(verdict(EndCause::MissedHeartbeat, None, Beat::AfterFirst), LastExit::Unclean);
    assert_eq!(verdict(EndCause::MissedHeartbeat, None, Beat::BeforeFirst), LastExit::StartupFailure);
}

#[test]
fn every_crash_exit_code_is_unclean() {
    for code in CRASH_CODES {
        assert_eq!(verdict(EndCause::Exited, Some(code), Beat::AfterFirst), LastExit::Unclean, "{code:#x}");
    }
}

#[test]
fn exit_one_before_the_first_beat_is_a_startup_failure_but_after_it_is_unknown() {
    assert_eq!(verdict(EndCause::Exited, Some(1), Beat::BeforeFirst), LastExit::StartupFailure);
    assert_eq!(verdict(EndCause::Exited, Some(1), Beat::AfterFirst), LastExit::Unknown);
}

#[test]
fn any_other_code_is_unknown() {
    for code in [2i32, 42, -1, 0xC000_0008u32 as i32] {
        assert_eq!(verdict(EndCause::Exited, Some(code), Beat::AfterFirst), LastExit::Unknown, "{code}");
    }
    assert_eq!(verdict(EndCause::Exited, None, Beat::AfterFirst), LastExit::Unknown);
}

#[test]
fn only_unclean_and_startup_failure_spend_a_restart() {
    assert!(counts_toward_restart(LastExit::Unclean));
    assert!(counts_toward_restart(LastExit::StartupFailure));
    for v in [LastExit::Clean, LastExit::Planned, LastExit::Unknown] {
        assert!(!counts_toward_restart(v), "{v:?}");
    }
}

// ---- section 2: persistence, fail closed ----

#[test]
fn svc_run_is_written_at_start_and_removed_at_stop() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_svc_run(home).unwrap();
    assert!(svc_run_path(home).exists());
    let loaded = load(home, |_| true);
    assert!(loaded.leftover);
    assert_eq!(loaded.persisted, Persisted::default());
    remove_svc_run(home).unwrap();
    assert!(!svc_run_path(home).exists());
    assert!(!load(home, |_| true).leftover);
    // A second removal is a no-op.
    remove_svc_run(home).unwrap();
}

#[test]
fn a_leftover_svc_run_is_unknown_and_does_not_trigger_safe_start() {
    let dir = tempfile::tempdir().unwrap();
    write_svc_run(dir.path()).unwrap();
    let book = Book::open(dir.path(), load(dir.path(), |_| true), |_| true);
    assert_eq!(book.last_exit(), LastExit::Unknown);
    assert_eq!(book.mode(), SafeStartMode::Normal);
}

#[test]
fn an_untrusted_safe_start_file_reads_as_streak_zero_and_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let path = safe_start_path(home);
    let original = b"{\"unclean_streak\":9,\"mode\":\"safe\",\"reason\":\"crash_loop\",\"since\":\"x\"}";
    fs::write(&path, original).unwrap();
    let loaded = load(home, |p| p != path.as_path());
    assert!(loaded.untrusted_state);
    assert_eq!(loaded.persisted, Persisted::default());
    assert_eq!(fs::read(&path).unwrap(), original, "the untrusted file was changed");
    let book = Book::open(home, loaded, |_| true);
    assert_eq!(book.mode(), SafeStartMode::Normal);
}

#[test]
fn an_unparseable_or_reparse_state_file_reads_as_default() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    fs::write(safe_start_path(home), b"{not json").unwrap();
    assert_eq!(load(home, |_| true).persisted, Persisted::default());
    fs::write(safe_start_path(home), b"{\"unclean_streak\":4294967295,\"mode\":\"safe\"}").unwrap();
    assert_eq!(load(home, |_| true).persisted, Persisted::default());
}

#[test]
fn the_state_file_is_written_and_read_back_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let state = Persisted {
        unclean_streak: 2,
        mode: SafeStartMode::Safe,
        reason: Some(SafeStartReason::CrashLoop),
        since: Some("2026-11-26T00:00:00Z".into()),
        ..Default::default()
    };
    save(home, &state).unwrap();
    assert_eq!(load(home, |_| true).persisted, state);
    assert!(!safe_start_path(home).with_file_name("safe-start.json.tmp").exists());
}

// ---- sections 2-3: the streak and the decision ----

fn book(home: &Path, streak: u32, mode: SafeStartMode, reason: Option<SafeStartReason>) -> Book {
    let persisted = Persisted { unclean_streak: streak, mode, reason, since: None, ..Default::default() };
    save(home, &persisted).unwrap();
    Book::open(home, load(home, |_| true), |_| true)
}

#[test]
fn one_unclean_start_is_safe_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(dir.path(), 0, SafeStartMode::Normal, None);
    b.record_end(LastExit::Unclean, "t0");
    assert_eq!(b.persisted.unclean_streak, 1);
    assert_eq!(b.mode(), SafeStartMode::Safe);
    assert_eq!(b.reason(), Some(SafeStartReason::UncleanOnce));
    // A clean end of that safe run resets the streak, and the mode goes back to normal.
    b.record_end(LastExit::Clean, "t1");
    assert_eq!(b.mode(), SafeStartMode::Normal);
    assert_eq!(b.persisted.unclean_streak, 0);
}

#[test]
fn two_unclean_ends_in_a_row_give_crash_loop() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(dir.path(), 0, SafeStartMode::Normal, None);
    b.record_end(LastExit::Unclean, "t0");
    b.record_end(LastExit::Unclean, "t1");
    assert_eq!(b.persisted.unclean_streak, 2);
    assert_eq!(b.mode(), SafeStartMode::Safe);
    assert_eq!(b.reason(), Some(SafeStartReason::CrashLoop));
}

#[test]
fn a_planned_end_of_a_normal_run_resets_the_streak() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(dir.path(), 1, SafeStartMode::Normal, None);
    b.record_end(LastExit::Planned, "t0");
    assert_eq!(b.persisted.unclean_streak, 0);
    assert_eq!(b.mode(), SafeStartMode::Normal);
    assert_eq!(load(dir.path(), |_| true).persisted.unclean_streak, 0, "the reset was persisted");
}

#[test]
fn a_logon_does_not_reset_the_streak() {
    // A logon is the supervisor's `on_new_session`, which never touches the book. Opening the
    // book again, as a service watcher would, keeps the streak as written.
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(dir.path(), 0, SafeStartMode::Normal, None);
    b.record_end(LastExit::Unclean, "t0");
    let after_logon = Book::open(dir.path(), load(dir.path(), |_| true), |_| true);
    assert_eq!(after_logon.persisted.unclean_streak, 1);
    assert_eq!(after_logon.mode(), SafeStartMode::Safe);
}

#[test]
fn user_chosen_safe_is_sticky_until_start_normally() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(dir.path(), 0, SafeStartMode::Normal, None);
    b.set_user_choice(SafeStartMode::Safe, "t0");
    assert_eq!(b.reason(), Some(SafeStartReason::UserChose));
    b.record_end(LastExit::Clean, "t1");
    assert_eq!(b.mode(), SafeStartMode::Safe, "a clean safe run keeps the choice");
    let mut reopened = Book::open(dir.path(), load(dir.path(), |_| true), |_| true);
    assert_eq!(reopened.mode(), SafeStartMode::Safe);
    reopened.set_user_choice(SafeStartMode::Normal, "t2");
    assert_eq!(reopened.mode(), SafeStartMode::Normal);
}

#[test]
fn leaving_clears_the_streak_and_the_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(dir.path(), 2, SafeStartMode::Safe, Some(SafeStartReason::CrashLoop));
    b.set_user_choice(SafeStartMode::Normal, "t1");
    assert_eq!(b.mode(), SafeStartMode::Normal);
    assert_eq!(b.persisted.unclean_streak, 0);
    let reopened = Book::open(dir.path(), load(dir.path(), |_| true), |_| true);
    assert_eq!(reopened.mode(), SafeStartMode::Normal);
}

#[test]
fn a_startup_failure_and_an_unknown_end_do_not_turn_safe_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = book(dir.path(), 0, SafeStartMode::Normal, None);
    b.record_end(LastExit::StartupFailure, "t0");
    assert_eq!(b.mode(), SafeStartMode::Normal);
    assert_eq!(b.persisted.unclean_streak, 0);
    b.record_end(LastExit::Unknown, "t1");
    assert_eq!(b.mode(), SafeStartMode::Normal);
}
