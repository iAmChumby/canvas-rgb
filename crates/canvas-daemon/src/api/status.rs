//! `GET /api/v1/status` and `POST /api/v1/quit`.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use canvas_proto::api::{routes, Status};
use canvas_proto::config::OnQuit;
use std::sync::Arc;

use crate::state::App;

/// `GET /api/v1/status`. The only route that needs no token.
pub async fn status(State(app): State<Arc<App>>) -> axum::Json<Status> {
    axum::Json(app.status().await)
}

/// `POST /api/v1/quit`.
///
/// Applies `settings.on_quit` (black by default, G3), tells the service the daemon is
/// quitting on purpose, and asks the daemon to exit.
pub async fn quit(State(app): State<Arc<App>>) -> Response {
    let on_quit = app.store.read().await.config.settings.on_quit;

    match on_quit {
        OnQuit::Black => {
            app.engine.write_solid(canvas_proto::Rgb8::BLACK);
            tracing::info!("quit: wrote black to every device");
        }
        OnQuit::LeaveLastFrame => {
            tracing::info!("quit: leaving the last frame on the devices");
        }
    }

    // Tell the service not to restart us (G3).
    crate::pipe_client::notify_quitting(&app).await;

    app.shutting_down.store(1, std::sync::atomic::Ordering::SeqCst);
    let _ = routes::QUIT;

    // Ask the main task to stop; it does the actual exit.
    let _ = app.quit_tx.send(());

    axum::Json(serde_json::json!({ "quitting": true })).into_response()
}
