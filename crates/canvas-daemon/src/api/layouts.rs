//! The layout routes: list, read, replace and activate.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{LayoutsResponse, SetActiveLayout};
use canvas_proto::device::{DeviceInfo, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// The LED count of a zone, when the device that has it is known.
fn zone_led_count(devices: &[DeviceInfo], zone: &PlacedZone) -> Option<u32> {
    devices
        .iter()
        .find(|d| d.id == zone.device_id)
        .and_then(|d| d.zones.iter().find(|z| z.index == zone.zone_index))
        .map(|z| z.led_count)
}

/// True when the zone is a matrix zone: those are never split, their map owns every LED.
fn is_matrix(devices: &[DeviceInfo], zone: &PlacedZone) -> bool {
    devices
        .iter()
        .find(|d| d.id == zone.device_id)
        .and_then(|d| d.zones.iter().find(|z| z.index == zone.zone_index))
        .is_some_and(|z: &ZoneInfo| matches!(z.kind, ZoneKind::Matrix { .. }))
}

/// Checks a layout against the devices, before it is saved.
///
/// Two things are refused, with a plain message the editor can show as it is:
/// - two placements of one zone that claim the same LEDs, and
/// - a slice of a zone that runs past the end of the zone.
///
/// A slice of a matrix zone is not an error: a matrix zone is never split, so its map
/// owns every LED and the slice is ignored.
pub fn validate_layout(layout: &Layout, devices: &[DeviceInfo]) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();

    // The placements of each zone, in the order they appear in the layout.
    let mut by_zone: BTreeMap<(String, u32), Vec<&PlacedZone>> = BTreeMap::new();
    for zone in &layout.zones {
        by_zone.entry((zone.device_id.0.clone(), zone.zone_index)).or_default().push(zone);
    }

    for ((device, zone_index), placements) in &by_zone {
        // What each placement claims: the first LED and the number of LEDs.
        let mut claims: Vec<(u64, u64)> = Vec::new();
        for placement in placements {
            let Some(slice) = placement.leds else {
                // The placement owns the whole zone. Any slice beside it overlaps it.
                for (start, count) in &claims {
                    errors.push(format!(
                        "{} zone {}: one placement owns the whole zone and another owns LEDs {}..{}, so they overlap",
                        device,
                        zone_index,
                        start,
                        start + count
                    ));
                }
                let count = zone_led_count(devices, placement).unwrap_or(u32::MAX) as u64;
                claims.push((0, count));
                continue;
            };
            if slice.count == 0 {
                continue;
            }
            if is_matrix(devices, placement) {
                // A matrix zone is never split: the map owns every LED.
                tracing::warn!(
                    device = %device,
                    zone = zone_index,
                    "a matrix zone is never split; the slice of LEDs on it is ignored"
                );
                continue;
            }
            let start = slice.start as u64;
            let count = slice.count as u64;
            if let Some(led_count) = zone_led_count(devices, placement) {
                if start + count > led_count as u64 {
                    errors.push(format!(
                        "{} zone {}: the slice of LEDs {}..{} runs past the end of the zone, which has {} LEDs",
                        device,
                        zone_index,
                        start,
                        start + count,
                        led_count
                    ));
                }
            }
            for (other_start, other_count) in &claims {
                if start < other_start + other_count && *other_start < start + count {
                    errors.push(format!(
                        "{} zone {}: two placements claim the same LEDs ({}..{} and {}..{}), so they overlap",
                        device,
                        zone_index,
                        other_start,
                        other_start + other_count,
                        start,
                        start + count
                    ));
                }
            }
            claims.push((start, count));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// The slices of a layout that no longer fit their zone.
///
/// The zone was resized under the layout, so the slice has to be mapped again. The
/// layout still loads: the LEDs that are gone are dark, and the ones that are left
/// keep their place.
pub fn slices_needing_mapping(layout: &Layout, devices: &[DeviceInfo]) -> Vec<String> {
    let mut out = Vec::new();
    for zone in &layout.zones {
        let Some(slice) = zone.leds else { continue };
        if slice.count == 0 || is_matrix(devices, zone) {
            continue;
        }
        let Some(led_count) = zone_led_count(devices, zone) else { continue };
        let start = slice.start as u64;
        let count = slice.count as u64;
        if start + count > led_count as u64 {
            out.push(format!(
                "{} zone {}: the slice of LEDs {}..{} needs {} LEDs, but the zone has {}",
                zone.device_id.0,
                zone.zone_index,
                start,
                start + count,
                count,
                led_count
            ));
        }
    }
    out
}

/// `GET /api/v1/layouts`.
pub async fn list_layouts(State(app): State<Arc<App>>) -> Json<LayoutsResponse> {
    let store = app.store.read().await;
    let layouts = store.layouts.layouts.clone();
    let active = store.layouts.active.clone();
    Json(LayoutsResponse { active, layouts })
}

/// `GET /api/v1/layouts/{id}`.
pub async fn get_layout(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Layout>> {
    let store = app.store.read().await;
    let layout = store
        .layouts
        .layouts
        .iter()
        .find(|l| l.id == id)
        .cloned()
        .ok_or_else(|| ApiFailure::not_found(format!("no layout with the id {id}")))?;
    Ok(Json(layout))
}

/// `PUT /api/v1/layouts/{id}`.
///
/// Replaces a layout. When it is the active one, the engine gets it.
pub async fn put_layout(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(layout): Json<Layout>,
) -> ApiResult<Json<Layout>> {
    if layout.id != id {
        return Err(ApiFailure::validation(format!(
            "the layout id in the body ({}) does not match the path ({id})",
            layout.id
        )));
    }
    // The layout is checked against the devices before it is saved: two placements
    // that claim the same LEDs, and a slice that runs past the end of its zone, are
    // refused with a plain message.
    let devices = app.devices.read().await.clone();
    for message in slices_needing_mapping(&layout, &devices) {
        tracing::warn!(%message, "a slice of the layout needs mapping again");
    }
    if let Err(message) = validate_layout(&layout, &devices) {
        return Err(ApiFailure::validation(message));
    }
    {
        let mut store = app
            .store
            .write()
            .await;
        match store.layouts.layouts.iter_mut().find(|l| l.id == id) {
            Some(existing) => *existing = layout.clone(),
            None => store.layouts.layouts.push(layout.clone()),
        }
        store
            .save_layouts()
            .map_err(|e| ApiFailure::internal(format!("could not save the layouts: {e}")))?;
    }

    // The active layout goes to the engine straight away.
    let active = app.store.read().await.layouts.active.clone();
    if active == id {
        app.engine.set_layout(layout.clone(), devices);
        if let Some(effect) = &layout.effect {
            crate::logging::warn_if_err(
                &format!("the saved layout was updated but its effect {} did not start", effect.id),
                crate::apply_effect(&app, &effect.id, &effect.props).await,
            );
        }
    }

    Ok(Json(layout))
}

/// `PUT /api/v1/layouts/active`.
///
/// Switches to another layout and re-sets it on the engine.
pub async fn put_active_layout(
    State(app): State<Arc<App>>,
    Json(request): Json<SetActiveLayout>,
) -> ApiResult<Json<LayoutsResponse>> {
    let layout = {
        let mut store = app
            .store
            .write()
            .await;
        let layout = store
            .layouts
            .layouts
            .iter()
            .find(|l| l.id == request.id)
            .cloned()
            .ok_or_else(|| ApiFailure::not_found(format!("no layout with the id {}", request.id)))?;
        store.layouts.active = request.id.clone();
        store
            .save_layouts()
            .map_err(|e| ApiFailure::internal(format!("could not save the layouts: {e}")))?;
        layout
    };

    let devices = app.devices.read().await.clone();
    app.engine.set_layout(layout.clone(), devices);
    if let Some(effect) = &layout.effect {
        crate::logging::warn_if_err(
            &format!("the layout became active but its effect {} did not start", effect.id),
            crate::apply_effect(&app, &effect.id, &effect.props).await,
        );
    }
    tracing::info!(layout = %layout.id, "the active layout changed");

    Ok(list_layouts(State(app)).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::{DeviceId, DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::layout::{LedSlice, RenderScale, SamplingMode, ZoneShape};

    fn zone(index: u32, kind: ZoneKind, led_count: u32) -> ZoneInfo {
        ZoneInfo {
            index,
            name: format!("zone{index}"),
            kind,
            led_count,
            min_leds: led_count,
            max_leds: led_count,
            resizable: true,
        }
    }

    fn device(id: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.to_string()),
            name: id.to_string(),
            vendor: "test".into(),
            kind: DeviceKind::Other,
            adapter: "fake".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones,
            connected: true,
        }
    }

    /// A placement of the zone, with the run of LEDs it owns.
    fn placed(zone_index: u32, leds: Option<(u32, u32)>) -> PlacedZone {
        PlacedZone {
            device_id: DeviceId("d0".into()),
            zone_index,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 20.0,
            rotation_deg: 0.0,
            reversed: false,
            visible: true,
            shape: ZoneShape::Auto,
            label: None,
            leds: leds.map(|(start, count)| LedSlice { start, count }),
            part: None,
            locked: false,
        }
    }

    fn layout(zones: Vec<PlacedZone>) -> Layout {
        Layout {
            id: "l".into(),
            name: "l".into(),
            render_scale: RenderScale::P100,
            sampling: SamplingMode::AreaAverage,
            zones,
            effect: None,
            case_form: canvas_proto::layout::CaseForm::None,
            landmarks: Vec::new(),
        }
    }

    #[test]
    fn a_layout_without_slices_is_accepted() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 40)])];
        let lay = layout(vec![placed(0, None)]);
        assert_eq!(validate_layout(&lay, &devices), Ok(()));
    }

    #[test]
    fn two_placements_that_claim_the_same_leds_are_refused() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 40)])];
        let lay = layout(vec![placed(0, Some((0, 12))), placed(0, Some((6, 12)))]);
        let error = validate_layout(&lay, &devices).expect_err("the slices overlap");
        assert!(error.contains("the same LEDs"), "{error}");
    }

    #[test]
    fn a_slice_past_the_end_of_the_zone_is_refused() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 20)])];
        let lay = layout(vec![placed(0, Some((16, 12)))]);
        let error = validate_layout(&lay, &devices).expect_err("the slice runs past the zone");
        assert!(error.contains("runs past the end of the zone"), "{error}");
        assert!(error.contains("20 LEDs"), "{error}");
    }

    #[test]
    fn a_whole_zone_placement_and_a_slice_of_the_same_zone_overlap() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 40)])];
        let lay = layout(vec![placed(0, None), placed(0, Some((12, 12)))]);
        let error = validate_layout(&lay, &devices).expect_err("they overlap");
        assert!(error.contains("overlap"), "{error}");
    }

    #[test]
    fn a_matrix_zone_with_a_slice_is_not_an_error() {
        let map = vec![Some(0), Some(1), Some(2), Some(3)];
        let devices =
            vec![device("d0", vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map }, 4)])];
        let lay = layout(vec![placed(0, Some((0, 2)))]);
        assert_eq!(validate_layout(&lay, &devices), Ok(()));
    }

    #[test]
    fn a_zone_that_shrank_under_its_slice_is_reported_as_needing_mapping() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 20)])];
        let lay = layout(vec![placed(0, Some((16, 12)))]);
        let messages = slices_needing_mapping(&lay, &devices);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].contains("needs"), "{:?}", messages[0]);
    }

    #[test]
    fn a_layout_that_fits_needs_no_mapping() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 40)])];
        let lay = layout(vec![placed(0, Some((0, 12))), placed(0, Some((12, 12)))]);
        assert!(slices_needing_mapping(&lay, &devices).is_empty());
    }
}
