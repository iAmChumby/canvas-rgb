//! Stage S3 of #376: one ring write on the real pad, with the saved profile put back afterwards.
//!
//! The `ring_s3` example drives this. It **uses the ring adapter's own write path**: the same
//! checks ([`ring::prepare_from`]), the same chunks and commit ([`ring::put_bytes`]), the same
//! read-back and restore-on-mismatch ([`ring::put_and_verify`]) and the same exact restore
//! ([`ring::restore_exact`]). There is no second implementation of any of them here; this file
//! only orders them and talks to the person.
//!
//! The run:
//! 1. Read the profile image (36 read requests). Save it through the caller's backup hook, which
//!    must succeed before anything is written. Refuse unless the expected profile is the active
//!    one and its custom ring record is marked in use.
//! 2. Write all 24 LEDs as pure red, packed with the byte order the caller names
//!    ([`ring::pack_ring_colours_as`]), commit, read the image back and check it. A mismatch
//!    restores and ends the run with an error.
//! 3. Say the rings should be red and wait (the caller's `hold` hook: Enter, or a fixed time).
//! 4. Restore the saved image exactly, commit, read the image back, and confirm it is
//!    byte-identical to the backup.
//!
//! On an error at any step after the first write, the image is restored and the error says so. A
//! restore that cannot be confirmed is an error that names the backup, and the escape hatch is
//! [`restore_backup`] (`--restore-from FILE`), which puts a backup back and does nothing else.
//!
//! **This writes the pad.** The ring write is the first one ever sent to the rings: the commit
//! argument and the record layout are static knowledge (research 420), and the pad may refuse
//! them. Nothing here has run on a pad.

use canvas_proto::color::Rgb8;
use canvas_proto::traits::AdapterError;

use super::probe::ProfileCapture;
use super::ring::{self, ByteOrder, RING_LEDS};
use super::{read_image_raw, refuse, HidOpener, Pace, VendorTransport};

/// What the tool is asked to do.
#[derive(Clone, Copy, Debug)]
pub struct S3Options {
    /// The byte order to try for the colour words.
    pub order: ByteOrder,
    /// The profile the pad must be on (0-based). The pad's own active profile decides which
    /// record is written, so a mismatch refuses.
    pub expect_profile: usize,
}

/// What the tool needs from its caller. Closures, so a test drives it with no terminal or file.
pub struct S3Io<'a> {
    /// Says a line to the person.
    pub say: &'a mut dyn FnMut(&str),
    /// Saves the backup, durably, before anything is written. An error ends the run with nothing
    /// written.
    pub backup: &'a mut dyn FnMut(&ProfileCapture) -> Result<(), String>,
    /// Waits while the rings show red: for Enter, or a fixed time. An error ends the wait early
    /// and restores.
    pub hold: &'a mut dyn FnMut() -> Result<(), String>,
}

/// How the run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3Outcome {
    /// The pad already held pure red in this byte order: nothing was written and nothing needs
    /// restoring.
    AlreadyRed,
    /// Red was written, held and the saved image is back, byte for byte.
    Done(ring::Restored),
}

/// The 24 LEDs of pure red.
fn red_words(order: ByteOrder) -> [[u8; 4]; super::probe::CUSTOM_COLOURS] {
    ring::pack_ring_colours_as(&[Rgb8::new(255, 0, 0); RING_LEDS], order)
}

