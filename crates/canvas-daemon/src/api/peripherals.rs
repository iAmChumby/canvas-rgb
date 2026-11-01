//! The peripheral routes: `GET /peripherals`, `GET /peripherals/{id}`, `PATCH /peripherals/{id}`
//! `PUT /peripherals/{id}/access` and `GET /peripherals/{id}/read` (the stage of a read that is open).
//!
//! The rule that shapes every route: the daemon never opens a peripheral it was not told to.
//! The enumeration opens nothing, a read happens only while reads are on for that device id,
//! and a write only while writes are on. What the daemon may do with a device is saved in
//! `peripherals.json` (see `crate::peripherals`).

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::device::DeviceId;
use canvas_proto::peripheral::{
    PeripheralAccess, PeripheralDetail, PeripheralInfo, PeripheralsResponse, ReadProgress,
};
use canvas_proto::props::Props;
use canvas_proto::traits::AdapterError;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::peripherals::READS_OFF;
use crate::state::App;

/// `GET /api/v1/peripherals`. Every peripheral the adapters see.
///
/// The enumeration opens nothing: a listed peripheral is not one the daemon may touch.
pub async fn list_peripherals(State(app): State<Arc<App>>) -> Json<PeripheralsResponse> {
    let peripherals = app.peripherals.discover().await;
    Json(PeripheralsResponse { peripherals })
}

/// `GET /api/v1/peripherals/{id}`: what the device is, what it has and what it holds now.
///
/// The schema is always there. The values are read from the device only while reads are on for
/// this id; otherwise they are empty and `problem` says why. A read that fails leaves the
/// values empty too, and `problem` carries the adapter's own message.
pub async fn get_peripheral(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<PeripheralDetail>> {
    let device = DeviceId(id.clone());
    let access = app.store.read().await.peripherals.access(&device);
    let info = find_info(&app, &device)
        .await
        .ok_or_else(|| ApiFailure::not_found(format!("no peripheral with the id {id}")))?;
    let schema = app.peripherals.describe(&device).await.map_err(adapter_failure)?;
    let (values, problem) = if access.reads {
        match app.peripherals.read(&device).await {
            Ok(values) => (values, None),
            Err(error) => {
                tracing::warn!(device = %id, error = %error, "the peripheral did not answer the read");
                (Props::new(), Some(error.to_string()))
            }
        }
    } else {
        (Props::new(), Some(READS_OFF.to_string()))
    };
    Ok(Json(PeripheralDetail { info, schema, values, access, problem }))
}

/// `PATCH /api/v1/peripherals/{id}`: writes the given keys to the device.
///
/// Refused with 422 `validation` while config writes are off for this device, and nothing is
/// sent. The adapter's own validation refuses an unknown key, a read-only key and a value out
/// of range, and nothing is sent then either.
pub async fn patch_peripheral(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(props): Json<Props>,
) -> ApiResult<Json<Props>> {
    let device = DeviceId(id.clone());
    let access = app.store.read().await.peripherals.access(&device);
    if !access.writes {
        return Err(ApiFailure::validation(format!(
            "config writes are off for {id}: turn them on before writing"
        )));
    }
    if !access.reads {
        // A saved access that says otherwise is broken: a write needs the device open.
        return Err(ApiFailure::validation(format!(
            "config writes need reads for {id}: turn reads on before writing"
        )));
    }
    if find_info(&app, &device).await.is_none() {
        return Err(ApiFailure::not_found(format!("no peripheral with the id {id}")));
    }
    app.peripherals.write(&device, &props).await.map_err(adapter_failure)?;
    tracing::info!(
        device = %id,
        keys = ?props.keys().collect::<Vec<_>>(),
        "the peripheral settings were written"
    );
    Ok(Json(props))
}

/// `PUT /api/v1/peripherals/{id}/access`: what the daemon may do with this device.
///
/// The access is saved per device id in `peripherals.json`. `writes` without `reads` is a
/// validation error: a write patches what the device last reported, so the device has to be
/// read first. Turning reads off closes the device, so the vendor app can take it back.
pub async fn put_peripheral_access(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(access): Json<PeripheralAccess>,
) -> ApiResult<Json<PeripheralAccess>> {
    if access.writes && !access.reads {
        return Err(ApiFailure::validation(
            "config writes need reads: turn reads on too",
        ));
    }
    let device = DeviceId(id.clone());
    // The access is saved first: it is what the daemon may do from now on, whether or not the
    // device answers right now.
    {
        let mut store = app.store.write().await;
        let mut next = store.peripherals.clone();
        next.set(&device, access);
        crate::peripherals::save(&store.home, &next)
            .map_err(|e| ApiFailure::internal(format!("could not save the peripheral access: {e}")))?;
        store.peripherals = next;
    }
    // Then the hardware follows it.
    match app.peripherals.apply(&device, access).await {
        Ok(()) => {}
        Err(AdapterError::UnknownDevice(_)) => {
            // The device is not here. The access is saved and applies when it comes back.
            tracing::info!(device = %id, "the peripheral is not here; the access is saved for later");
        }
        Err(error) => {
            tracing::warn!(device = %id, error = %error, "the peripheral could not be opened");
            return Err(ApiFailure::internal(format!(
                "the access is saved, but the peripheral could not be opened: {error}"
            )));
        }
    }
    tracing::info!(device = %id, reads = access.reads, writes = access.writes, "the peripheral access was saved");
    Ok(Json(access))
}

/// `GET /api/v1/peripherals/{id}/read`: the stage of the read of this device that is open now.
///
/// It answers at once and never waits on the device or on the adapters' lock, so it can be asked
/// while `GET /peripherals/{id}` is still waiting for a sleeping mouse. An id that is not reading
/// (or not a device) is `idle`.
pub async fn get_peripheral_read(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Json<ReadProgress> {
    Json(app.peripherals.read_progress(&DeviceId(id)))
}

/// The peripheral with this id, as the last enumeration found it.
///
/// The enumeration is refreshed when the id is not in it, so a device that was plugged in
/// after the last one is still found.
async fn find_info(app: &App, device: &DeviceId) -> Option<PeripheralInfo> {
    let found = app.peripherals.discover().await;
    if let Some(info) = found.iter().find(|i| &i.id == device) {
        return Some(info.clone());
    }
    app.peripherals
        .discover_now()
        .await
        .into_iter()
        .find(|i| &i.id == device)
}

/// The API error for an adapter error.
///
/// A refusal (a key the device does not have, a read-only key, a value out of range) is the
/// caller's mistake, so it is a 422. A device that is not there is a 404. Anything else is the
/// device's fault, so it is a 500.
fn adapter_failure(error: AdapterError) -> ApiFailure {
    match error {
        AdapterError::UnknownDevice(id) => {
            ApiFailure::not_found(format!("no peripheral with the id {}", id.0))
        }
        AdapterError::Protocol(message) => ApiFailure::validation(message),
        AdapterError::Unsupported(message) => ApiFailure::validation(message.to_string()),
        AdapterError::NotConnected => ApiFailure::validation(
            "the peripheral is not open: turn reads on before reading or writing it",
        ),
        AdapterError::Zone { device, zone, reason } => ApiFailure::internal(format!(
            "zone {zone} of {}: {reason}",
            device.0
        )),
        AdapterError::Io(error) => {
            ApiFailure::internal(format!("the peripheral did not answer: {error}"))
        }
    }
}
