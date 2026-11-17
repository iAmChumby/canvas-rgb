//! The lights API (Scenes plan, Task 6 part A): `GET /api/v1/lights` and
//! `PATCH /api/v1/lights/{id}`.
//!
//! The rig is the one `tests/api.rs` uses: the daemon's state built from the in-test fakes,
//! a temporary home, and a router driven with `oneshot`. These tests never touch hardware.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_daemon::storage::Store;
use canvas_proto::api::{routes, ApiError, LightDeleted, LightsResponse, LightView};
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::LedSlice;
use canvas_proto::scene::{
    Arrangement, Light, LightsFile, GROUP_NAME_MAX, LIGHTS_SCHEMA,
};
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A temporary directory that is removed when it is dropped.
struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-lights-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp directory is created");
        Self { path }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Builds the router, the app and the temp home, with `lights` in the store and `devices` as the
/// discovered list.
async fn build(
    lights: LightsFile,
    devices: Vec<DeviceInfo>,
) -> (axum::Router, Arc<App>, TempHome) {
    let home = TempHome::new();
    let mut store = Store::at(&home.path);
    store.lights = lights;
    build_with(store, devices, home).await
}

async fn build_with(
    store: Store,
    devices: Vec<DeviceInfo>,
    home: TempHome,
) -> (axum::Router, Arc<App>, TempHome) {
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    *app.devices.write().await = devices;
    (api::router(app.clone()), app, home)
}

/// A request with the token.
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// A request with a JSON body and the token.
fn request_with_body(method: &str, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds")
}

/// The body of a response, as JSON.
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the body is bounded");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// The path of one light. A light id holds `#`, which a URI would read as a fragment: the id is
/// sent percent-encoded, as the editor's own client sends it.
fn light_path(id: &str) -> String {
    routes::LIGHT.replace("{id}", &id.replace('#', "%23"))
}

fn zone(index: u32, name: &str, leds: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: name.to_string(),
        kind: ZoneKind::Linear,
        led_count: leds,
        min_leds: 1,
        max_leds: 512,
        resizable: true,
    }
}

fn device(id: &str, name: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.to_string()),
        name: name.to_string(),
        vendor: "fake".to_string(),
        kind: DeviceKind::Motherboard,
        adapter: "fake".to_string(),
        adapter_index: 0,
        serial: id.to_string(),
        location: String::new(),
        zones,
        connected: true,
    }
}

fn light(
    id: &str,
    device_id: &str,
    zone_index: u32,
    leds: Option<(u32, u32)>,
    name: Option<&str>,
) -> Light {
    Light {
        id: id.to_string(),
        device_id: DeviceId(device_id.to_string()),
        zone_index,
        leds: leds.map(|(start, count)| LedSlice { start, count }),
        name: name.map(str::to_string),
        arrangement: Arrangement::Strip,
        reversed: false,
        kind: None,
        copies: None,
        first_led: None,
    }
}

fn lights_file(lights: Vec<Light>) -> LightsFile {
    LightsFile { chains: Vec::new(), schema_version: LIGHTS_SCHEMA, lights }
}

async fn get_lights(router: axum::Router) -> (StatusCode, serde_json::Value) {
    let response = router
        .oneshot(request("GET", routes::LIGHTS))
        .await
        .expect("the router answers");
    let status = response.status();
    (status, json(response).await)
}

fn views(body: &serde_json::Value) -> LightsResponse {
    serde_json::from_value(body.clone()).expect("the body is a LightsResponse")
}

/// OpenRGB's copy of the MonsGeek keyboard: `is_driven` drops it from discovery (#393).
const UNDRIVEN: &str = "openrgb:HID: hid#VID_3151&PID_4015&MI_02#7&abc";

/// The scene a leftover light landed in, on disk: one group holding only that light.
fn scenes_with(group_id: &str, name: &str, members: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "active": "gaming",
        "scenes": [{
            "id": "gaming", "name": "Gaming", "rev": 1,
            "groups": [{
                "id": group_id, "name": name, "members": members,
                "plays": { "type": "off" }, "mode": "flow_across", "brightness": 100
            }]
        }]
    })
}

/// A `DELETE` on one light, with the token. The id holds a space and `#`, both encoded as the
/// editor's client sends them.
fn delete_request(id: &str) -> Request<Body> {
    request(
        "DELETE",
        &routes::LIGHT.replace("{id}", &id.replace('#', "%23").replace(' ', "%20")),
    )
}