/// Runs S3 on an open vendor channel. Blocking.
pub(super) fn run(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    opts: &S3Options,
    io: &mut S3Io<'_>,
) -> Result<S3Outcome, AdapterError> {
    (io.say)("reading the pad's profile image: 36 read requests, nothing written yet");
    let (saved, replies) = read_image_raw(vendor, pace)?;
    let capture = ProfileCapture {
        personality: Some("XInput".to_string()),
        requests_sent: replies.len(),
        replies,
        image: saved.clone(),
    };
    let prepared = ring::prepare_from(saved, &red_words(opts.order), Some(opts.expect_profile))?;
    (io.say)(&format!(
        "profile {} is active and its custom ring record is marked in use",
        prepared.active + 1
    ));
    (io.backup)(&capture)
        .map_err(|e| refuse(format!("the backup could not be saved ({e}); nothing was written")))?;
    (io.say)("the backup is saved");
    if prepared.is_noop() {
        (io.say)("the pad already holds pure red in this order; nothing to write");
        return Ok(S3Outcome::AlreadyRed);
    }

    (io.say)(&format!(
        "writing 24 LEDs of pure red ({:?} order): {} chunks, a commit, then a read-back",
        opts.order,
        prepared.record.len().div_ceil(super::codec::CHUNK)
    ));
    // The adapter's own write: on a failed write or a mismatched read-back it has already
    // restored, and its error says how that went.
    if let Err(e) = ring::put_and_verify(vendor, pace, &prepared) {
        (io.say)(&format!("the write failed and the saved image was put back: {e}"));
        return Err(e);
    }
    (io.say)("the pad's image reads back as written: only the ring record changed");
    (io.say)("rings should be RED now; press Enter to restore");

    if let Err(why) = (io.hold)() {
        (io.say)(&format!("the wait ended with an error ({why}); restoring"));
        return finish_restore(vendor, pace, &prepared.saved, io, Some(why));
    }
    finish_restore(vendor, pace, &prepared.saved, io, None)
}

fn finish_restore(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    saved: &[u8],
    io: &mut S3Io<'_>,
    cause: Option<String>,
) -> Result<S3Outcome, AdapterError> {
    (io.say)("restoring the saved image: write back, commit, read back");
    match ring::restore_exact(vendor, pace, saved) {
        Ok(how) => {
            (io.say)("the image reads back byte-identical to the backup; the rings are as they were");
            match cause {
                None => Ok(S3Outcome::Done(how)),
                Some(why) => Err(refuse(format!("the wait failed ({why}); the saved image was restored"))),
            }
        }
        Err(e) => {
            (io.say)(&format!(
                "THE RESTORE COULD NOT BE CONFIRMED: {e}. Run ring_s3 --restore-from <the backup file>"
            ));
            Err(e)
        }
    }
}

/// Puts a backup back and does nothing else: the escape hatch. Blocking.
///
/// The backup must be a ring probe or S3 backup file (every reply checked, 1592 bytes). The tool
/// says how far the pad's image is from it before writing, writes only the span that differs,
/// and confirms by reading the image again.
pub(super) fn restore_backup(
    vendor: &mut dyn VendorTransport,
    pace: Pace,
    backup: &ProfileCapture,
    say: &mut dyn FnMut(&str),
) -> Result<ring::Restored, AdapterError> {
    if super::codec::active_profile(&backup.image).is_none() {
        return Err(refuse("the backup names no active profile; it is not a profile image of this pad"));
    }
    say("reading the pad's profile image: 36 read requests");
    let (now, _) = read_image_raw(vendor, pace)?;
    if now == backup.image {
        say("the pad's image already matches the backup; nothing to write");
    } else {
        say(&format!("the pad's image differs from the backup: {}", ring::diff_summary(&backup.image, &now)));
        say("writing back the span that differs, then a commit and a read-back");
    }
    let how = ring::restore_exact(vendor, pace, &backup.image)?;
    say("the image reads back byte-identical to the backup");
    Ok(how)
}

/// Runs S3 on the real pad. Blocking.
pub fn run_on_hardware(opts: &S3Options, io: &mut S3Io<'_>) -> Result<S3Outcome, AdapterError> {
    let (mut vendor, _) = super::probe::open_pad_vendor(&HidOpener::<hidapi::HidApi>::default())?;
    run(vendor.as_mut(), Pace::default(), opts, io)
}

