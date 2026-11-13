//! Scenes: the render thread's per-group work.
//!
//! A scene is a list of groups. Each group owns its effect, its props, its brightness, one
//! canvas and a plan (where its lights read that canvas, from [`crate::arrangement`]). One
//! frame is:
//!
//! 1. every group runs its effect on its own canvas, behind the guard that catches panics;
//! 2. every light of the group is sampled from that canvas and scaled by the group's
//!    brightness (these colours are what the editor's live LEDs show);
//! 3. every device's frame is built by writing each light's colours at its `frame_start`;
//!    LEDs no light wrote are black;
//! 4. the global stages (brightness, warmth, the device's white balance) run over the frame;
//! 5. a scene change that asked for a fade blends the frames last sent into this one.
//!
//! The work is split in two so no lock is held while an effect runs. [`SceneState`] lives in the
//! engine's state: the API threads edit it under the lock, and the render thread takes each
//! group's effect out of it for the frame ([`SceneState::take_input`]). [`SceneRunner`] is the
//! render thread's own: canvases, plans and buffers, and it runs the frame without the lock
//! ([`SceneRunner::render`]). The effects go back into the state when the frame is done.
//!
//! The engine stays one render thread: the groups run one after the other.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo};
use canvas_proto::layout::SamplingMode;
use canvas_proto::props::{InputNeeds, Props};
use canvas_proto::scene::{GroupMode, Light, LightId, SCENE_FADE_MS};
use canvas_proto::traits::{
    AudioSpectrum, Canvas, Effect, InputSource, Inputs, LightFrame, ScreenFrame,
};

use crate::arrangement::{build_group_plan, GroupPlan};
use crate::engine::{guarded_call, panic_text, report_effect_panic};
use crate::sampling;

/// The render scale every group's canvas runs at in scene mode: 0.25 is an 80x50 canvas, the
/// size the stock HTML effects draw at. It is fixed, not a setting.
///
/// Why: an HTML group costs the same at every scale (5.4 to 6.7 ms a frame, the script and not
/// the picture), while 20 built-in groups fall from 10.1 ms p95 at 320x200 to 0.70 ms at 80x50,
/// so the smallest canvas buys the most headroom. See "Scenes: cost per group" in
/// `docs/benchmark.md`.
pub const SCENE_RENDER_SCALE: f32 = 0.25;

/// How a light's LEDs read their group's canvas in scene mode: the mean of the pixels the LED's
/// cell covers. It is fixed, not a setting. It is the mode the benchmarks timed, so the cost
/// numbers hold for it; what the other two modes cost is unmeasured.
pub const SCENE_SAMPLING: SamplingMode = SamplingMode::AreaAverage;

/// The light preview (the editor's live LEDs) never exceeds this rate.
const LIGHT_PREVIEW_MAX_FPS: u32 = canvas_proto::consts::PREVIEW_MAX_FPS;

/// The most a single frame advances a fade, as a share of the fade's length. A render thread
/// that stalls for a second still shows the fade over at least four frames instead of jumping
/// from the old scene to the new one.
const FADE_MAX_STEP_DIVISOR: f32 = 4.0;

// ---------------------------------------------------------------- the engine's state

/// One group of the running scene, as the engine's state holds it.
pub(crate) struct GroupSlot {
    pub id: String,
    pub members: Vec<LightId>,
    pub mode: GroupMode,
    /// 0..=100.
    pub brightness: u8,
    /// `None`: the group is dark (set to Off), or the render thread has the effect for the
    /// frame in flight.
    pub effect: Option<Box<dyn Effect>>,
    /// What the effect asked for, read once when it was installed.
    pub needs: InputNeeds,
    /// Unique for the engine's whole life. The render thread puts an effect back only when the
    /// slot still has the generation it took it at: a replaced effect is dropped instead.
    pub effect_generation: u64,
    /// When the effect was installed; `Inputs.time` counts from here.
    pub started: Instant,
    /// Props the effect has not been initialised with yet.
    pub pending_props: Option<Props>,
    /// Why the group is dark: the text of a panic. The effect is not called again until it is
    /// replaced.
    pub failed: Option<String>,
}