/// A `PUT` of a whole scene, with the token and a JSON body.
fn put_scene_request(id: &str, put: &canvas_proto::api::ScenePut) -> Request<Body> {
    request_with_body(
        "PUT",
        &routes::SCENE.replace("{id}", id),
        &serde_json::to_string(put).expect("the scene put serialises"),
    )
}

/// A home holding two leftover lights in one group, loaded the way the daemon starts. Deleting
/// `id` leaves `stays` in the group, so the scene that comes back is still valid to PUT (#497).
async fn build_delete_pair() -> (axum::Router, Arc<App>, TempHome, String) {
    let id = format!("{UNDRIVEN}#z0p1");
    let stays = format!("{UNDRIVEN}#z0p2");
    let home = TempHome::new();
    let on_disk = lights_file(vec![
        light(&id, UNDRIVEN, 0, Some((0, 8)), Some("Old strip")),
        light(&stays, UNDRIVEN, 0, Some((8, 8)), Some("Other strip")),
    ]);
    std::fs::write(
        home.path.join(canvas_proto::scene::LIGHTS_FILE),
        serde_json::to_vec(&on_disk).expect("the lights file serialises"),
    )
    .expect("the lights file is written");
    std::fs::write(
        home.path.join(canvas_proto::scene::SCENES_FILE),
        serde_json::to_vec(&scenes_with("g-shared", "Two strips", &[&id, &stays]))
            .expect("the scenes file serialises"),
    )
    .expect("the scenes file is written");
    let store = Store::load_from(&home.path);
    let (router, app, home) = build_with(store, Vec::new(), home).await;
    (router, app, home, id)
}