/// Restores a backup onto the real pad. Blocking.
pub fn restore_backup_on_hardware(
    backup: &ProfileCapture,
    say: &mut dyn FnMut(&str),
) -> Result<ring::Restored, AdapterError> {
    let (mut vendor, _) = super::probe::open_pad_vendor(&HidOpener::<hidapi::HidApi>::default())?;
    restore_backup(vendor.as_mut(), Pace::default(), backup, say)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::eightbitdo_ultimate2::{codec, Opener, Personality};
    use crate::fake_pad::{self, FakePad};

    fn pace() -> Pace {
        Pace {
            poll_ms: 1,
            chunk_pause: Duration::ZERO,
            write_pause: Duration::ZERO,
            session_pause: Duration::ZERO,
        }
    }

    fn opts(order: ByteOrder) -> S3Options {
        // The fixture's active profile is profile 3.
        S3Options { order, expect_profile: 2 }
    }

    /// The custom record of the fixture's active profile in an image.
    fn record_of(img: &[u8]) -> Vec<u8> {
        let at = codec::SINGLE + ring::SINGLE_LEN * 2;
        img[at..at + ring::SINGLE_LEN].to_vec()
    }

    struct Run {
        result: Result<S3Outcome, AdapterError>,
        said: Vec<String>,
        backups: Vec<ProfileCapture>,
        /// The pad's image while the rings were held.
        held_image: Option<Vec<u8>>,
    }

    fn drive(pad: &FakePad, order: ByteOrder, hold: Result<(), String>, save: Result<(), String>) -> Run {
        let mut vendor = pad.vendor();
        let mut said = Vec::new();
        let mut backups = Vec::new();
        let mut held_image = None;
        let result = {
            let mut say = |l: &str| said.push(l.to_string());
            let mut backup = |c: &ProfileCapture| {
                backups.push(c.clone());
                save.clone()
            };
            let mut hold = || {
                held_image = Some(pad.image());
                hold.clone()
            };
            let mut io = S3Io { say: &mut say, backup: &mut backup, hold: &mut hold };
            run(&mut vendor, pace(), &opts(order), &mut io)
        };
        Run { result, said, backups, held_image }
    }

    fn fresh() -> FakePad {
        FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput)
    }

    #[test]
    fn the_happy_path_writes_red_holds_and_restores_the_image_byte_for_byte() {
        let pad = fresh();
        let before = pad.image();
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Ok(()));
        assert_eq!(r.result.unwrap(), S3Outcome::Done(ring::Restored::Range));
        // The backup was taken from the pad before anything was written, and holds the image.
        assert_eq!(r.backups.len(), 1);
        assert_eq!(r.backups[0].image, before);
        assert_eq!(r.backups[0].replies.len(), 36);
        // While the rings were held, the pad held 24 words of red in BGR order (file order
        // 00 00 ff 00), the flag was still marked in use, and nothing else had moved.
        let held = r.held_image.expect("the run waited");
        let record = record_of(&held);
        assert_eq!(&record[..4], &codec::ENABLED.to_le_bytes());
        for i in 0..24 {
            assert_eq!(&record[4 + 4 * i..8 + 4 * i], &[0x00, 0x00, 0xFF, 0x00], "word {i}");
        }
        let at = codec::SINGLE + ring::SINGLE_LEN * 2;
        assert_eq!(&held[..at], &before[..at]);
        assert_eq!(&held[at + ring::SINGLE_LEN..], &before[at + ring::SINGLE_LEN..]);
        // After the restore the pad's image is the backup, byte for byte.
        assert_eq!(pad.image(), r.backups[0].image);
        assert_eq!(pad.image(), before);
        assert!(r.said.iter().any(|l| l == "rings should be RED now; press Enter to restore"));
        assert!(r.said.iter().any(|l| l.contains("byte-identical to the backup")));
        // Reports: 3 chunks + commit for the write, 1 chunk-run + commit for the restore.
        let (_, writes, commits, _) = pad.report_counts();
        assert_eq!(commits, 2, "one commit for the write, one for the restore");
        // The record in three chunks; the restore writes only the span that differs (words 1-22
        // of the fixture: its first and last LEDs are already red), 88 bytes in two chunks.
        assert_eq!(writes, 3 + 2);
    }

    #[test]
    fn the_rgb_order_writes_red_as_ff_00_00() {
        let pad = fresh();
        let r = drive(&pad, ByteOrder::Rgb, Ok(()), Ok(()));
        r.result.unwrap();
        let record = record_of(&r.held_image.unwrap());
        assert_eq!(&record[4..8], &[0xFF, 0x00, 0x00, 0x00]);
        assert_eq!(pad.image(), fake_pad::ring_fixture_image());
    }

    #[test]
    fn the_backup_is_saved_before_the_first_write_and_a_failed_save_writes_nothing() {
        let pad = fresh();
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Err("the disk is full".into()));
        let err = r.result.unwrap_err().to_string();
        assert!(err.contains("backup could not be saved") && err.contains("nothing was written"), "{err}");
        let (reads, writes, commits, states) = pad.report_counts();
        assert_eq!((writes, commits, states), (0, 0, 0), "not one write-side report");
        assert_eq!(reads, 36);
        assert!(r.held_image.is_none(), "it never got as far as the red");
    }

    #[test]
    fn a_failed_commit_restores_the_image() {
        let pad = fresh();
        pad.silence_next_commits(1);
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Ok(()));
        let err = r.result.unwrap_err().to_string();
        assert!(err.contains("commit"), "{err}");
        assert!(err.contains("read back as saved"), "{err}");
        assert_eq!(pad.image(), r.backups[0].image, "the pad's image is the backup again");
        assert!(r.held_image.is_none(), "no wait after a failed write");
    }

    #[test]
    fn a_dropped_write_acknowledgement_restores_the_image() {
        let pad = fresh();
        pad.silence_next_write_acks(1);
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Ok(()));
        let err = r.result.unwrap_err().to_string();
        assert!(err.contains("write chunk"), "{err}");
        assert_eq!(pad.image(), r.backups[0].image, "the half-written record was put back");
    }

    #[test]
    fn a_pad_that_drops_the_write_restores_and_says_the_read_back_differed() {
        let pad = fresh();
        pad.drop_next_commit();
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Ok(()));
        let err = r.result.unwrap_err().to_string();
        assert!(err.contains("did not read back as written"), "{err}");
        assert!(err.contains("byte(s) differ"), "the error names where the image differs: {err}");
        assert_eq!(pad.image(), r.backups[0].image);
    }

    #[test]
    fn a_pad_that_acknowledges_but_does_not_take_the_write_restores_and_never_waits() {
        let pad = fresh();
        pad.ignore_writes();
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Ok(()));
        let err = r.result.unwrap_err().to_string();
        assert!(err.contains("did not read back as written"), "{err}");
        assert_eq!(pad.image(), r.backups[0].image);
        assert!(r.held_image.is_none(), "the rings were never said to be red");
        assert!(!r.said.iter().any(|l| l.contains("should be RED")));
    }

    #[test]
    fn a_pad_that_changes_a_byte_elsewhere_is_restored_whole() {
        let pad = fresh();
        pad.scribble_on_next_commit(codec::NAME);
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Ok(()));
        let err = r.result.unwrap_err().to_string();
        assert!(err.contains("did not read back as written"), "{err}");
        assert_eq!(pad.image(), r.backups[0].image, "the stray byte is back too");
    }

    #[test]
    fn a_wait_that_fails_restores_the_image_and_says_so() {
        let pad = fresh();
        let r = drive(&pad, ByteOrder::Bgr, Err("stdin closed".into()), Ok(()));
        let err = r.result.unwrap_err().to_string();
        assert!(err.contains("stdin closed") && err.contains("restored"), "{err}");
        assert_eq!(pad.image(), r.backups[0].image);
        assert!(r.said.iter().any(|l| l.contains("byte-identical")));
    }

    #[test]
    fn a_restore_that_cannot_be_confirmed_is_an_error_that_names_the_escape_hatch() {
        let pad = fresh();
        // The pad takes the red, then stops taking writes, so the restore cannot land.
        let mut vendor = pad.vendor();
        let mut said = Vec::new();
        let mut say = |l: &str| said.push(l.to_string());
        let mut backup = |_: &ProfileCapture| Ok(());
        let p2 = pad.clone();
        let mut hold = || {
            p2.ignore_writes();
            Ok(())
        };
        let mut io = S3Io { say: &mut say, backup: &mut backup, hold: &mut hold };
        let err = run(&mut vendor, pace(), &opts(ByteOrder::Bgr), &mut io).unwrap_err();
        assert!(err.to_string().contains("still differs"), "{err}");
        assert!(said.iter().any(|l| l.contains("RESTORE COULD NOT BE CONFIRMED") && l.contains("--restore-from")));
        assert_ne!(pad.image(), fake_pad::ring_fixture_image(), "it really did not come back");
    }

    #[test]
    fn a_profile_that_is_not_the_expected_one_or_has_no_custom_record_is_refused_with_nothing_written() {
        let pad = fresh();
        let mut vendor = pad.vendor();
        let mut said = Vec::new();
        let mut say = |l: &str| said.push(l.to_string());
        let mut backup = |_: &ProfileCapture| Ok(());
        let mut hold = || Ok(());
        let mut io = S3Io { say: &mut say, backup: &mut backup, hold: &mut hold };
        let wrong = S3Options { order: ByteOrder::Bgr, expect_profile: 0 };
        let err = run(&mut vendor, pace(), &wrong, &mut io).unwrap_err().to_string();
        assert!(err.contains("active profile is 3, not 1"), "{err}");
        assert_eq!(pad.report_counts().1, 0);

        let mut image = fake_pad::ring_fixture_image();
        let at = codec::SINGLE + ring::SINGLE_LEN * 2;
        image[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
        let pad = FakePad::with_image(image, Personality::XInput);
        let mut vendor = pad.vendor();
        let mut io = S3Io { say: &mut say, backup: &mut backup, hold: &mut hold };
        let err = run(&mut vendor, pace(), &opts(ByteOrder::Bgr), &mut io).unwrap_err().to_string();
        assert!(err.contains("not marked in use"), "{err}");
        let (_, writes, commits, states) = pad.report_counts();
        assert_eq!((writes, commits, states), (0, 0, 0));
    }

    #[test]
    fn a_pad_that_already_shows_red_is_not_written_and_not_restored() {
        let mut image = fake_pad::ring_fixture_image();
        let at = codec::SINGLE + ring::SINGLE_LEN * 2;
        for (i, w) in red_words(ByteOrder::Bgr).iter().enumerate() {
            image[at + 4 + 4 * i..at + 8 + 4 * i].copy_from_slice(w);
        }
        let pad = FakePad::with_image(image, Personality::XInput);
        let r = drive(&pad, ByteOrder::Bgr, Ok(()), Ok(()));
        assert_eq!(r.result.unwrap(), S3Outcome::AlreadyRed);
        assert_eq!(pad.report_counts().1, 0);
        assert!(r.held_image.is_none());
    }

    #[test]
    fn restore_from_puts_a_backup_back_and_the_image_equals_it() {
        // A pad left red by an earlier run, and the backup that run took.
        let backup = fake_pad::capture_of(&fake_pad::ring_fixture_image(), Personality::XInput);
        let mut red = fake_pad::ring_fixture_image();
        let at = codec::SINGLE + ring::SINGLE_LEN * 2;
        for (i, w) in red_words(ByteOrder::Bgr).iter().enumerate() {
            red[at + 4 + 4 * i..at + 8 + 4 * i].copy_from_slice(w);
        }
        let pad = FakePad::with_image(red.clone(), Personality::XInput);
        assert_ne!(pad.image(), backup.image);
        let mut vendor = pad.vendor();
        let mut said = Vec::new();
        let how = restore_backup(&mut vendor, pace(), &backup, &mut |l| said.push(l.to_string())).unwrap();
        assert_eq!(how, ring::Restored::Range);
        assert_eq!(pad.image(), backup.image, "byte-identical to the backup");
        assert!(said.iter().any(|l| l.contains("differs from the backup")), "{said:?}");
        // Only the span that differs was written: two chunks and one commit.
        let (_, writes, commits, _) = pad.report_counts();
        assert_eq!((writes, commits), (2, 1));
        // A pad that already matches is not written at all.
        let pad = fresh();
        let mut vendor = pad.vendor();
        let how = restore_backup(&mut vendor, pace(), &backup, &mut |_| {}).unwrap();
        assert_eq!(how, ring::Restored::AlreadyMatched);
        assert_eq!(pad.report_counts().1, 0);
    }

    #[test]
    fn a_backup_read_from_its_file_text_restores_the_same_image() {
        // The example saves ProfileCapture::to_text and --restore-from reads it back.
        let pad = fresh();
        let before = pad.image();
        let mut vendor = pad.vendor();
        let mut said = Vec::new();
        let mut backups = Vec::new();
        let mut say = |l: &str| said.push(l.to_string());
        let mut backup = |c: &ProfileCapture| {
            backups.push(c.to_text());
            Ok(())
        };
        let p2 = pad.clone();
        // Leave the pad red: the wait ends and then the restore is made to fail by the pad, so the
        // red stays, as it would after a killed run.
        let mut hold = || {
            p2.ignore_writes();
            Ok(())
        };
        let mut io = S3Io { say: &mut say, backup: &mut backup, hold: &mut hold };
        assert!(run(&mut vendor, pace(), &opts(ByteOrder::Bgr), &mut io).is_err());
        assert_ne!(pad.image(), before);
        // A later run: the pad takes writes again; the backup comes from the saved text.
        pad.take_writes();
        let parsed = ProfileCapture::from_text(&backups[0]).expect("the saved text parses");
        let mut vendor = pad.vendor();
        restore_backup(&mut vendor, pace(), &parsed, &mut |_| {}).unwrap();
        assert_eq!(pad.image(), before, "the image equals the backup read from the file text");
        assert_eq!(pad.image(), parsed.image);
    }

    #[test]
    fn a_backup_that_is_not_a_profile_image_is_refused_before_any_write() {
        let pad = fresh();
        let mut vendor = pad.vendor();
        let mut bad = fake_pad::capture_of(&fake_pad::ring_fixture_image(), Personality::XInput);
        bad.image = vec![0xEE; codec::IMAGE_LEN];
        let err = restore_backup(&mut vendor, pace(), &bad, &mut |_| {}).unwrap_err().to_string();
        assert!(err.contains("no active profile"), "{err}");
        assert!(pad.sent_reports().is_empty());
        let _ = pad.open_vendor(Personality::XInput);
    }

    // ---- on the image the real pad gave (profile 1 active)

    fn real_pad() -> FakePad {
        FakePad::from_capture(&fake_pad::real_profile_capture(), Personality::XInput)
    }

    fn real_opts(order: ByteOrder) -> S3Options {
        S3Options { order, expect_profile: 0 }
    }

    fn drive_real(pad: &FakePad, order: ByteOrder) -> (Result<S3Outcome, AdapterError>, Option<Vec<u8>>, Vec<ProfileCapture>) {
        let mut vendor = pad.vendor();
        let mut backups = Vec::new();
        let mut held = None;
        let result = {
            let mut say = |_: &str| {};
            let mut backup = |c: &ProfileCapture| {
                backups.push(c.clone());
                Ok(())
            };
            let mut hold = || {
                held = Some(pad.image());
                Ok(())
            };
            let mut io = S3Io { say: &mut say, backup: &mut backup, hold: &mut hold };
            run(&mut vendor, pace(), &real_opts(order), &mut io)
        };
        (result, held, backups)
    }

    #[test]
    fn on_the_real_pads_image_red_is_written_to_profile_1_and_the_image_comes_back_identical() {
        for order in [ByteOrder::Bgr, ByteOrder::Rgb] {
            let pad = real_pad();
            let real = fake_pad::real_profile_capture().image;
            let (result, held, backups) = drive_real(&pad, order);
            result.unwrap();
            assert_eq!(backups[0].image, real, "the backup is the pad's image");
            let held = held.unwrap();
            let at = codec::SINGLE; // profile 1
            let red: [u8; 4] = if order == ByteOrder::Bgr { [0, 0, 0xFF, 0] } else { [0xFF, 0, 0, 0] };
            for i in 0..24 {
                assert_eq!(&held[at + 4 + 4 * i..at + 8 + 4 * i], &red, "{order:?} word {i}");
            }
            // Only the 24 words of profile 1's custom record changed.
            assert_eq!(&held[..at + 4], &real[..at + 4]);
            assert_eq!(&held[at + ring::SINGLE_LEN..], &real[at + ring::SINGLE_LEN..]);
            assert_eq!(pad.image(), real, "{order:?}: byte-identical after the restore");
            // The real image's 24 words are all `ff 00 8b 00`, so the write is 24 changed words:
            // one record span, two or three chunks each way.
            let (_, _, commits, _) = pad.report_counts();
            assert_eq!(commits, 2);
        }
    }

    #[test]
    fn on_the_real_image_every_failure_mode_ends_with_the_image_equal_to_the_backup() {
        let real = fake_pad::real_profile_capture().image;
        type Arm = fn(&FakePad);
        let modes: [(&str, Arm); 5] = [
            ("failed commit", |p| p.silence_next_commits(1)),
            ("dropped write ack", |p| p.silence_next_write_acks(1)),
            ("dropped write", |p| p.drop_next_commit()),
            ("write not taken", |p| p.ignore_writes()),
            ("stray byte", |p| p.scribble_on_next_commit(codec::NAME)),
        ];
        for (name, arm) in modes {
            let pad = real_pad();
            arm(&pad);
            let (result, held, _) = drive_real(&pad, ByteOrder::Bgr);
            assert!(result.is_err(), "{name}: the run reports the failure");
            assert!(held.is_none(), "{name}: the rings are never said to be red");
            assert_eq!(pad.image(), real, "{name}: the image is the backup again");
        }
    }
}