/// The running scene, in the engine's state.
pub(crate) struct SceneState {
    /// Bumped on every `set_scene`; the render thread rebuilds its plans when it changes.
    pub generation: u64,
    pub lights: Vec<Light>,
    pub groups: Vec<GroupSlot>,
    /// Set by `set_scene(fade: true)`, cleared when the render thread takes it.
    pub fade_pending: bool,
}

/// What one group hands the render thread for one frame.
pub(crate) struct GroupWork {
    pub id: String,
    pub effect: Option<Box<dyn Effect>>,
    pub effect_generation: u64,
    pub needs: InputNeeds,
    pub started: Instant,
    pub brightness: u8,
    pub pending_props: Option<Props>,
    /// The group already failed: its effect is not called.
    pub failed: bool,
}

/// What the render thread needs to rebuild its plans: sent only when the scene changed.
pub(crate) struct Rebuild {
    lights: Vec<Light>,
    devices: Vec<DeviceInfo>,
    groups: Vec<(Vec<LightId>, GroupMode)>,
}

/// Everything the render thread takes from the scene for one frame.
pub(crate) struct SceneInput {
    generation: u64,
    works: Vec<GroupWork>,
    rebuild: Option<Rebuild>,
    start_fade: bool,
}

impl SceneState {
    /// Takes each group's effect and pending props out for one frame. `seen_generation` is the
    /// scene generation the render thread has plans for.
    pub fn take_input(&mut self, devices: &[DeviceInfo], seen_generation: u64) -> SceneInput {
        let rebuild = (self.generation != seen_generation).then(|| Rebuild {
            lights: self.lights.clone(),
            devices: devices.to_vec(),
            groups: self.groups.iter().map(|g| (g.members.clone(), g.mode)).collect(),
        });
        let works = self
            .groups
            .iter_mut()
            .map(|g| GroupWork {
                id: g.id.clone(),
                failed: g.failed.is_some(),
                // A failed group keeps its effect: it is not called, and nothing else needs it.
                effect: if g.failed.is_some() { None } else { g.effect.take() },
                effect_generation: g.effect_generation,
                needs: g.needs,
                started: g.started,
                brightness: g.brightness,
                pending_props: g.pending_props.take(),
            })
            .collect();
        SceneInput {
            generation: self.generation,
            works,
            rebuild,
            start_fade: std::mem::take(&mut self.fade_pending),
        }
    }

    /// Puts the frame's effects back and records the failures. An effect whose slot was
    /// replaced during the frame (a newer generation, or the scene itself changed) comes back
    /// in the returned list: the caller drops it behind the guard, outside the lock.
    pub fn put_back(&mut self, done: Vec<GroupDone>) -> Vec<Box<dyn Effect>> {
        let mut superseded = Vec::new();
        for item in done {
            let slot = self
                .groups
                .iter_mut()
                .find(|g| g.id == item.id && g.effect_generation == item.effect_generation);
            match slot {
                Some(slot) => {
                    if slot.effect.is_none() {
                        slot.effect = item.effect;
                    } else if let Some(e) = item.effect {
                        superseded.push(e);
                    }
                    if let Some(text) = item.failure {
                        slot.failed = Some(text);
                    }
                }
                None => {
                    if let Some(e) = item.effect {
                        superseded.push(e);
                    }
                }
            }
        }
        superseded
    }
}

/// One group's effect coming back from the frame.
pub(crate) struct GroupDone {
    id: String,
    effect: Option<Box<dyn Effect>>,
    effect_generation: u64,
    /// The text of a panic in this frame, if the effect panicked.
    failure: Option<String>,
}

// ---------------------------------------------------------------- the render thread's side

/// A group's own canvas, plan and colours, on the render thread.
struct RunGroup {
    canvas: Canvas,
    /// In render pixels of `canvas`.
    plan: GroupPlan,
    /// One buffer per light of the plan: the light's colours after the group's brightness.
    colors: Vec<Vec<Rgb8>>,
    /// The effect generation the canvas was last cleared for.
    seen_effect_generation: u64,
}