/// #414 regression: an old `lights.json` can hold a *named slice* light (id ending `p<n>`, with
/// `leds` and a `name`) on a device the daemon never drives. The #393 start-up cleanup only removes
/// whole-zone lights the owner never touched, so this light survives. The owner sees it in Scenes'
/// add-devices list as "0 LEDs · not connected" and can put it in a group. `DELETE` now removes it,
/// names the group it emptied, and drops that group. On the old code the route answered 405 (no way
/// to remove the light), so this test fails there.
#[tokio::test]
async fn deleting_an_old_named_slice_light_removes_it_and_the_group_it_emptied() {
    let id = format!("{UNDRIVEN}#z0p1");
    let home = TempHome::new();
    let on_disk = lights_file(vec![light(&id, UNDRIVEN, 0, Some((0, 8)), Some("Old keyboard strip"))]);
    std::fs::write(
        home.path.join(canvas_proto::scene::LIGHTS_FILE),
        serde_json::to_vec(&on_disk).expect("the lights file serialises"),
    )
    .expect("the lights file is written");
    std::fs::write(
        home.path.join(canvas_proto::scene::SCENES_FILE),
        serde_json::to_vec(&scenes_with("g-old", "Old keyboard", &[&id]))
            .expect("the scenes file serialises"),
    )
    .expect("the scenes file is written");
    // The daemon starts from those files: the #393 cleanup runs, but a named slice is the owner's.
    let store = Store::load_from(&home.path);
    assert!(
        store.lights.lights.iter().any(|l| l.id == id),
        "the named slice survives the start-up cleanup: it carries a name and leds"
    );
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let (router, app, home) = build_with(store, devices, home).await;

    // What the owner sees from GET /lights before the delete: no LEDs, not connected.
    let (status, body) = get_lights(router.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let view = views(&body).lights.into_iter().find(|v| v.light.id == id).expect("the light is listed");
    assert!(!view.connected, "the device the light names is not driven");
    assert_eq!(view.led_count, 0, "no zone stands behind the slice");

    let response = router.clone().oneshot(delete_request(&id)).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let deleted: LightDeleted = serde_json::from_value(body.clone()).expect("the body is a LightDeleted");
    assert!(deleted.ok, "{body}");
    assert_eq!(deleted.id, id);
    assert_eq!(deleted.emptied_groups.len(), 1, "{body}");
    let group = &deleted.emptied_groups[0];
    assert_eq!(group.scene_id, "gaming");
    assert_eq!(group.group_id, "g-old");
    assert_eq!(group.name, "Old keyboard", "the group that went is named for the note");

    // The light is gone from memory, from GET and from the saved file; the empty group went too.
    assert!(app.store.read().await.lights.lights.iter().all(|l| l.id != id));
    let (_, body) = get_lights(router.clone()).await;
    assert!(views(&body).lights.iter().all(|v| v.light.id != id));
    let saved: LightsFile = serde_json::from_str(
        &std::fs::read_to_string(home.path.join(canvas_proto::scene::LIGHTS_FILE)).unwrap(),
    )
    .expect("lights.json is a lights file");
    assert!(saved.lights.iter().all(|l| l.id != id));
    assert!(
        app.store.read().await.scenes.scenes[0].groups.is_empty(),
        "the group the light was the last member of was dropped"
    );
}

/// A light that shares its group with another: the group keeps the other member and stays.
#[tokio::test]
async fn deleting_one_light_keeps_another_member_of_its_group() {
    let id = format!("{UNDRIVEN}#z0p1");
    let other_id = format!("{UNDRIVEN}#z0p2");
    let home = TempHome::new();
    let on_disk = lights_file(vec![
        light(&id, UNDRIVEN, 0, Some((0, 8)), Some("Old strip")),
        light(&other_id, UNDRIVEN, 0, Some((8, 8)), Some("Other strip")),
    ]);
    std::fs::write(
        home.path.join(canvas_proto::scene::LIGHTS_FILE),
        serde_json::to_vec(&on_disk).unwrap(),
    )
    .unwrap();
    std::fs::write(
        home.path.join(canvas_proto::scene::SCENES_FILE),
        serde_json::to_vec(&scenes_with(
            "g-shared",
            "Two strips",
            &[&id, &other_id],
        ))
        .unwrap(),
    )
    .unwrap();
    let store = Store::load_from(&home.path);
    let (router, app, _home) = build_with(store, Vec::new(), home).await;

    let response = router.oneshot(delete_request(&id)).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let deleted: LightDeleted = serde_json::from_value(body.clone()).expect("the body is a LightDeleted");
    assert!(deleted.emptied_groups.is_empty(), "{body}");
    let store = app.store.read().await;
    let group = &store.scenes.scenes[0].groups[0];
    assert_eq!(group.id, "g-shared", "the group stayed");
    assert_eq!(group.members, vec![other_id.clone()], "its other member stayed");
    assert!(store.lights.lights.iter().any(|l| l.id == other_id));
}

/// #497: the delete is a server-side scene write, so it moves the scene's rev like every other
/// one. A PUT that still holds the rev from before the delete is refused with 409, even though its
/// scene is valid by the time it arrives. On code that left `rev` alone the PUT succeeded (200).
#[tokio::test]
async fn a_scene_put_naming_the_rev_from_before_the_delete_is_a_409() {
    let (router, app, _home, id) = build_delete_pair().await;
    assert_eq!(app.store.read().await.scenes.scenes[0].rev, 1, "the rev before the delete");

    let response = router.clone().oneshot(delete_request(&id)).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(app.store.read().await.scenes.scenes[0].rev, 2, "the delete moved the rev it changed");

    // The scene as it is now is valid (the group kept its other member). Only the rev is stale.
    let after = app.store.read().await.scenes.scenes[0].clone();
    let put = canvas_proto::api::ScenePut { scene: after, expected_rev: 1, dry_run: false };
    let response = router.oneshot(put_scene_request("gaming", &put)).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::CONFLICT, "a PUT with the old rev is stale");
    let error: ApiError =
        serde_json::from_value(json(response).await).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::CONFLICT);
}

