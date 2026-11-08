//! The desk's own colours.
//!
//! `app/web/src/styles/app.css` keeps these out of `tokens.css` (they are
//! the editor canvas' colours, not the app's), so they live here beside the
//! desk that draws them. The values are the ones the CSS declares, for the
//! dark and the light theme.

use gpui::Rgba;

use crate::theme::Theme;

/// The desk's colours, for one theme. Every token is the CSS value, even
/// when the round-1 desk does not paint it yet (the case badge, the part
/// line): the table stays complete for round 2.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct DeskTokens {
    /// The plate a zone's outline sits on.
    pub zone_plate: Rgba,
    /// The keyline round a zone's outline.
    pub zone_halo: Rgba,
    /// The line a zone's outline is drawn with.
    pub zone_line: Rgba,
    /// The line a part's drawing is drawn with.
    pub part_line: Rgba,
    /// The line a landmark's drawing is drawn with: quieter than a part.
    pub landmark_line: Rgba,
    /// The rulers and the frame of the effect area.
    pub ruler_ink: Rgba,
    /// The numbers on the rulers.
    pub ruler_num: Rgba,
    /// The dark keyline under the rulers.
    pub ruler_key: Rgba,
    /// An LED that is not lit.
    pub dot_unlit: Rgba,
    /// The stage the lights stand on: dark in both themes.
    pub stage_lights: Rgba,
    /// The grid on that stage.
    pub stage_lights_grid: Rgba,
    /// The badge on a part that sits outside the frame.
    pub outside_ink: Rgba,
}

impl DeskTokens {
    /// The desk's colours for the app's theme.
    pub fn for_theme(t: &Theme) -> Self {
        if t.is_dark() {
            DeskTokens {
                zone_plate: Theme::color("rgba(0, 0, 0, 0.26)"),
                zone_halo: Theme::color("rgba(0, 0, 0, 0.5)"),
                zone_line: Theme::color("rgba(255, 255, 255, 0.92)"),
                part_line: Theme::color("rgba(255, 255, 255, 0.82)"),
                landmark_line: Theme::color("rgba(255, 255, 255, 0.5)"),
                ruler_ink: Theme::color("rgba(255, 255, 255, 0.12)"),
                ruler_num: Theme::color("rgba(255, 255, 255, 0.45)"),
                ruler_key: Theme::color("rgba(0, 0, 0, 0.5)"),
                dot_unlit: Theme::color("rgba(255, 255, 255, 0.34)"),
                stage_lights: Theme::color("#07080c"),
                stage_lights_grid: Theme::color("rgba(255, 255, 255, 0.05)"),
                outside_ink: Theme::color("rgba(255, 255, 255, 0.62)"),
            }
        } else {
            DeskTokens {
                zone_plate: Theme::color("rgba(0, 0, 0, 0.2)"),
                zone_halo: Theme::color("rgba(0, 0, 0, 0.45)"),
                zone_line: Theme::color("rgba(255, 255, 255, 0.95)"),
                part_line: Theme::color("rgba(255, 255, 255, 0.95)"),
                landmark_line: Theme::color("rgba(255, 255, 255, 0.62)"),
                ruler_ink: Theme::color("rgba(255, 255, 255, 0.16)"),
                ruler_num: Theme::color("rgba(255, 255, 255, 0.5)"),
                ruler_key: Theme::color("rgba(0, 0, 0, 0.5)"),
                dot_unlit: Theme::color("rgba(255, 255, 255, 0.5)"),
                stage_lights: Theme::color("#07080c"),
                stage_lights_grid: Theme::color("rgba(255, 255, 255, 0.05)"),
                outside_ink: Theme::color("rgba(255, 255, 255, 0.72)"),
            }
        }
    }
}