/// One device's frame, being built.
struct DeviceBuf {
    id: DeviceId,
    led_count: usize,
    frame: Vec<Rgb8>,
}

/// A fade from the frames last sent into the new scene's.
struct Fade {
    from: HashMap<DeviceId, Vec<Rgb8>>,
    elapsed_ms: f32,
    /// False until the fade's first frame has been shown (it shows the old frames).
    running: bool,
}

/// The global stages of a frame, snapshotted under the lock.
pub(crate) struct FrameParams {
    pub dt: f32,
    pub now: Instant,
    /// 0.0..=1.0.
    pub brightness: f32,
    pub warmth: [f32; 3],
    pub white_balance: Arc<HashMap<DeviceId, [f32; 3]>>,
    pub input_source: Option<Arc<dyn InputSource>>,
    /// Whether a light preview sink is set.
    pub want_light_preview: bool,
}

/// What a frame gives back besides the device frames.
pub(crate) struct FrameOutput {
    pub done: Vec<GroupDone>,
    pub light_frame: Option<LightFrame>,
    /// How many effect panics this frame caught.
    pub panics: u64,
}

/// The render thread's scene state. Built empty; the first frame after a `set_scene` fills it.
pub(crate) struct SceneRunner {
    generation: u64,
    groups: Vec<RunGroup>,
    devices: Vec<DeviceBuf>,
    device_index: HashMap<DeviceId, usize>,
    /// The frames last produced, after every stage, for a fade to start from.
    last_sent: HashMap<DeviceId, Vec<Rgb8>>,
    fade: Option<Fade>,
    last_light_preview_at: Option<Instant>,
}

impl Default for SceneRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneRunner {
    pub fn new() -> Self {
        Self {
            generation: u64::MAX,
            groups: Vec::new(),
            devices: Vec::new(),
            device_index: HashMap::new(),
            last_sent: HashMap::new(),
            fade: None,
            last_light_preview_at: None,
        }
    }

    /// The scene generation this runner has plans for.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Runs one frame: effects, sampling, merge, global stages, fade. The device frames wait in
    /// the runner for [`SceneRunner::take_frame`].
    pub fn render(&mut self, input: SceneInput, params: &FrameParams) -> FrameOutput {
        if let Some(rebuild) = input.rebuild {
            self.rebuild(rebuild, input.generation);
            // A new scene replaces a fade that was still running; `start_fade` below starts a
            // new one when this scene asked for it.
            self.fade = None;
        }
        if input.start_fade {
            self.begin_fade();
        }

        // Capture data is read once per frame and handed to every group.
        let (screen, audio) = read_captures(&input.works, params.input_source.as_ref());

        let mut done = Vec::with_capacity(input.works.len());
        let mut panics = 0;
        for (index, work) in input.works.into_iter().enumerate() {
            let Some(group) = self.groups.get_mut(index) else {
                // Cannot happen: the plans are rebuilt whenever the scene changes. Keep the
                // effect so it goes back to its slot.
                done.push(GroupDone {
                    id: work.id,
                    effect: work.effect,
                    effect_generation: work.effect_generation,
                    failure: None,
                });
                continue;
            };
            let (item, panicked) = run_group(group, work, params.dt, &screen, &audio);
            panics += u64::from(panicked);
            done.push(item);
        }

        self.merge_devices();
        let light_frame = self.light_frame(params);
        self.apply_global_stages(params);
        self.apply_fade(params.dt);
        for device in &self.devices {
            self.last_sent.entry(device.id.clone()).or_default().clone_from(&device.frame);
        }
        FrameOutput { done, light_frame, panics }
    }

    /// Takes a device's finished frame for its sink. `None` when the runner has no frame for
    /// that device, or the frame's length is not the length the sink expects: a frame built
    /// for the old size of a zone that has since changed is dropped, never sent.
    pub fn take_frame(&mut self, device: &DeviceId, led_count: usize) -> Option<Vec<Rgb8>> {
        let buf = &mut self.devices[*self.device_index.get(device)?];
        let frame = std::mem::replace(&mut buf.frame, Vec::with_capacity(buf.led_count));
        (frame.len() == led_count).then_some(frame)
    }