/// #497: park a scene PUT between its validation and its write (through a test seam). The PUT
/// validated while the light was still there and holds the scene order; the delete must not slip
/// in and let the PUT write that light back after it is gone. On the code without the scene gate
/// the delete lands first and the PUT resurrects the light in `scenes.json` — this test fails
/// there. On the fixed code the delete waits for the scene order and follows the PUT, so the
/// light is gone and the rev moved twice.
#[tokio::test]
async fn a_scene_put_parked_before_its_write_cannot_resurrect_a_deleted_light() {
    let (router, app, home, id) = build_delete_pair().await;
    let before = app.store.read().await.scenes.scenes[0].clone();
    assert_eq!(before.rev, 1);

    let seam: Arc<canvas_daemon::api::scenes::ScenePutSeam> = Arc::new(Default::default());
    app.set_scene_put_seam(seam.clone());

    // The PUT validates against the light (still there) and parks before its write, holding the
    // scene order. It is spawned so the delete can be started while it waits at the seam.
    let put = {
        let router = router.clone();
        let put = canvas_proto::api::ScenePut {
            scene: before,
            expected_rev: 1,
            dry_run: false,
        };
        tokio::spawn(async move {
            router.oneshot(put_scene_request("gaming", &put)).await.expect("the router answers")
        })
    };
    seam.wait_until_reached().await;

    // Start the delete while the PUT is parked. On the fixed code it waits for the scene order the
    // PUT holds, so the light stays in the store; on code without the gate it lands now. Give each
    // spawned task a bounded number of yields to reach its blocking point — no clock is read.
    let delete = {
        let router = router.clone();
        let id = id.clone();
        tokio::spawn(async move {
            router.oneshot(delete_request(&id)).await.expect("the router answers")
        })
    };
    for _ in 0..256 {
        tokio::task::yield_now().await;
        let still = app.store.read().await.scenes.scenes.iter().any(|sc| {
            sc.groups.iter().any(|g| g.members.contains(&id))
        });
        if !still {
            break; // the delete landed first (the code under test)
        }
    }

    seam.go();
    let put_response = put.await.expect("the PUT task finished");
    let delete_response = delete.await.expect("the delete task finished");
    assert_eq!(put_response.status(), StatusCode::OK, "the PUT validated while the light was there");
    assert_eq!(delete_response.status(), StatusCode::OK);

    // The PUT landed, then the delete followed it: the light is gone from both memory and the
    // saved file, and the rev moved for the PUT and again for the delete.
    let saved: canvas_proto::scene::ScenesFile = serde_json::from_str(
        &std::fs::read_to_string(home.path.join(canvas_proto::scene::SCENES_FILE)).unwrap(),
    )
    .expect("scenes.json is a scenes file");
    assert!(
        saved.scenes.iter().all(|sc| sc.groups.iter().all(|g| !g.members.contains(&id))),
        "the deleted light is not written back into scenes.json"
    );
    let scene = app.store.read().await.scenes.scenes[0].clone();
    assert!(!scene.groups.iter().any(|g| g.members.contains(&id)));
    assert_eq!(scene.rev, 3, "the PUT and the delete each moved the rev once");
}

#[tokio::test]
async fn lights_lists_every_light_with_its_count_and_label() {
    let devices = vec![
        device(
            "fake:mb",
            "MSI B850",
            vec![zone(0, "JARGB 1", 12), zone(1, "JARGB 2", 8)],
        ),
        device("fake:keys", "ROYUAN", vec![zone(0, "Keys", 84)]),
    ];
    let lights = lights_file(vec![
        light("fake:mb#z0", "fake:mb", 0, None, None),
        light("fake:mb#z1p1", "fake:mb", 1, Some((2, 4)), Some("Top fan")),
        // 6 + 8 runs past the zone's 8 LEDs: the count is what is left.
        light("fake:mb#z1p2", "fake:mb", 1, Some((6, 8)), None),
        light("fake:keys#z0", "fake:keys", 0, None, None),
    ]);

    let (router, _app, _home) = build(lights, devices).await;
    let (status, body) = get_lights(router).await;

    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 4, "{body}");
    let ids: Vec<&str> = answer.lights.iter().map(|v| v.light.id.as_str()).collect();
    assert_eq!(ids, vec!["fake:mb#z0", "fake:mb#z1p1", "fake:mb#z1p2", "fake:keys#z0"]);
    let counts: Vec<u32> = answer.lights.iter().map(|v| v.led_count).collect();
    assert_eq!(counts, vec![12, 4, 2, 84], "{body}");
    let labels: Vec<&str> = answer.lights.iter().map(|v| v.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["MSI B850 JARGB 1", "Top fan", "MSI B850 JARGB 2", "ROYUAN Keys"]
    );
    assert!(answer.lights.iter().all(|v| v.connected));
}

#[tokio::test]
async fn a_light_of_an_absent_device_is_listed_disconnected_with_no_leds() {
    let lights = lights_file(vec![light("fake:gone#z0", "fake:gone", 0, None, None)]);
    let (router, _app, _home) = build(lights, Vec::new()).await;
    let (status, body) = get_lights(router).await;

    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    let view = &answer.lights[0];
    assert!(!view.connected, "{body}");
    assert_eq!(view.led_count, 0, "{body}");
    // There is no device name to fall back to, so the light keeps its id.
    assert_eq!(view.label, "fake:gone#z0");
}

