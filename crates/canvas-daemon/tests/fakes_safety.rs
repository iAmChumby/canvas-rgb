//! Fakes mode must never reach the real service (#199): every request to the service goes
//! through `App::pipe`, which a fakes-mode daemon replaces with an in-process fake.
//!
//! These tests use the in-test fakes: no pipe, device or port is opened.

mod common;

use std::sync::{Arc, Mutex};

use canvas_daemon::pipe_client::{PipeError, PipeLink};
use canvas_daemon::state::App;
use canvas_proto::pipe::{PipeRequest, PipeResponse};
use common::{FakeAdapter, FakeEngine};

/// A pipe that records every request and answers `Ok`.
#[derive(Default)]
struct RecordingPipe {
    seen: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PipeLink for RecordingPipe {
    async fn exchange(&self, request: &PipeRequest) -> Result<PipeResponse, PipeError> {
        self.seen.lock().unwrap().push(format!("{request:?}"));
        Ok(PipeResponse::Ok)
    }
}

fn app_with(pipe: Arc<RecordingPipe>) -> Arc<App> {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        canvas_daemon::storage::Store::default(),
        Arc::new(FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        "token".into(),
        "0.1.0-test".into(),
        tx,
    );
    app.pipe = pipe;
    Arc::new(app)
}

/// The quit notice (the `POST /api/v1/quit` route and the quit sequence) goes through the app's
/// pipe. Before #199's review fix it opened the real `\\.\pipe\canvas-rgb-svc` directly, so a
/// fakes-mode daemon told the real service it was quitting.
#[tokio::test]
async fn the_quit_notice_goes_through_the_apps_pipe() {
    let pipe = Arc::new(RecordingPipe::default());
    let app = app_with(pipe.clone());
    canvas_daemon::pipe_client::notify_quitting(&app).await;
    let seen = pipe.seen.lock().unwrap().clone();
    assert_eq!(seen, vec![format!("{:?}", PipeRequest::DaemonQuitting)]);
}