    fn rebuild(&mut self, rebuild: Rebuild, generation: u64) {
        self.generation = generation;
        self.devices = rebuild
            .devices
            .iter()
            .filter(|d| d.led_count() > 0)
            .map(|d| DeviceBuf {
                id: d.id.clone(),
                led_count: d.led_count() as usize,
                frame: Vec::new(),
            })
            .collect();
        self.device_index =
            self.devices.iter().enumerate().map(|(i, d)| (d.id.clone(), i)).collect();
        self.groups = rebuild
            .groups
            .iter()
            .map(|(members, mode)| {
                let mut plan = build_group_plan(members, *mode, &rebuild.lights, &rebuild.devices);
                scale_plan(&mut plan, SCENE_RENDER_SCALE);
                let colors =
                    plan.lights.iter().map(|l| vec![Rgb8::BLACK; l.leds.len()]).collect();
                RunGroup {
                    canvas: Canvas::with_scale(SCENE_RENDER_SCALE),
                    plan,
                    colors,
                    seen_effect_generation: u64::MAX,
                }
            })
            .collect();
    }

    fn begin_fade(&mut self) {
        // Nothing was sent yet (the first scene after a start, or after layout mode): cut.
        if self.last_sent.is_empty() {
            return;
        }
        self.fade = Some(Fade { from: self.last_sent.clone(), elapsed_ms: 0.0, running: false });
    }

    /// Every device's frame: black, then each light's colours at its `frame_start`.
    fn merge_devices(&mut self) {
        for device in &mut self.devices {
            device.frame.clear();
            device.frame.resize(device.led_count, Rgb8::BLACK);
        }
        for group in &self.groups {
            for (light, colors) in group.plan.lights.iter().zip(&group.colors) {
                let Some(&index) = self.device_index.get(&light.device_id) else { continue };
                let frame = &mut self.devices[index].frame;
                let start = light.frame_start as usize;
                for (offset, color) in colors.iter().enumerate() {
                    if let Some(slot) = frame.get_mut(start + offset) {
                        *slot = *color;
                    }
                }
            }
        }
    }

    /// The editor's live LEDs: each light's colours after the group's brightness and before the
    /// global stages, at most [`LIGHT_PREVIEW_MAX_FPS`] a second.
    fn light_frame(&mut self, params: &FrameParams) -> Option<LightFrame> {
        if !params.want_light_preview {
            return None;
        }
        let interval = Duration::from_nanos(1_000_000_000 / u64::from(LIGHT_PREVIEW_MAX_FPS));
        if self
            .last_light_preview_at
            .is_some_and(|at| params.now.duration_since(at) < interval)
        {
            return None;
        }
        self.last_light_preview_at = Some(params.now);
        let lights = self
            .groups
            .iter()
            .flat_map(|g| g.plan.lights.iter().zip(&g.colors))
            .map(|(light, colors)| (light.light.clone(), colors.clone()))
            .collect();
        Some(LightFrame { lights })
    }

    /// Global brightness, warmth and the device's white balance, as one gain per channel.
    fn apply_global_stages(&mut self, params: &FrameParams) {
        for device in &mut self.devices {
            let balance = params.white_balance.get(&device.id).copied().unwrap_or([1.0; 3]);
            let gain = [0, 1, 2].map(|i| params.brightness * params.warmth[i] * balance[i]);
            if gain.iter().any(|g| *g < 1.0) {
                for c in device.frame.iter_mut() {
                    let f = c.to_f32();
                    *c = Rgb8::from_f32([f[0] * gain[0], f[1] * gain[1], f[2] * gain[2]]);
                }
            }
        }
    }