#[tokio::test]
async fn light_patch_renames_clears_and_sets_ring_and_reversed() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, Some("Old"))]);
    let (router, app, home) = build(lights, devices).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"name":"  Top fan  ","arrangement":"ring","reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    assert_eq!(view.light.name.as_deref(), Some("Top fan"), "{body}");
    assert_eq!(view.light.arrangement, Arrangement::Ring);
    assert!(view.light.reversed);
    assert_eq!(view.label, "Top fan", "{body}");
    assert_eq!(view.led_count, 12);

    // The file holds the change, so it survives a restart.
    let saved: LightsFile =
        serde_json::from_str(&std::fs::read_to_string(home.path.join("lights.json")).unwrap())
            .expect("lights.json is a lights file");
    assert_eq!(saved.lights[0].name.as_deref(), Some("Top fan"));
    assert_eq!(saved.lights[0].arrangement, Arrangement::Ring);
    assert!(saved.lights[0].reversed);

    // A null name clears it; the fields left out stay as they were.
    let response = router
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"name":null}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    assert_eq!(view.light.name, None, "{body}");
    assert_eq!(view.label, "MSI B850 JARGB 1", "{body}");
    assert_eq!(view.light.arrangement, Arrangement::Ring);
    assert!(view.light.reversed);
    assert_eq!(app.store.read().await.lights.lights[0].name, None);
}

#[tokio::test]
async fn light_patch_sets_and_resets_first_led_and_round_trips_it() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, home) = build(lights, devices).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"first_led":3}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    assert_eq!(view.light.first_led, Some(3), "{body}");

    // lights.json carries it, so a restart keeps the owner's first LED.
    let saved: LightsFile =
        serde_json::from_str(&std::fs::read_to_string(home.path.join("lights.json")).unwrap())
            .expect("lights.json is a lights file");
    assert_eq!(saved.lights[0].first_led, Some(3));
    assert_eq!(app.store.read().await.lights.lights[0].first_led, Some(3));

    // 0 resets it to None, on disk too.
    let response = router
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"first_led":0}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(app.store.read().await.lights.lights[0].first_led, None);
    let saved: LightsFile =
        serde_json::from_str(&std::fs::read_to_string(home.path.join("lights.json")).unwrap())
            .expect("lights.json is a lights file");
    assert_eq!(saved.lights[0].first_led, None, "0 is stored as None");
}

/// A reversed light with no first LED plays the old full flip, so LED 0 as the first LED of a
/// reversed run has to survive the PATCH as a stored 0 (#468). Without it the owner could never
/// put the first LED back on LED 0 of a reversed light.
#[tokio::test]
async fn light_patch_keeps_a_first_led_of_zero_on_a_reversed_light() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;
    let patch = |body: &'static str| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), body))
                .await
                .expect("the router answers");
            assert_eq!(response.status(), StatusCode::OK);
        }
    };

    // Reversed and anchored on LED 0 in one patch: stored as 0, not dropped.
    patch(r#"{"reversed":true,"first_led":0}"#).await;
    let stored = app.store.read().await.lights.lights[0].clone();
    assert!(stored.reversed);
    assert_eq!(stored.first_led, Some(0), "LED 0 stays first on a reversed light");

    // The same on a light that is already reversed.
    patch(r#"{"first_led":3}"#).await;
    patch(r#"{"first_led":0}"#).await;
    assert_eq!(app.store.read().await.lights.lights[0].first_led, Some(0));

    // Back to forward with 0: nothing to store, as before #468.
    patch(r#"{"reversed":false,"first_led":0}"#).await;
    let stored = app.store.read().await.lights.lights[0].clone();
    assert!(!stored.reversed);
    assert_eq!(stored.first_led, None);
}

#[tokio::test]
async fn light_patch_refuses_a_first_led_at_or_past_the_led_count() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 8)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    for bad in [8u32, 9, 100] {
        let body = serde_json::json!({ "first_led": bad }).to_string();
        let response = router
            .clone()
            .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &body))
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let json = json(response).await;
        let error: ApiError = serde_json::from_value(json.clone()).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
        assert!(
            error.message.contains("first LED must be below"),
            "a plain message about the limit: {json}"
        );
        assert_eq!(app.store.read().await.lights.lights[0].first_led, None, "nothing was saved");
    }
}

#[tokio::test]
async fn a_light_whose_zone_shrank_shows_no_first_led() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 8)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;
    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"first_led":5}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The zone reports fewer LEDs than the stored first LED. The view drops it (the editor shows
    // no notch) while the stored value is left alone, so growing the zone back restores it.
    *app.devices.write().await = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 4)])];
    let (status, body) = get_lights(router).await;
    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights[0].led_count, 4);
    assert_eq!(answer.lights[0].light.first_led, None, "the shrunk light has no first LED: {body}");
    assert_eq!(app.store.read().await.lights.lights[0].first_led, Some(5), "still stored");
}

