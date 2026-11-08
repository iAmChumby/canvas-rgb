//! Views. Displays is the first slice (G7); the other pages render the
//! "Not ported yet" placeholder from the shell.

pub mod command_palette;
pub mod devices;
pub mod displays;
pub mod effects;
pub mod fans;
pub mod layout;
pub mod onboarding;
pub mod peripherals;
pub mod settings;
pub mod updates;

/// Debug builds only: `CANVAS_RGB_CAPTURE_SCROLL=<px>` (set by
/// `tools/ui-capture/capture-gpui.ps1 -Scroll`) shifts a page up by that many
/// pixels, so a capture can reach the panels below the fold (#205). The pages
/// clip rather than scroll, so the shift is the whole of the scrolling a shot
/// needs. Release builds always answer 0.
pub fn capture_scroll() -> f32 {
    if cfg!(debug_assertions) {
        std::env::var("CANVAS_RGB_CAPTURE_SCROLL")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(0.0)
    } else {
        0.0
    }
}