    /// Blends the frames last sent into this scene's while a fade runs.
    fn apply_fade(&mut self, dt: f32) {
        let Some(fade) = self.fade.as_mut() else { return };
        let fade_ms = SCENE_FADE_MS as f32;
        if fade.running {
            fade.elapsed_ms += (dt * 1000.0).min(fade_ms / FADE_MAX_STEP_DIVISOR);
        } else {
            fade.running = true;
        }
        let t = fade.elapsed_ms / fade_ms;
        if t >= 1.0 {
            self.fade = None;
            return;
        }
        for device in &mut self.devices {
            // A device that was not there, or changed its LED count, has nothing to fade from.
            let Some(from) = fade.from.get(&device.id).filter(|f| f.len() == device.frame.len())
            else {
                continue;
            };
            for (to, from) in device.frame.iter_mut().zip(from) {
                *to = Rgb8::new(
                    lerp(from.r, to.r, t),
                    lerp(from.g, to.g, t),
                    lerp(from.b, to.b, t),
                );
            }
        }
    }
}

fn lerp(from: u8, to: u8, t: f32) -> u8 {
    let (a, b) = (f32::from(from), f32::from(to));
    (a + (b - a) * t).round().clamp(0.0, 255.0) as u8
}

/// The plan's samples are in the group's canvas units (320x200); the canvas is `scale` of that.
fn scale_plan(plan: &mut GroupPlan, scale: f32) {
    for light in &mut plan.lights {
        for led in &mut light.leds {
            led.center_x *= scale;
            led.center_y *= scale;
            led.bounds = led.bounds.map(|v| v * scale);
        }
    }
}

/// Reads the screen and audio once, when any live group asks for them.
fn read_captures(
    works: &[GroupWork],
    source: Option<&Arc<dyn InputSource>>,
) -> (Option<Arc<ScreenFrame>>, Option<Arc<AudioSpectrum>>) {
    let live = || works.iter().filter(|w| !w.failed && w.effect.is_some());
    let screen = live().any(|w| w.needs.screen).then(|| source?.screen()).flatten();
    let audio = live().any(|w| w.needs.audio).then(|| source?.audio()).flatten();
    (screen, audio)
}