#[tokio::test]
async fn light_patch_refuses_an_empty_or_too_long_name() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    for bad in [
        r#"{"name":""}"#.to_string(),
        r#"{"name":"   "}"#.to_string(),
        // A name over the limit in characters, not bytes.
        serde_json::json!({ "name": "x".repeat(GROUP_NAME_MAX + 1) }).to_string(),
        serde_json::json!({ "name": "é".repeat(GROUP_NAME_MAX + 1) }).to_string(),
    ] {
        let response = router
            .clone()
            .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &bad))
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
        assert_eq!(app.store.read().await.lights.lights[0].name, None, "nothing was saved");
    }

    // Exactly the limit is accepted.
    let at_max = serde_json::json!({ "name": "x".repeat(GROUP_NAME_MAX) }).to_string();
    let response = router
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &at_max))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn light_patch_of_an_unknown_light_is_404() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, _home) = build(lights, devices).await;

    let response = router
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z9"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::NOT_FOUND);
}

#[tokio::test]
async fn a_refused_lights_save_is_a_503_with_the_message() {
    // `lights.json` could not be read at start: the store refuses to write over it.
    let home = TempHome::new();
    let mut store = Store::at(&home.path);
    store.lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    store.lights_blocked = true;
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let (router, _app, _home) = build_with(store, devices, home).await;

    let response = router
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::SERVICE_UNAVAILABLE);
    assert!(error.message.contains("lights.json"), "{}", error.message);
}

#[tokio::test]
async fn the_lights_routes_need_the_token() {
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, _home) = build(lights, Vec::new()).await;

    // The routes are there: with the token they answer.
    let with_token = request("GET", routes::LIGHTS);
    let response = router.clone().oneshot(with_token).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    for (method, path) in [
        ("GET", routes::LIGHTS.to_string()),
        ("PATCH", light_path("fake:mb#z0")),
        ("DELETE", light_path("fake:mb#z0")),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("host", "127.0.0.1:9430")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reversed":true}"#.to_string()))
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method} {path}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::UNAUTHORIZED);
    }
}

/// The guard is on the whole router, so the lights routes answer a browser from another site and
/// a request that names another host the way the other routes do.
#[tokio::test]
async fn the_lights_routes_refuse_a_foreign_origin_and_a_foreign_host() {
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, _home) = build(lights, Vec::new()).await;

    for (method, path) in [
        ("GET", routes::LIGHTS.to_string()),
        ("PATCH", light_path("fake:mb#z0")),
        ("DELETE", light_path("fake:mb#z0")),
    ] {
        // A browser from another site is refused.
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("host", "127.0.0.1:9430")
                    .header("origin", "https://evil.example")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reversed":true}"#.to_string()))
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "origin: {method} {path}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::FORBIDDEN_ORIGIN);

        // A request that names another host is refused too.
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("host", "evil.example")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reversed":true}"#.to_string()))
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "host: {method} {path}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::BAD_HOST);
    }
}

/// A run that starts past the end of its zone owns no LED at all: there is nothing to light.
#[tokio::test]
async fn a_light_whose_run_starts_past_its_zone_has_no_leds() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    // The run starts at LED 20 of a zone with 12 LEDs.
    let lights = lights_file(vec![light("fake:mb#z0p2", "fake:mb", 0, Some((20, 4)), None)]);
    let (router, _app, _home) = build(lights, devices).await;

    let (status, body) = get_lights(router).await;
    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    assert_eq!(answer.lights[0].led_count, 0, "{body}");
}

/// A patch that leaves the name out keeps the name the light had.
#[tokio::test]
async fn a_patch_without_a_name_keeps_the_name_the_light_had() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, Some("Top fan"))]);
    let (router, app, _home) = build(lights, devices).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    assert_eq!(view.light.name.as_deref(), Some("Top fan"), "{body}");
    assert!(view.light.reversed, "{body}");
    assert_eq!(app.store.read().await.lights.lights[0].name.as_deref(), Some("Top fan"));
}

