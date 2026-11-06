//! The audio-source routes (#166): the picker list, the pick, and the live test.
//!
//! `PUT /audio/source` is the only writer of `ConfigFile.audio_source`; it
//! takes the store lock, saves, and reopens the capture if it runs. The test
//! lease is a set keyed by id, so two editor windows each hold their own.

use axum::extract::{Query, State};
use axum::Json;
use canvas_proto::audio::{AudioSource, AudioSources, AudioTestLease, AudioTestRequest, AudioTestState, ResolvedAudioSource};
use serde::Deserialize;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// `GET /api/v1/audio/sources`: enumerates on demand (and after a device
/// notification), never on a timer. COM work runs on a blocking thread.
pub async fn get_sources(State(app): State<Arc<App>>) -> ApiResult<Json<AudioSources>> {
    let inputs = Arc::clone(&app.inputs);
    let sources = tokio::task::spawn_blocking(move || {
        let endpoints = inputs.backend().enumerate();
        let requested = inputs.requested();
        let resolved = inputs.backend().resolve(&requested);
        (endpoints, resolved)
    })
    .await
    .map_err(|e| ApiFailure::internal(format!("the audio enumeration did not finish: {e}")))?;
    // The API reads the same resolved value the log reads: record it so the
    // two cannot disagree.
    let resolved = sources.1.clone();
    app.inputs.record_resolved_public(&resolved);
    Ok(Json(AudioSources { endpoints: sources.0, current: resolved }))
}

/// `PUT /api/v1/audio/source`: the only writer of the pick.
pub async fn put_source(
    State(app): State<Arc<App>>,
    Json(source): Json<AudioSource>,
) -> ApiResult<Json<ResolvedAudioSource>> {
    {
        let mut store = app.store.write().await;
        store.config.audio_source = source.clone();
        store
            .save_config()
            .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))?;
    }
    app.inputs.set_requested(source);
    let inputs = Arc::clone(&app.inputs);
    let resolved = tokio::task::spawn_blocking(move || {
        let requested = inputs.requested();
        inputs.backend().resolve(&requested)
    })
    .await
    .map_err(|e| ApiFailure::internal(format!("the audio resolve did not finish: {e}")))?;
    app.inputs.record_resolved_public(&resolved);
    Ok(Json(resolved))
}

/// `PUT /api/v1/audio/test`: takes a test lease.
pub async fn put_test(
    State(app): State<Arc<App>>,
    Json(request): Json<AudioTestRequest>,
) -> Json<AudioTestLease> {
    let lease_id = app.inputs.take_lease(request.listen);
    Json(AudioTestLease { lease_id })
}

/// Query of `GET /api/v1/audio/test?lease=<id>`.
#[derive(Debug, Deserialize)]
pub struct TestQuery {
    pub lease: String,
}

/// `GET /api/v1/audio/test?lease=<id>`: reads and renews the lease; an
/// unknown or expired lease answers 410.
pub async fn get_test(
    State(app): State<Arc<App>>,
    Query(query): Query<TestQuery>,
) -> ApiResult<Json<AudioTestState>> {
    if !app.inputs.renew_lease(&query.lease) {
        return Err(ApiFailure::new(
            axum::http::StatusCode::GONE,
            "gone",
            "the audio test lease is unknown or expired; take a new one",
        ));
    }
    Ok(Json(app.inputs.test_state()))
}

/// How often the daemon checks the audio leases. One second, like the draft
/// sweeper: a lease ends `AUDIO_TEST_LEASE` after its last renewal.
pub const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Starts the task that ends a test lease whose renewal stopped, so the
/// capture stops within `AUDIO_TEST_LEASE` plus this of the last poll.
pub fn spawn_sweeper(app: Arc<App>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            ticker.tick().await;
            if app.shutting_down.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                return;
            }
            sweep_once(&app).await;
        }
    });
}

/// One sweep: expired leases end and the refcount re-runs. Panic-guarded like
/// the draft sweeper, so one panic never stops the sweeper for good.
async fn sweep_once(app: &Arc<App>) {
    let sweep_app = Arc::clone(app);
    match tokio::spawn(async move { sweep_app.inputs.sweep_leases().await }).await {
        Ok(_) => {}
        Err(error) if error.is_panic() => {
            let payload = error.into_panic();
            tracing::error!(
                panic = %crate::panic_payload_text(&payload),
                "the audio lease sweeper panicked; the sweep continues"
            );
        }
        Err(error) => {
            tracing::error!(
                error = %error,
                "the audio lease sweeper's sweep did not finish; the sweep continues"
            );
        }
    }
}