/// One group's frame: its effect on its canvas, then its lights sampled and scaled. Returns the
/// effect going back to the state, and whether it panicked this frame.
fn run_group(
    group: &mut RunGroup,
    work: GroupWork,
    dt: f32,
    screen: &Option<Arc<ScreenFrame>>,
    audio: &Option<Arc<AudioSpectrum>>,
) -> (GroupDone, bool) {
    // A new effect never starts from the previous effect's frame.
    if group.seen_effect_generation != work.effect_generation {
        group.seen_effect_generation = work.effect_generation;
        group.canvas.fill([0.0; 3]);
    }

    let mut effect = work.effect;
    let mut failure: Option<String> = None;
    if let Some(e) = effect.as_mut() {
        if let Some(props) = work.pending_props.as_ref() {
            if let Err(payload) = guarded_call(|| e.init(props)) {
                report_effect_panic(e.as_ref(), "init", &payload);
                failure = Some(panic_text(&payload));
            }
        }
        if failure.is_none() {
            let inputs = Inputs {
                time: work.started.elapsed().as_secs_f64(),
                screen: screen.clone(),
                audio: audio.clone(),
            };
            let canvas = &mut group.canvas;
            if let Err(payload) = guarded_call(|| e.update(dt, canvas, &inputs)) {
                report_effect_panic(e.as_ref(), "update", &payload);
                failure = Some(panic_text(&payload));
            }
        }
    }

    // A group with no effect, or one that has failed, is black.
    let live = !work.failed && failure.is_none() && effect.is_some();
    let factor = f32::from(work.brightness.min(100)) / 100.0;
    for (light, out) in group.plan.lights.iter().zip(group.colors.iter_mut()) {
        if live {
            sampling::sample_leds(
                &group.canvas.pixels,
                group.canvas.width,
                group.canvas.height,
                &light.leds,
                SCENE_SAMPLING,
                out,
            );
            if factor < 1.0 {
                for c in out.iter_mut() {
                    let f = c.to_f32();
                    *c = Rgb8::from_f32([f[0] * factor, f[1] * factor, f[2] * factor]);
                }
            }
        } else {
            out.clear();
            out.resize(light.leds.len(), Rgb8::BLACK);
        }
    }

    let panicked = failure.is_some();
    (
        GroupDone {
            id: work.id,
            effect,
            effect_generation: work.effect_generation,
            failure,
        },
        panicked,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::{DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::scene::{zone_light_id, Arrangement};

    fn strip_device(id: &str, leds: u32) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.to_string()),
            name: id.to_string(),
            vendor: "test".into(),
            kind: DeviceKind::Other,
            adapter: "fake".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones: vec![ZoneInfo {
                index: 0,
                name: "zone0".into(),
                kind: ZoneKind::Linear,
                led_count: leds,
                min_leds: leds,
                max_leds: leds,
                resizable: false,
            }],
            connected: true,
        }
    }

    fn light(id: &str) -> Light {
        let device_id = DeviceId(id.to_string());
        Light {
            id: zone_light_id(&device_id, 0),
            device_id,
            zone_index: 0,
            leds: None,
            name: None,
            arrangement: Arrangement::Strip,
            reversed: false,
        }
    }

    fn state(group_members: &[&str], lights: Vec<Light>) -> SceneState {
        SceneState {
            generation: 1,
            lights,
            groups: vec![GroupSlot {
                id: "g".into(),
                members: group_members.iter().map(|m| m.to_string()).collect(),
                mode: GroupMode::FlowAcross,
                brightness: 100,
                effect: None,
                needs: InputNeeds::default(),
                effect_generation: 1,
                started: Instant::now(),
                pending_props: None,
                failed: None,
            }],
            fade_pending: false,
        }
    }

    fn params() -> FrameParams {
        FrameParams {
            dt: 1.0 / 30.0,
            now: Instant::now(),
            brightness: 1.0,
            warmth: [1.0; 3],
            white_balance: Arc::new(HashMap::new()),
            input_source: None,
            want_light_preview: false,
        }
    }

    /// A frame built for the old LED count of a zone is never handed to a sink that expects the
    /// new one (Review Focus 2), and the frame of the right length is.
    #[test]
    fn a_frame_of_the_old_length_is_dropped_not_sent() {
        let mut scene = state(&["d1#z0"], vec![light("d1")]);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 12)];
        let input = scene.take_input(&devices, runner.generation());
        runner.render(input, &params());
        let id = DeviceId("d1".into());
        // The zone grew to 20 LEDs after the frame was built: its sink now expects 20.
        assert_eq!(runner.take_frame(&id, 20), None, "a 12-LED frame for a 20-LED sink");

        let input = scene.take_input(&devices, runner.generation());
        runner.render(input, &params());
        let frame = runner.take_frame(&id, 12).expect("a frame of the right length");
        assert_eq!(frame.len(), 12);
    }

    /// The plan is rebuilt when the scene is replaced with the zone's new size: the next frame
    /// has the new length.
    #[test]
    fn a_new_scene_rebuilds_the_frame_to_the_new_length() {
        let mut runner = SceneRunner::new();
        let mut scene = state(&["d1#z0"], vec![light("d1")]);
        let input = scene.take_input(&[strip_device("d1", 12)], runner.generation());
        runner.render(input, &params());
        let mut scene = state(&["d1#z0"], vec![light("d1")]);
        scene.generation = 2;
        let input = scene.take_input(&[strip_device("d1", 20)], runner.generation());
        runner.render(input, &params());
        let frame = runner.take_frame(&DeviceId("d1".into()), 20).expect("a 20-LED frame");
        assert_eq!(frame.len(), 20);
    }

    #[test]
    fn scale_plan_scales_centres_and_cells() {
        let mut plan = build_group_plan(
            &["d1#z0".to_string()],
            GroupMode::FlowAcross,
            &[light("d1")],
            &[strip_device("d1", 4)],
        );
        let before = plan.lights[0].leds[1];
        scale_plan(&mut plan, SCENE_RENDER_SCALE);
        let after = &plan.lights[0].leds[1];
        assert!((after.center_x - before.center_x * SCENE_RENDER_SCALE).abs() < 1e-4);
        assert!((after.bounds[2] - before.bounds[2] * SCENE_RENDER_SCALE).abs() < 1e-4);
    }

    #[test]
    fn the_scene_canvas_is_80_by_50() {
        let canvas = Canvas::with_scale(SCENE_RENDER_SCALE);
        assert_eq!((canvas.width, canvas.height), (80, 50));
    }
}