/// A save that is refused leaves the light as it was: the list answers the old values.
#[tokio::test]
async fn a_refused_save_leaves_the_light_as_it_was() {
    // `lights.json` could not be read at start: the store refuses to write over it.
    let home = TempHome::new();
    let mut store = Store::at(&home.path);
    store.lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, Some("Top fan"))]);
    store.lights_blocked = true;
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let (router, _app, _home) = build_with(store, devices, home).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"name":"New fan","reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    // The light is as it was: the refused save changed nothing.
    let (status, body) = get_lights(router).await;
    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    assert_eq!(answer.lights[0].light.name.as_deref(), Some("Top fan"), "{body}");
    assert!(!answer.lights[0].light.reversed, "{body}");
}

/// The name's limit is on characters, not bytes: forty multi-byte characters are forty
/// characters and are accepted, forty-one are refused.
#[tokio::test]
async fn a_name_is_counted_in_characters_not_bytes() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    let at_max = serde_json::json!({ "name": "é".repeat(GROUP_NAME_MAX) }).to_string();
    let response = router
        .clone()
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &at_max))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK, "40 characters are 40 characters");
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    let expected = "é".repeat(GROUP_NAME_MAX);
    assert_eq!(view.light.name.as_deref(), Some(expected.as_str()), "{body}");

    // One more is over the limit.
    let over = serde_json::json!({ "name": "é".repeat(GROUP_NAME_MAX + 1) }).to_string();
    let response = router
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &over))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
    assert_eq!(
        app.store.read().await.lights.lights[0].name.as_deref(),
        Some(expected.as_str()),
        "the refused name was not saved"
    );
}

/// A body the route cannot read is refused in the API's own error shape, like every other
/// validation error, and nothing is saved.
#[tokio::test]
async fn an_unknown_arrangement_is_refused_in_the_api_error_shape() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"arrangement":"spiral"}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
    assert_eq!(
        app.store.read().await.lights.lights[0].arrangement,
        Arrangement::Strip,
        "nothing was saved"
    );
}

/// A patch that changes nothing leaves the file on disk alone.
#[tokio::test]
async fn a_patch_that_changes_nothing_does_not_rewrite_the_lights_file() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, home) = build(lights, devices).await;

    // A change, so the file is there.
    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The file's time is put in the past, so a rewrite of the same bytes would show as a new
    // time. The clock is not read to decide anything: the time is set, then compared.
    let path = home.path.join("lights.json");
    let stamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    let file = std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("the lights file is there");
    file.set_times(std::fs::FileTimes::new().set_modified(stamp))
        .expect("the file's time is set");
    assert_eq!(modified_at(&path), stamp, "the file's time was set");

    // A patch that changes nothing is answered, and the file is not written.
    let response = router
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), r#"{}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(modified_at(&path), stamp, "the empty patch rewrote lights.json");
}

/// When the file was last written.
fn modified_at(path: &std::path::Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .expect("the file is there")
        .modified()
        .expect("the file has a time")
}

/// The snapshot folders a delete left, newest last.
fn delete_snapshots(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(home.join("backups")) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("-before-removing-a-light"))
        })
        .collect();
    dirs.sort();
    dirs
}

/// The snapshot is taken before the write: the copy holds `lights.json` and `scenes.json` exactly
/// as they were the moment before the light was removed, while the live files no longer do.
#[tokio::test]
async fn the_snapshot_holds_both_files_byte_for_byte_as_they_were_before_the_delete() {
    let id = format!("{UNDRIVEN}#z0p1");
    let home = TempHome::new();
    std::fs::write(
        home.path.join(canvas_proto::scene::LIGHTS_FILE),
        serde_json::to_vec(&lights_file(vec![light(
            &id,
            UNDRIVEN,
            0,
            Some((0, 8)),
            Some("Old strip"),
        )]))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        home.path.join(canvas_proto::scene::SCENES_FILE),
        serde_json::to_vec(&scenes_with("g-old", "Old keyboard", &[&id])).unwrap(),
    )
    .unwrap();
    let store = Store::load_from(&home.path);
    // What is on disk the moment before the write, whatever the load did with it.
    let lights_before = std::fs::read(home.path.join(canvas_proto::scene::LIGHTS_FILE)).unwrap();
    let scenes_before = std::fs::read(home.path.join(canvas_proto::scene::SCENES_FILE)).unwrap();
    let (router, _app, home) = build_with(store, Vec::new(), home).await;

    let response = router.oneshot(delete_request(&id)).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    let snaps = delete_snapshots(&home.path);
    assert_eq!(snaps.len(), 1, "one snapshot was taken: {snaps:?}");
    assert_eq!(
        std::fs::read(snaps[0].join(canvas_proto::scene::LIGHTS_FILE)).unwrap(),
        lights_before,
        "the snapshot's lights.json is byte-for-byte the pre-delete file"
    );
    assert_eq!(
        std::fs::read(snaps[0].join(canvas_proto::scene::SCENES_FILE)).unwrap(),
        scenes_before,
        "the snapshot's scenes.json is byte-for-byte the pre-delete file"
    );
    assert_ne!(
        std::fs::read(home.path.join(canvas_proto::scene::LIGHTS_FILE)).unwrap(),
        lights_before,
        "the live lights.json changed"
    );
    assert_ne!(
        std::fs::read(home.path.join(canvas_proto::scene::SCENES_FILE)).unwrap(),
        scenes_before,
        "the live scenes.json changed"
    );
}

