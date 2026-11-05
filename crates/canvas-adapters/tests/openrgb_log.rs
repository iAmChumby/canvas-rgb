//! A guard on the OpenRGB adapter's mode line (#86).
//!
//! The keepalive sends `SET_CUSTOM_MODE` again every 10 s per controller, and that line is a
//! DEBUG one: it goes nowhere in a normal run. The name and the location of the controller were
//! copied before the level was looked at, so every keepalive paid for a copy that was thrown
//! away. The level is checked first now.
//!
//! The scan reads the source of the adapter. It lives in its own file, so the needles below are
//! not in the file that is scanned.

/// The source of the OpenRGB adapter, the mode line included.
const OPENRGB: &str = include_str!("../src/openrgb.rs");

/// The level check that comes before the copy.
const LEVEL_CHECK: &str = "tracing::enabled!";

/// The call that copies the name and the location of the controller.
const COPY: &str = "controller_wire(index)";

/// The level is looked at before the name and the location are copied.
#[test]
fn the_level_is_checked_before_the_controller_is_read() {
    // The first mode line (INFO) and the keepalive line (DEBUG): each copies the name and the
    // location only after its own level was looked at.
    let first_check = OPENRGB.find(LEVEL_CHECK).expect("the level check");
    let first_copy = OPENRGB.find(COPY).expect("the copy");
    assert!(first_check < first_copy, "the level comes before the copy");
    let last_check = OPENRGB.rfind(LEVEL_CHECK).expect("the level check");
    let last_copy = OPENRGB.rfind(COPY).expect("the copy");
    assert!(
        last_check < last_copy,
        "the keepalive's level comes before its copy"
    );
}
