//! The peripheral routes: `GET /peripherals`, `GET /peripherals/{id}`, `PATCH /peripherals/{id}`
//! `PUT /peripherals/{id}/access` and `GET /peripherals/{id}/read` (the stage of a read that is open).
//!
//! The rule that shapes every route: the daemon never opens a peripheral it was not told to.
//! The enumeration opens nothing, a read happens only while reads are on for that device id,
//! and a write only while writes are on. What the daemon may do with a device is saved in
//! `peripherals.json` (see `crate::peripherals`).

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use canvas_proto::device::DeviceId;
use canvas_proto::peripheral::{
    PeripheralAccess, PeripheralDetail, PeripheralInfo, PeripheralsResponse, ReadProgress,
};
use canvas_proto::props::Props;
use canvas_proto::traits::AdapterError;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::peripherals::{user_words, NOT_READ_YET, READS_OFF};
use crate::state::App;

/// A request named a device the adapters do not see. The id belongs in the log, not in an answer
/// a person reads (#572).
const NOT_HERE: &str = "The peripheral is not connected. Connect it again, then pick it from the list.";

/// The name a person knows a device by, for a refusal that names it (#572).
fn name_of(info: Option<&PeripheralInfo>) -> String {
    info.map(|info| info.name.clone()).unwrap_or_else(|| "this device".to_string())
}

/// `GET /api/v1/peripherals`. Every peripheral the adapters see.
///
/// The enumeration opens nothing: a listed peripheral is not one the daemon may touch. An open
/// pad's battery comes from its own input reports, read passively (#566).
pub async fn list_peripherals(State(app): State<Arc<App>>) -> Json<PeripheralsResponse> {
    let found = app.peripherals.discover().await;
    let peripherals = app.peripherals.with_batteries(found).await;
    Json(PeripheralsResponse { peripherals })
}

/// Reads `?fresh=` from the raw query: `true` or `1` reads the device even when the daemon keeps
/// its values; `false`, `0` or no `fresh` is the plain read. Any other value, or `fresh` given
/// twice, is a validation error in the API's JSON shape (a typed extractor would answer with
/// axum's plain-text 400).
fn parse_fresh(query: Option<&str>) -> Result<bool, ApiFailure> {
    crate::api::parse_flag(query, "fresh")
}

/// `GET /api/v1/peripherals/{id}`: what the device is, what it has and what it holds now.
///
/// A plain GET never reads the device (#127): the values are the ones the daemon kept from the
/// last read, with the writes since merged in, loaded from `peripheral-values.json` at start so
/// also the first open after a restart reads nothing. With nothing kept the values are empty
/// and `problem` says they have not been read yet; the idle background refresh reads them when
/// the person is away. `?fresh=true` is the person's **Read again** and does read now.
///
/// The schema is always there. The values are served only while reads are on for this id;
/// otherwise they are empty and `problem` says why. A fresh read that fails leaves the values
/// empty too, and `problem` says in plain words what happened ([`user_words`]).
///
/// A connection that carries no settings (a pad over Bluetooth) answers a schema with no
/// sections, no values and the reason in `problem`, and the device is not read.
pub async fn get_peripheral(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> ApiResult<Json<PeripheralDetail>> {
    let fresh = parse_fresh(query.as_deref())?;
    let device = DeviceId(id.clone());
    let access = app.store.read().await.peripherals.access(&device);
    let info = find_info(&app, &device)
        .await
        .ok_or_else(|| ApiFailure::not_found(NOT_HERE.to_string()))?;
    let (schema, unavailable) =
        app.peripherals.describe_reach(&device).await.map_err(adapter_failure)?;
    let (values, problem, read_at_ms) = if let Some(why) = unavailable {
        // Nothing to read over this connection, whatever the access is: say why, on a plain GET
        // too, and promise no later read.
        (Props::new(), Some(why.to_string()), None)
    } else if access.reads {
        if fresh {
            match app.peripherals.read_fresh_stamped(&device).await {
                Ok((values, read_at_ms)) => (values, None, Some(read_at_ms)),
                Err(error) => {
                    tracing::warn!(device = %id, error = %error, "the peripheral did not answer the read");
                    (Props::new(), Some(user_words(&error)), None)
                }
            }
        } else {
            match app.peripherals.kept_stamped(&device) {
                Some((values, read_at_ms)) => (values, None, Some(read_at_ms)),
                None => (Props::new(), Some(NOT_READ_YET.to_string()), None),
            }
        }
    } else {
        (Props::new(), Some(READS_OFF.to_string()), None)
    };
    Ok(Json(PeripheralDetail { info, schema, values, access, problem, read_at_ms }))
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
    // The name comes from the enumeration, so a refusal names the device a person can see rather
    // than the id in the URL (#572).
    let info = find_info(&app, &device).await;
    let name = name_of(info.as_ref());
    if !access.writes {
        return Err(ApiFailure::validation(format!(
            "Changes are off for {name}. Turn on Change first."
        )));
    }
    if !access.reads {
        // A saved access that says otherwise is broken: a write needs the device open.
        return Err(ApiFailure::validation(format!(
            "Changes need reads for {name}. Turn on Read first."
        )));
    }
    if info.is_none() {
        return Err(ApiFailure::not_found(NOT_HERE.to_string()));
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
            "Changes need reads as well. Turn on Read first.",
        ));
    }
    let device = DeviceId(id.clone());
    // The access is saved first: it is what the daemon may do from now on, whether or not the
    // device answers right now.
    // Published to memory only after the file is written: an access that failed to save never
    // takes effect, which is the fail-closed direction.
    let key = device.clone();
    crate::store_io::store_publish(
        &app,
        |store| {
            store.peripherals.set(&key, access);
            Ok(())
        },
        |store, ()| {
            crate::peripherals::save(&store.home, &store.peripherals).map_err(|e| {
                tracing::warn!(error = %e, "the peripheral access could not be saved");
                ApiFailure::internal("The change could not be saved. Check the log for details.")
            })
        },
    )
    .await?;
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
                "The access is saved, but the device could not be opened. {}",
                user_words(&error)
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
    // One function turns an adapter error into the words a person reads, on the write path as on
    // the read path. The id, the zone and the link error belong in the log, not in the answer
    // (#572).
    let message = user_words(&error);
    match error {
        AdapterError::UnknownDevice(id) => {
            tracing::warn!(device = %id.0, "the peripheral is not here");
            ApiFailure::not_found(message)
        }
        AdapterError::Protocol(_) | AdapterError::Unsupported(_) | AdapterError::NotConnected => {
            ApiFailure::validation(message)
        }
        AdapterError::Zone { device, zone, reason } => {
            tracing::warn!(device = %device.0, zone, reason, "the peripheral zone failed");
            ApiFailure::internal(message)
        }
        AdapterError::Io(cause) => {
            tracing::warn!(error = %cause, "the peripheral link failed");
            ApiFailure::internal(message)
        }
    }
}