/// Deleting a light the daemon does not know is a 404.
#[tokio::test]
async fn deleting_an_unknown_light_is_404() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    let response = router.oneshot(delete_request("fake:mb#z9")).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::NOT_FOUND);
    assert_eq!(app.store.read().await.lights.lights.len(), 1, "nothing was removed");
}

/// A light on a device the daemon drives is refused: those go through chain setup.
#[tokio::test]
async fn deleting_a_light_on_a_driven_device_is_a_409_and_changes_nothing() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, home) = build(lights, devices).await;

    let response = router.oneshot(delete_request("fake:mb#z0")).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::CONFLICT);
    assert!(app.store.read().await.lights.lights.iter().any(|l| l.id == "fake:mb#z0"));
    assert!(delete_snapshots(&home.path).is_empty(), "no snapshot was taken");
}

/// A snapshot that cannot be taken answers 503 and changes nothing.
#[tokio::test]
async fn a_failed_snapshot_refuses_the_delete_and_changes_nothing() {
    let id = format!("{UNDRIVEN}#z0p1");
    let home = TempHome::new();
    let lights_bytes = serde_json::to_vec(&lights_file(vec![light(
        &id,
        UNDRIVEN,
        0,
        Some((0, 8)),
        Some("Old strip"),
    )]))
    .unwrap();
    let scenes_bytes = serde_json::to_vec(&scenes_with("g-old", "Old keyboard", &[&id])).unwrap();
    std::fs::write(home.path.join(canvas_proto::scene::LIGHTS_FILE), &lights_bytes).unwrap();
    std::fs::write(home.path.join(canvas_proto::scene::SCENES_FILE), &scenes_bytes).unwrap();
    // `backups` is a file, so the snapshot's create_dir_all under it fails.
    std::fs::write(home.path.join("backups"), b"not a folder").unwrap();
    let store = Store::load_from(&home.path);
    let (router, app, home) = build_with(store, Vec::new(), home).await;

    let response = router.oneshot(delete_request(&id)).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::SERVICE_UNAVAILABLE);
    assert!(app.store.read().await.lights.lights.iter().any(|l| l.id == id), "the light stayed");
    assert_eq!(std::fs::read(home.path.join(canvas_proto::scene::LIGHTS_FILE)).unwrap(), lights_bytes);
    assert_eq!(std::fs::read(home.path.join(canvas_proto::scene::SCENES_FILE)).unwrap(), scenes_bytes);
}

/// A save that cannot write answers 503 and leaves the light in memory.
#[tokio::test]
async fn a_failed_save_refuses_the_delete_and_changes_nothing() {
    let id = format!("{UNDRIVEN}#z0p1");
    let home = TempHome::new();
    let mut store = Store::at(&home.path);
    store.lights = lights_file(vec![light(&id, UNDRIVEN, 0, Some((0, 8)), Some("Old strip"))]);
    store.lights_blocked = true;
    let (router, app, _home) = build_with(store, Vec::new(), home).await;

    let response = router.oneshot(delete_request(&id)).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::SERVICE_UNAVAILABLE);
    assert!(app.store.read().await.lights.lights.iter().any(|l| l.id == id), "the light stayed");
}

/// A light of a device that is here but has no such zone is named after the device.
#[tokio::test]
async fn a_light_of_a_zone_the_device_does_not_have_is_labelled_with_the_device_name() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    // The light names zone 7; the device has only zone 0.
    let lights = lights_file(vec![light("fake:mb#z7", "fake:mb", 7, None, None)]);
    let (router, _app, _home) = build(lights, devices).await;

    let (status, body) = get_lights(router).await;
    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    assert_eq!(answer.lights[0].label, "MSI B850", "{body}");
    assert_eq!(answer.lights[0].led_count, 0, "{body}");
    assert!(answer.lights[0].connected, "{body}");
}
