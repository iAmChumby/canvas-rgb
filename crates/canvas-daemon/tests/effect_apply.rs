//! A panicking `schema` in `apply_built` is an apply error, never a crash (#99).

mod common;

use std::sync::Arc;

use canvas_daemon::state::App;
use canvas_proto::props::{InputNeeds, PropertySchema, Props};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use common::{FakeAdapter, FakeEngine};

/// An effect whose `schema` panics every time.
struct PanicSchemaDaemonEffect;

impl Effect for PanicSchemaDaemonEffect {
    fn schema(&self) -> PropertySchema {
        panic!("schema boom");
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
}

/// An effect whose schema and update are fine.
struct GoodDaemonEffect;

impl Effect for GoodDaemonEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_good_daemon".into(),
            name: "Test Good Daemon".into(),
            description: "Fine.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
}

fn app_with_fake_engine(tag: &str) -> (Arc<App>, Arc<FakeEngine>) {
    let home = std::env::temp_dir().join(format!(
        "canvas-rgb-effect-apply-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp home is created");
    let store = canvas_daemon::storage::Store::at(&home);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    (app, engine)
}

#[tokio::test]
async fn apply_built_with_a_panicking_schema_is_an_error_not_a_crash() {
    let (app, engine) = app_with_fake_engine("schema");
    let result = canvas_daemon::apply_built(&app, Box::new(PanicSchemaDaemonEffect), &Props::new()).await;
    let error = result.expect_err("a panicking schema becomes an apply error");
    assert!(error.contains("schema"), "the apply error names the schema: {error}");
    assert!(engine.effect_id.lock().unwrap().is_none(), "the panicking effect never reached the engine");

    let ok = canvas_daemon::apply_built(&app, Box::new(GoodDaemonEffect), &Props::new()).await;
    assert!(ok.is_ok(), "the daemon keeps applying after the schema panic: {ok:?}");
    assert_eq!(
        engine.effect_id.lock().unwrap().as_deref(),
        Some("test_good_daemon"),
        "the engine runs the next effect"
    );
}
