//! A guard on the identify log's own text (#86).
//!
//! The identify line says one line per request, and the page's cadence decides how many lines
//! that is: a chase posts an identify every 125 ms for 40 steps, a whole-part light every 2 s.
//! The text in `src/identify.rs` used to say "about every 2 s" for every request, which is the
//! cadence of the whole-part light only.
//!
//! The scan reads the source of the log module. It lives in its own file, so the needles below
//! are not in the file that is scanned.

/// The source of the identify module, the log text included.
const IDENTIFY: &str = include_str!("../src/identify.rs");

/// The cadence of a chase: the page posts an identify every 125 ms.
const CHASE: &str = "125 ms";

/// The cadence of a whole-part light: the page posts one every 2 s.
const WHOLE_PART: &str = "2 s";

/// The log's text must say how often the page posts, so a reader knows how many lines a click
/// makes.
#[test]
fn the_log_text_says_how_often_the_page_posts() {
    assert!(IDENTIFY.contains(CHASE), "the chase cadence is named");
    assert!(
        IDENTIFY.contains(WHOLE_PART),
        "the whole-part cadence is named"
    );
    // The stale sentence claimed one cadence for every request. It is put together here, so this
    // test does not carry it itself.
    let stale = format!("resends about every {} s", 2);
    assert!(!IDENTIFY.contains(&stale), "the stale cadence is gone");
}
