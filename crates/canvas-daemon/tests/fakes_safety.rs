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

/// A sink that drops every picture: the recorder only needs to know one was given.
struct NullSink;

impl canvas_proto::traits::PreviewSink for NullSink {
    fn submit(&self, _width: u32, _height: u32, _rgb: Vec<u8>) {}
}

/// `FakeEngine` records each `set_group_preview` call (the group asked for, and whether a sink
/// was given), so the #378 route builder can assert the selection and the handoff without
/// editing the shared harness.
#[test]
fn the_fake_engine_records_group_preview_calls() {
    use canvas_proto::traits::{Engine, PreviewSink};

    let engine = FakeEngine::default();
    assert!(engine.group_preview_calls().is_empty());
    let sink: Arc<dyn PreviewSink> = Arc::new(NullSink);
    engine.set_group_preview(Some("desk".into()), Some(sink));
    engine.set_group_preview(None, None);
    assert_eq!(
        engine.group_preview_calls(),
        vec![(Some("desk".to_string()), true), (None, false)]
    );
}
