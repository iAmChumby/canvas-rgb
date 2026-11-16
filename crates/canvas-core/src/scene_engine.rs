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
//! The frame's group buffers live in the runner and travel with the frame, so the hot path
//! allocates nothing per group.
//!
//! The engine stays one render thread: the groups run one after the other.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo};
use canvas_proto::layout::SamplingMode;
use canvas_proto::props::{InputNeeds, Props};
use canvas_proto::scene::{Arrangement, GroupMode, Light, LightId, SCENE_FADE_MS};
use canvas_proto::traits::{
    AudioSpectrum, Canvas, Effect, InputSource, Inputs, LightFrame, ScreenFrame,
};
use tracing::warn;

use crate::arrangement::{build_group_plan, GroupPlan};
use crate::engine::{guarded_call, panic_text, report_effect_panic};
use crate::pattern_fit::fit_colors;
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
    /// The group's place in the scene's group list. The effect comes back to that place,
    /// and the generation says whether the slot still holds the effect it took.
    pub index: usize,
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
    ///
    /// `works` is the render thread's buffer: it is refilled and handed back with the
    /// frame, so the hot path allocates nothing per group.
    pub fn take_input(
        &mut self,
        devices: &[DeviceInfo],
        seen_generation: u64,
        mut works: Vec<GroupWork>,
    ) -> SceneInput {
        let rebuild = (self.generation != seen_generation).then(|| Rebuild {
            lights: self.lights.clone(),
            devices: devices.to_vec(),
            groups: self.groups.iter().map(|g| (g.members.clone(), g.mode)).collect(),
        });
        works.clear();
        for (index, g) in self.groups.iter_mut().enumerate() {
            works.push(GroupWork {
                index,
                failed: g.failed.is_some(),
                // A failed group keeps its effect: it is not called, and nothing else needs it.
                effect: if g.failed.is_some() { None } else { g.effect.take() },
                effect_generation: g.effect_generation,
                needs: g.needs,
                started: g.started,
                brightness: g.brightness,
                pending_props: g.pending_props.take(),
            });
        }
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
    pub fn put_back(&mut self, done: &mut Vec<GroupDone>) -> Vec<Box<dyn Effect>> {
        let mut superseded = Vec::new();
        for item in done.drain(..) {
            // The slot is the group's place in the list, and the generation says whether it
            // still holds the effect this frame took: a replaced effect never goes back.
            let slot = self
                .groups
                .get_mut(item.index)
                .filter(|slot| slot.effect_generation == item.effect_generation);
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
    /// The group's place in the scene's group list.
    pub(crate) index: usize,
    /// The effect coming back. The caller drops it behind the guard when it has nowhere
    /// to put it.
    pub(crate) effect: Option<Box<dyn Effect>>,
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
    /// The LED count and arrangement of each light of the plan, in `plan.lights` order: what a
    /// pattern is fitted to.
    shapes: Vec<(u32, Arrangement)>,
    /// Whether each light of the plan is `reversed`, in `plan.lights` order. A pattern is stored
    /// in position order, so a reversed light's fitted colours are flipped.
    reversed: Vec<bool>,
    /// The fitted colours of each light of the plan, in the device's LED order and before the
    /// group's brightness. Filled when a pattern is fitted; empty for a canvas effect.
    fitted: Vec<Vec<Rgb8>>,
    /// The effect generation the canvas was last cleared for.
    seen_effect_generation: u64,
}

impl RunGroup {
    /// Fits the group's pattern to every light of its plan: once per change, never per frame.
    /// `fitted` ends in the device's LED order (each light's own `reversed` applied) and before
    /// the group's brightness.
    fn fit_group(&mut self, effect: Option<&dyn Effect>) {
        let Some(pattern) = effect.and_then(|e| e.as_pattern()) else {
            // A canvas effect, or no effect: nothing is fitted.
            self.fitted.clear();
            return;
        };
        let Some(frame) = pattern.frames.first() else {
            // `validate_pattern` refuses a pattern with no frames; a group given one anyway is
            // black rather than a panic on the render thread.
            self.fitted.clear();
            return;
        };
        let source = &frame.colors;
        let fitted: Vec<Vec<Rgb8>> = self
            .shapes
            .iter()
            .enumerate()
            .map(|(index, (leds, arrangement))| {
                let mut colors = fit_colors(source, &pattern.built_on, *leds, *arrangement);
                // The pattern is stored in position order; a reversed light's frame runs the
                // other way, so its fitted colours are flipped once, here.
                if self.reversed.get(index).copied().unwrap_or(false) {
                    colors.reverse();
                }
                colors
            })
            .collect();
        self.fitted = fitted;
    }
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
    /// The frames last sent to a device's sink, whatever mode sent them. A fade starts
    /// from these.
    last_sent: HashMap<DeviceId, Vec<Rgb8>>,
    fade: Option<Fade>,
    last_light_preview_at: Option<Instant>,
    /// The frame's groups, on their way to the state. The buffer is reused every frame.
    works: Vec<GroupWork>,
    /// The groups coming back from the frame. The buffer is reused every frame.
    done: Vec<GroupDone>,
    /// Whether a light's LEDs did not fit the device's frame and the write was dropped.
    /// The warning is logged once, not on every frame.
    warned_out_of_range: bool,
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
            works: Vec::new(),
            done: Vec::new(),
            warned_out_of_range: false,
        }
    }

    /// The scene generation this runner has plans for.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Takes the frame's group buffer out, so the scene can fill it. [`SceneRunner::render`]
    /// puts it back when the frame is done.
    pub fn take_works(&mut self) -> Vec<GroupWork> {
        std::mem::take(&mut self.works)
    }

    /// The groups coming back from the frame. The caller drains it.
    pub fn done(&mut self) -> &mut Vec<GroupDone> {
        &mut self.done
    }

    /// Runs one frame: effects, sampling, merge, global stages, fade. The device frames wait in
    /// the runner for [`SceneRunner::take_frame`].
    pub fn render(&mut self, mut input: SceneInput, params: &FrameParams) -> FrameOutput {
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

        // The frame's buffer comes back to the runner, so the next frame allocates nothing.
        let mut works = std::mem::take(&mut input.works);
        self.done.clear();
        let mut panics = 0;
        for (index, work) in works.drain(..).enumerate() {
            let Some(group) = self.groups.get_mut(index) else {
                // Cannot happen: the plans are rebuilt whenever the scene changes. Keep the
                // effect so it goes back to its slot.
                self.done.push(GroupDone {
                    index: work.index,
                    effect: work.effect,
                    effect_generation: work.effect_generation,
                    failure: None,
                });
                continue;
            };
            let (item, panicked) = run_group(group, work, params.dt, &screen, &audio);
            panics += u64::from(panicked);
            self.done.push(item);
        }

        // The frame's buffer goes back to the runner: the next frame allocates nothing.
        self.works = works;

        self.merge_devices();
        let light_frame = self.light_frame(params);
        self.apply_global_stages(params);
        self.apply_fade(params.dt);
        FrameOutput { light_frame, panics }
    }

    /// Records a frame that was sent to a device's sink, whatever mode sent it.
    ///
    /// A fade blends from the frame the device was last sent, not from the frame the last
    /// scene produced: layout mode's frames go here too, so a fade that follows layout
    /// mode starts from what the strip is showing and not from a scene that ended long
    /// ago. The copy reuses its capacity, so the steady state allocates nothing.
    pub fn record_sent(&mut self, device: &DeviceId, frame: &[Rgb8]) {
        match self.last_sent.get_mut(device) {
            Some(saved) => {
                saved.clear();
                saved.extend_from_slice(frame);
            }
            None => {
                self.last_sent.insert(device.clone(), frame.to_vec());
            }
        }
    }

    /// Takes a device's finished frame for its sink. `None` when the runner has no frame for
    /// that device, or the frame's length is not the length the sink expects: a frame built
    /// for the old size of a zone that has since changed is dropped, never sent.
    pub fn take_frame(&mut self, device: &DeviceId, led_count: usize) -> Option<Vec<Rgb8>> {
        let buf = &mut self.devices[*self.device_index.get(device)?];
        let frame = std::mem::replace(&mut buf.frame, Vec::with_capacity(buf.led_count));
        (frame.len() == led_count).then_some(frame)
    }

    /// Whether the scene holds this device. A sink whose device the scene does not hold
    /// gets a black frame of its own length, like layout mode does, instead of nothing.
    pub fn knows_device(&self, device: &DeviceId) -> bool {
        self.device_index.contains_key(device)
    }

    fn rebuild(&mut self, rebuild: Rebuild, generation: u64) {
        self.generation = generation;
        // A new scene may have its own out-of-range write: it is logged once, for this
        // scene.
        self.warned_out_of_range = false;
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
                // What a pattern is fitted to, read from the lights the plan was built from.
                let mut shapes = Vec::with_capacity(plan.lights.len());
                let mut reversed = Vec::with_capacity(plan.lights.len());
                for samples in &plan.lights {
                    let light = rebuild.lights.iter().find(|l| l.id == samples.light);
                    shapes.push((
                        samples.leds.len() as u32,
                        light.map(|l| l.arrangement).unwrap_or(Arrangement::Strip),
                    ));
                    reversed.push(light.is_some_and(|l| l.reversed));
                }
                RunGroup {
                    canvas: Canvas::with_scale(SCENE_RENDER_SCALE),
                    plan,
                    colors,
                    shapes,
                    reversed,
                    fitted: Vec::new(),
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
                    match frame.get_mut(start + offset) {
                        Some(slot) => *slot = *color,
                        None => {
                            // A light's LEDs do not fit the device's frame: the write is
                            // dropped. It is logged once, not on every frame.
                            if !self.warned_out_of_range {
                                self.warned_out_of_range = true;
                                warn!(
                                    device = %light.device_id,
                                    start,
                                    leds = colors.len(),
                                    frame = frame.len(),
                                    "a light's LEDs do not fit the device's frame; the writes past its end are dropped"
                                );
                            }
                        }
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
    let mut effect = work.effect;
    let is_pattern = effect.as_deref().and_then(|e| e.as_pattern()).is_some();
    // A new effect never starts from the previous effect's frame. A pattern is fitted here too:
    // a new effect, a new plan and a resized zone all change the generation, and a frame never
    // does. A pattern never draws on its canvas, so the canvas is not cleared for one.
    if group.seen_effect_generation != work.effect_generation {
        group.seen_effect_generation = work.effect_generation;
        if !is_pattern {
            group.canvas.fill([0.0; 3]);
        }
        group.fit_group(effect.as_deref());
    }

    let mut failure: Option<String> = None;
    // A pattern is read, never drawn: `init` and `update` are not called for one.
    if !is_pattern {
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
    }

    // A group with no effect, or one that has failed, is black. A pattern group copies the
    // colours fitted for each light; a canvas group samples the canvas.
    let live = !work.failed && failure.is_none() && effect.is_some();
    let factor = f32::from(work.brightness.min(100)) / 100.0;
    for (index, (light, out)) in
        group.plan.lights.iter().zip(group.colors.iter_mut()).enumerate()
    {
        match group.fitted.get(index).map(Vec::as_slice) {
            Some(fitted) if live && !fitted.is_empty() => {
                out.clear();
                out.extend_from_slice(fitted);
                scale_light(out, factor);
            }
            _ if live => {
                sampling::sample_leds(
                    &group.canvas.pixels,
                    group.canvas.width,
                    group.canvas.height,
                    &light.leds,
                    SCENE_SAMPLING,
                    out,
                );
                scale_light(out, factor);
            }
            _ => {
                out.clear();
                out.resize(light.leds.len(), Rgb8::BLACK);
            }
        }
    }

    let panicked = failure.is_some();
    (
        GroupDone {
            index: work.index,
            effect,
            effect_generation: work.effect_generation,
            failure,
        },
        panicked,
    )
}

/// Multiplies every colour of one light by the group's brightness (`factor` is 0.0..=1.0).
fn scale_light(colors: &mut [Rgb8], factor: f32) {
    if factor < 1.0 {
        for c in colors.iter_mut() {
            let f = c.to_f32();
            *c = Rgb8::from_f32([f[0] * factor, f[1] * factor, f[2] * factor]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::{DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::layout::{CANVAS_HEIGHT, CANVAS_WIDTH};
    use canvas_proto::pattern::{BuiltOn, Frame as PatternFrame, Motion, Pattern};
    use canvas_proto::props::PropertySchema;
    use canvas_proto::scene::{zone_light_id, Arrangement};
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::geometry::LedSample;

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
            kind: None,
            copies: None,
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

    /// One frame: the scene hands the runner its groups, the runner renders them, and the effects
    /// go back to the scene as the engine's loop puts them back.
    fn frame(scene: &mut SceneState, runner: &mut SceneRunner, devices: &[DeviceInfo]) {
        frame_with(scene, runner, devices, &params());
    }

    fn frame_with(
        scene: &mut SceneState,
        runner: &mut SceneRunner,
        devices: &[DeviceInfo],
        params: &FrameParams,
    ) {
        let generation = runner.generation();
        let works = runner.take_works();
        let input = scene.take_input(devices, generation, works);
        runner.render(input, params);
        let mut done = std::mem::take(runner.done());
        let superseded = scene.put_back(&mut done);
        assert!(superseded.is_empty(), "one frame supersedes no effect");
    }

    // ------------------------------------------------------------ patterns (0.16.0, #339)

    const RED: Rgb8 = Rgb8::new(255, 0, 0);
    const GREEN: Rgb8 = Rgb8::new(0, 255, 0);
    const BLUE: Rgb8 = Rgb8::new(0, 0, 255);

    /// `n` colours, no two alike and none of them off.
    fn ramp(n: usize) -> Vec<Rgb8> {
        (0..n).map(|i| Rgb8::new((i % 256) as u8, (i / 256) as u8, 1)).collect()
    }

    /// The whole of zone 0 of device `id`, as a ring.
    fn ring(id: &str) -> Light {
        let mut light = light(id);
        light.arrangement = Arrangement::Ring;
        light
    }

    /// A pattern of one frame, built on `leds` LEDs of `arrangement`.
    fn pattern_on(leds: u32, arrangement: Arrangement, colors: Vec<Rgb8>) -> Pattern {
        pattern_with(vec![PatternFrame { colors, hold: 0, extra: Default::default() }], leds, arrangement)
    }

    fn pattern_with(frames: Vec<PatternFrame>, leds: u32, arrangement: Arrangement) -> Pattern {
        Pattern {
            id: "pattern:Test".into(),
            built_on: BuiltOn { led_count: leds, arrangement, grid: None },
            frames,
            motion: Motion::none(),
            extra: Default::default(),
        }
    }

    /// A pattern as an effect: the engine reads its colours and never calls `init` or `update`.
    struct PatternTestEffect {
        pattern: Pattern,
        inits: Arc<AtomicU32>,
        updates: Arc<AtomicU32>,
    }

    impl PatternTestEffect {
        fn new(pattern: Pattern) -> Self {
            Self { pattern, inits: Arc::new(AtomicU32::new(0)), updates: Arc::new(AtomicU32::new(0)) }
        }
    }

    impl Effect for PatternTestEffect {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: self.pattern.id.clone(),
                name: self.pattern.name().to_string(),
                description: "A test pattern.".into(),
                needs: InputNeeds::default(),
                properties: vec![],
                presets: vec![],
                thumbnail: None,
            }
        }

        fn init(&mut self, _props: &Props) {
            self.inits.fetch_add(1, Ordering::AcqRel);
        }

        fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {
            self.updates.fetch_add(1, Ordering::AcqRel);
        }

        fn as_pattern(&self) -> Option<&Pattern> {
            Some(&self.pattern)
        }
    }

    /// One group of `members`, playing `effect` at `brightness`.
    fn state_playing(
        members: &[&str],
        lights: Vec<Light>,
        effect: Option<Box<dyn Effect>>,
        brightness: u8,
    ) -> SceneState {
        let mut scene = state(members, lights);
        scene.groups[0].effect = effect;
        scene.groups[0].brightness = brightness;
        scene
    }

    fn take(runner: &mut SceneRunner, id: &str, leds: u32) -> Vec<Rgb8> {
        runner.take_frame(&DeviceId(id.into()), leds as usize).expect("a frame of the right length")
    }

    #[test]
    fn a_pattern_group_writes_its_colours_by_led() {
        let mut colors = vec![Rgb8::BLACK; 12];
        colors[0] = RED;
        let effect = PatternTestEffect::new(pattern_on(12, Arrangement::Ring, colors));
        let (inits, updates) = (effect.inits.clone(), effect.updates.clone());
        let mut scene =
            state_playing(&["d1#z0"], vec![ring("d1")], Some(Box::new(effect)), 100);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 12)];
        frame(&mut scene, &mut runner, &devices);
        let frame = take(&mut runner, "d1", 12);
        assert_eq!(frame[0], RED, "{frame:?}");
        assert!(frame[1..].iter().all(|c| *c == Rgb8::BLACK), "{frame:?}");
        assert_eq!(inits.load(Ordering::Acquire), 0, "a pattern is never initialised");
        assert_eq!(updates.load(Ordering::Acquire), 0, "a pattern never draws a canvas");
        let canvas = &runner.groups[0].canvas;
        assert!(
            canvas.pixels.iter().all(|p| *p == [0.0; 3]),
            "a pattern's canvas is never filled or sampled"
        );
    }

    #[test]
    fn a_pattern_plays_identically_on_every_light_of_its_kind() {
        let source = ramp(12);
        let effect = PatternTestEffect::new(pattern_on(12, Arrangement::Ring, source.clone()));
        let mut scene = state_playing(
            &["d1#z0", "d2#z0", "d3#z0"],
            vec![ring("d1"), ring("d2"), ring("d3")],
            Some(Box::new(effect)),
            100,
        );
        let mut runner = SceneRunner::new();
        let devices =
            vec![strip_device("d1", 12), strip_device("d2", 12), strip_device("d3", 12)];
        frame(&mut scene, &mut runner, &devices);
        for id in ["d1", "d2", "d3"] {
            let frame = take(&mut runner, id, 12);
            assert_eq!(frame, source, "{id} plays the pattern as built");
        }
    }

    #[test]
    fn off_leds_stay_black() {
        let mut colors = vec![Rgb8::BLACK; 16];
        colors[4] = GREEN;
        colors[9] = RED;
        let effect = PatternTestEffect::new(pattern_on(16, Arrangement::Strip, colors));
        let mut scene = state_playing(&["d1#z0"], vec![light("d1")], Some(Box::new(effect)), 100);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 16)];
        frame(&mut scene, &mut runner, &devices);
        let frame = take(&mut runner, "d1", 16);
        for (i, color) in frame.iter().enumerate() {
            let want = match i {
                4 => GREEN,
                9 => RED,
                _ => Rgb8::BLACK,
            };
            assert_eq!(*color, want, "LED {i}: {frame:?}");
        }
    }

    #[test]
    fn group_brightness_and_global_stages_apply_after_the_pattern() {
        let source = vec![Rgb8::new(200, 200, 200); 12];
        let effect = PatternTestEffect::new(pattern_on(12, Arrangement::Ring, source));
        let mut scene =
            state_playing(&["d1#z0"], vec![ring("d1")], Some(Box::new(effect)), 50);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 12)];
        let mut params = params();
        params.brightness = 0.5;
        params.warmth = [1.0, 0.5, 0.5];
        params.white_balance =
            Arc::new(HashMap::from([(DeviceId("d1".into()), [1.0, 1.0, 0.5])]));
        frame_with(&mut scene, &mut runner, &devices, &params);
        let frame = take(&mut runner, "d1", 12);
        // 200 at the group's 50% is 100; the global stages take it to 50 red, 25 green and
        // 200 x 0.125 = 25 (then the white balance's half) = 13 blue.
        for c in &frame {
            assert!((49..=51).contains(&c.r), "red at 50: {frame:?}");
            assert!((24..=26).contains(&c.g), "green at 25: {frame:?}");
            assert!((12..=14).contains(&c.b), "blue at 13: {frame:?}");
        }
    }

    #[test]
    fn a_stretched_light_gets_the_stretched_colours() {
        let source = ramp(12);
        let effect = PatternTestEffect::new(pattern_on(12, Arrangement::Ring, source.clone()));
        let mut scene =
            state_playing(&["d1#z0"], vec![ring("d1")], Some(Box::new(effect)), 100);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 24)];
        frame(&mut scene, &mut runner, &devices);
        let frame = take(&mut runner, "d1", 24);
        assert_eq!(frame.len(), 24);
        assert_eq!(frame[0], source[0], "the top LED stays at the top");
        assert_eq!(frame[1], source[1]);
        assert_eq!(frame[12], source[6], "half way round stays half way round");
        // Round ties up, so the LED just short of the wrap reads the pattern's last colour.
        assert_eq!(frame[22], source[11]);
    }

    #[test]
    fn a_two_frame_pattern_plays_its_first_frame() {
        let first = ramp(12);
        let mut second = vec![BLUE; 12];
        second[0] = GREEN;
        let pattern = pattern_with(
            vec![
                PatternFrame { colors: first.clone(), hold: 250, extra: Default::default() },
                PatternFrame { colors: second, hold: 250, extra: Default::default() },
            ],
            12,
            Arrangement::Strip,
        );
        let effect = PatternTestEffect::new(pattern);
        let mut scene = state_playing(&["d1#z0"], vec![light("d1")], Some(Box::new(effect)), 100);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 12)];
        for _ in 0..5 {
            frame(&mut scene, &mut runner, &devices);
            let frame = take(&mut runner, "d1", 12);
            assert_eq!(frame, first, "0.16.0 plays frame 0");
        }
    }

    #[test]
    fn a_reversed_light_shows_the_pattern_the_other_way_round() {
        let mut colors = vec![Rgb8::BLACK; 12];
        colors[0] = RED;
        let effect = PatternTestEffect::new(pattern_on(12, Arrangement::Strip, colors));
        let mut reversed = light("d2");
        reversed.reversed = true;
        let mut scene = state_playing(
            &["d1#z0", "d2#z0"],
            vec![light("d1"), reversed],
            Some(Box::new(effect)),
            100,
        );
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 12), strip_device("d2", 12)];
        frame(&mut scene, &mut runner, &devices);
        let plain = take(&mut runner, "d1", 12);
        let flipped = take(&mut runner, "d2", 12);
        assert_eq!(plain[0], RED, "the plain light shows LED 0 at the start: {plain:?}");
        assert_eq!(flipped[11], RED, "the reversed light shows it at the end: {flipped:?}");
        assert_eq!(flipped[0], Rgb8::BLACK);
        assert!(plain[1..].iter().all(|c| *c == Rgb8::BLACK));
        assert!(flipped[..11].iter().all(|c| *c == Rgb8::BLACK));
    }

    /// A frame built for the old LED count of a zone is never handed to a sink that expects the
    /// new one (Review Focus 2), and the frame of the right length is.
    #[test]
    fn a_frame_of_the_old_length_is_dropped_not_sent() {
        let mut scene = state(&["d1#z0"], vec![light("d1")]);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 12)];
        frame(&mut scene, &mut runner, &devices);
        let id = DeviceId("d1".into());
        // The zone grew to 20 LEDs after the frame was built: its sink now expects 20.
        assert_eq!(runner.take_frame(&id, 20), None, "a 12-LED frame for a 20-LED sink");

        frame(&mut scene, &mut runner, &devices);
        let frame = runner.take_frame(&id, 12).expect("a frame of the right length");
        assert_eq!(frame.len(), 12);
    }

    /// The plan is rebuilt when the scene is replaced with the zone's new size: the next frame
    /// has the new length.
    #[test]
    fn a_new_scene_rebuilds_the_frame_to_the_new_length() {
        let mut runner = SceneRunner::new();
        let mut scene = state(&["d1#z0"], vec![light("d1")]);
        frame(&mut scene, &mut runner, &[strip_device("d1", 12)]);
        let mut scene = state(&["d1#z0"], vec![light("d1")]);
        scene.generation = 2;
        frame(&mut scene, &mut runner, &[strip_device("d1", 20)]);
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

    // ------------------------------------------------ strip sampling (#372)

    /// Purple Rain, the scene that showed only the microphone moving: Density 252, Speed
    /// 247, drops #552b7b over #1b003c.
    fn purple_rain_props() -> Props {
        use canvas_proto::props::PropValue;
        Props::from([
            ("drop_color".to_string(), PropValue::Color(Rgb8::new(0x55, 0x2b, 0x7b))),
            ("background".to_string(), PropValue::Color(Rgb8::new(0x1b, 0x00, 0x3c))),
            ("density".to_string(), PropValue::Number(252.0)),
            ("speed".to_string(), PropValue::Number(247.0)),
        ])
    }

    /// Frames run before the ranges are counted: 4 s at 30 fps, past Rain's fill-in.
    const RAIN_WARMUP_FRAMES: usize = 120;

    /// A group of `ids` (each a strip), playing seeded Purple Rain in `mode`.
    fn rain_scene(ids: &[&str], mode: GroupMode) -> SceneState {
        let members: Vec<String> = ids.iter().map(|id| format!("{id}#z0")).collect();
        let member_refs: Vec<&str> = members.iter().map(String::as_str).collect();
        let mut scene = state_playing(
            &member_refs,
            ids.iter().map(|id| light(id)).collect(),
            Some(Box::new(canvas_effects::rain::Rain::with_seed(372))),
            100,
        );
        scene.groups[0].mode = mode;
        scene.groups[0].pending_props = Some(purple_rain_props());
        scene
    }

    /// Runs `frames` frames at 30 fps and returns, for each LED of each device in `ids`
    /// (id and LED count), the per channel (min, max) it showed.
    fn run_ranges(
        scene: &mut SceneState,
        runner: &mut SceneRunner,
        devices: &[DeviceInfo],
        ids: &[(&str, u32)],
        frames: usize,
    ) -> Vec<Vec<[(u8, u8); 3]>> {
        let mut ranges: Vec<Vec<[(u8, u8); 3]>> =
            ids.iter().map(|(_, leds)| vec![[(255u8, 0u8); 3]; *leds as usize]).collect();
        // Rain starts with its drops above the canvas and needs about three seconds to fill it;
        // those frames show the fill-in, not the steady rain, so they are run and not counted.
        for _ in 0..RAIN_WARMUP_FRAMES {
            frame(scene, runner, devices);
        }
        for _ in 0..frames {
            frame(scene, runner, devices);
            for ((id, leds), device) in ids.iter().zip(ranges.iter_mut()) {
                for (range, c) in device.iter_mut().zip(take(runner, id, *leds)) {
                    for (slot, v) in range.iter_mut().zip([c.r, c.g, c.b]) {
                        slot.0 = slot.0.min(v);
                        slot.1 = slot.1.max(v);
                    }
                }
            }
        }
        ranges
    }

    /// The largest per-channel range of each LED.
    fn widest(ranges: &[[(u8, u8); 3]]) -> Vec<u8> {
        ranges
            .iter()
            .map(|r| r.iter().map(|(lo, hi)| hi.saturating_sub(*lo)).max().unwrap_or(0))
            .collect()
    }

    /// Every LED of an 8-LED strip in "each its own" must move with Rain's thin lines. Before
    /// the fix a strip LED averaged a full-height column holding about 19 streaks and its
    /// range was about 6 of 255.
    #[test]
    fn rain_animates_every_led_of_a_strip_in_each_its_own() {
        let mut scene = rain_scene(&["d1"], GroupMode::EachItsOwn);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 8)];
        let ranges = run_ranges(&mut scene, &mut runner, &devices, &[("d1", 8)], 120).remove(0);
        for (led, r) in ranges.iter().enumerate() {
            eprintln!(
                "#372 each-its-own LED {led}: r {}..{} g {}..{} b {}..{}",
                r[0].0, r[0].1, r[1].0, r[1].1, r[2].0, r[2].1
            );
        }
        let widest = widest(&ranges);
        eprintln!("#372 each-its-own widest channel range per LED: {widest:?}");
        for (led, w) in widest.iter().enumerate() {
            assert!(*w >= 30, "LED {led} moved only {w} of 255 in 120 frames: {widest:?}");
        }
    }

    /// Flow across with two strips: each strip is half the canvas and still animates.
    #[test]
    fn rain_still_animates_two_strips_in_flow_across() {
        let mut scene = rain_scene(&["d1", "d2"], GroupMode::FlowAcross);
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 8), strip_device("d2", 8)];
        let ranges = run_ranges(&mut scene, &mut runner, &devices, &[("d1", 8), ("d2", 8)], 120);
        let all: Vec<u8> = ranges.iter().flat_map(|r| widest(r)).collect();
        eprintln!("#372 flow-across widest channel range per LED: {all:?}");
        for (led, w) in all.iter().enumerate() {
            assert!(*w >= 30, "LED {led} moved only {w} of 255 in 120 frames: {all:?}");
        }
    }

    /// An effect that paints a left-to-right ramp, the same on every row.
    struct HorizontalRamp;

    impl Effect for HorizontalRamp {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "test-ramp".into(),
                name: "Ramp".into(),
                description: "A horizontal gradient.".into(),
                needs: InputNeeds::default(),
                properties: vec![],
                presets: vec![],
                thumbnail: None,
            }
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
            for y in 0..canvas.height {
                for x in 0..canvas.width {
                    let t = x as f32 / (canvas.width - 1) as f32;
                    canvas.set(x, y, [t, 1.0 - t, 0.25 + 0.5 * t]);
                }
            }
        }
    }

    /// A gradient that runs top to bottom, the same on every column.
    struct VerticalRamp;

    impl Effect for VerticalRamp {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "test-vertical-ramp".into(),
                name: "Vertical Ramp".into(),
                description: "A vertical gradient.".into(),
                needs: InputNeeds::default(),
                properties: vec![],
                presets: vec![],
                thumbnail: None,
            }
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
            for y in 0..canvas.height {
                let t = y as f32 / (canvas.height - 1) as f32;
                for x in 0..canvas.width {
                    canvas.set(x, y, [t, 1.0 - t, 0.25 + 0.5 * t]);
                }
            }
        }
    }

    /// One flat colour over the whole canvas.
    struct FlatColour;

    impl Effect for FlatColour {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "test-flat-colour".into(),
                name: "Flat Colour".into(),
                description: "One colour everywhere.".into(),
                needs: InputNeeds::default(),
                properties: vec![],
                presets: vec![],
                thumbnail: None,
            }
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
            canvas.fill([0.2, 0.4, 0.6]);
        }
    }

    /// The value of the ramp at a position on the canvas, in 8-bit.
    ///
    /// The ramp paints pixel `p` with the value of `t = p / (length - 1)`, and a pixel's
    /// centre is at `p + 0.5`, so the gradient's value at a position `x` is the one of
    /// `t = (x - 0.5) / (length - 1)`.
    fn ramp_value_at(t: f32) -> [i32; 3] {
        [
            (t * 255.0 + 0.5).floor() as i32,
            ((1.0 - t) * 255.0 + 0.5).floor() as i32,
            ((0.25 + 0.5 * t) * 255.0 + 0.5).floor() as i32,
        ]
    }

    /// A horizontal linear gradient gives every LED the gradient's value at the LED's own
    /// centre, within 2/255 (#372).
    ///
    /// The cell is a small square at the LED, so the LED reads where it stands. The old
    /// full-height band gave the mean of the LED's share of the picture, which is a
    /// different number: that is the truth the cap changed, and this is what it is now.
    #[test]
    fn a_horizontal_gradient_gives_each_led_the_gradients_value_at_its_centre() {
        for mode in [GroupMode::EachItsOwn, GroupMode::FlowAcross] {
            let mut scene =
                state_playing(&["d1#z0"], vec![light("d1")], Some(Box::new(HorizontalRamp)), 100);
            scene.groups[0].mode = mode;
            let mut runner = SceneRunner::new();
            let devices = vec![strip_device("d1", 12)];
            frame(&mut scene, &mut runner, &devices);
            let (centres, width) = {
                let group = &runner.groups[0];
                let centres =
                    group.plan.lights[0].leds.iter().map(|led| led.center_x).collect::<Vec<f32>>();
                (centres, group.canvas.width as f32)
            };
            let now = take(&mut runner, "d1", 12);
            for (led, (centre, colour)) in centres.iter().zip(&now).enumerate() {
                let want = ramp_value_at((centre - 0.5) / (width - 1.0));
                for (got, expect) in [colour.r, colour.g, colour.b].into_iter().zip(want) {
                    assert!(
                        i32::from(got).abs_diff(expect) <= 2,
                        "{mode:?} LED {led}: {colour:?} against {expect} at x {centre}"
                    );
                }
            }
        }
    }

    /// A flat colour reads the same on every LED, whatever cell it has (#372).
    #[test]
    fn a_flat_colour_reads_the_same_on_every_led() {
        for mode in [GroupMode::EachItsOwn, GroupMode::FlowAcross] {
            let mut scene =
                state_playing(&["d1#z0"], vec![light("d1")], Some(Box::new(FlatColour)), 100);
            scene.groups[0].mode = mode;
            let mut runner = SceneRunner::new();
            let devices = vec![strip_device("d1", 8)];
            frame(&mut scene, &mut runner, &devices);
            let now = take(&mut runner, "d1", 8);
            let want = Rgb8::from_f32([0.2, 0.4, 0.6]);
            for (led, colour) in now.iter().enumerate() {
                assert_eq!(*colour, want, "{mode:?} LED {led}: {now:?}");
            }
        }
    }

    /// A vertical linear gradient gives every LED the value at mid-height, within 2/255:
    /// a strip's LEDs all stand at mid-height (#372).
    #[test]
    fn a_vertical_gradient_gives_every_led_the_mid_height_value() {
        for mode in [GroupMode::EachItsOwn, GroupMode::FlowAcross] {
            let mut scene =
                state_playing(&["d1#z0"], vec![light("d1")], Some(Box::new(VerticalRamp)), 100);
            scene.groups[0].mode = mode;
            let mut runner = SceneRunner::new();
            let devices = vec![strip_device("d1", 12)];
            frame(&mut scene, &mut runner, &devices);
            let (centres, height) = {
                let group = &runner.groups[0];
                let centres =
                    group.plan.lights[0].leds.iter().map(|led| led.center_y).collect::<Vec<f32>>();
                (centres, group.canvas.height as f32)
            };
            let now = take(&mut runner, "d1", 12);
            for (led, (centre, colour)) in centres.iter().zip(&now).enumerate() {
                let want = ramp_value_at((centre - 0.5) / (height - 1.0));
                for (got, expect) in [colour.r, colour.g, colour.b].into_iter().zip(want) {
                    assert!(
                        i32::from(got).abs_diff(expect) <= 2,
                        "{mode:?} LED {led}: {colour:?} against {expect} at y {centre}"
                    );
                }
            }
        }
    }

    /// A 240-LED strip's cells are narrower than the cap already, so the cap changes
    /// nothing in width: the cell is still the LED's share of the slot. What changes is
    /// its height: the cell is the square of that share, not the whole height of the slot
    /// as the old band was (#372).
    #[test]
    fn a_240_led_strip_keeps_its_cell_width_and_loses_the_full_height() {
        let share = CANVAS_WIDTH as f32 / 240.0;
        for mode in [GroupMode::EachItsOwn, GroupMode::FlowAcross] {
            let plan = build_group_plan(
                &["d1#z0".to_string()],
                mode,
                &[light("d1")],
                &[strip_device("d1", 240)],
            );
            for led in &plan.lights[0].leds {
                let [x0, y0, x1, y1] = led.bounds;
                assert!(
                    (x1 - x0 - share).abs() < 0.05,
                    "{mode:?}: the cell is {} wide, not {share}",
                    x1 - x0
                );
                assert!(
                    (y1 - y0 - share).abs() < 0.05,
                    "{mode:?}: the cell is {} tall, not {share}",
                    y1 - y0
                );
                assert!(
                    y1 - y0 < CANVAS_HEIGHT as f32,
                    "{mode:?}: the cell is the whole height of the slot again"
                );
            }
        }
    }

    /// A 240-LED strip has cells narrower than one logical unit: every cell is still a real
    /// area inside the canvas, with finite numbers.
    #[test]
    fn a_240_led_strip_has_no_nan_no_stray_cell_and_no_empty_cell() {
        for mode in [GroupMode::EachItsOwn, GroupMode::FlowAcross] {
            for arrangement in [Arrangement::Strip, Arrangement::Ring] {
                let mut l = light("d1");
                l.arrangement = arrangement;
                let plan = build_group_plan(
                    &[l.id.clone()],
                    mode,
                    &[l],
                    &[strip_device("d1", 240)],
                );
                let leds = &plan.lights[0].leds;
                assert_eq!(leds.len(), 240);
                for (i, led) in leds.iter().enumerate() {
                    let [x0, y0, x1, y1] = led.bounds;
                    let what = format!("{mode:?} {arrangement:?} LED {i}: {led:?}");
                    assert!(
                        [led.center_x, led.center_y, x0, y0, x1, y1].iter().all(|v| v.is_finite()),
                        "{what}"
                    );
                    assert!(x0 >= 0.0 && y0 >= 0.0, "{what}");
                    assert!(x1 <= 320.0 && y1 <= 200.0, "{what}");
                    assert!(x1 - x0 > 0.0 && y1 - y0 > 0.0, "zero area: {what}");
                }
            }
        }
    }

    // ------------------------------- the candidate rules (#372, the review's blocker)

    /// How a strip's LED reads its column of the picture.
    ///
    /// The three candidates the review named, in the order of preference: the plain mean
    /// over the column, the mean of the column's brightest quarter of rows, and the
    /// column's brightest row.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Rule {
        /// The mean of every pixel in the cell.
        Mean,
        /// The mean of the brightest quarter of the cell's rows.
        BrightestQuarter,
        /// The mean colour of the cell's brightest row.
        BrightestRow,
    }

    impl Rule {
        /// The name the tables print.
        fn name(self) -> &'static str {
            match self {
                Rule::Mean => "mean",
                Rule::BrightestQuarter => "brightest quarter",
                Rule::BrightestRow => "brightest row",
            }
        }
    }

    /// The candidates, in the order of preference: the plain mean first, its widths
    /// swept, then the brightest quarter, then the brightest row.
    fn candidates() -> Vec<(Rule, f32)> {
        let mut out = Vec::new();
        for rule in [Rule::Mean, Rule::BrightestQuarter, Rule::BrightestRow] {
            for width in [7.0f32, 5.0, 3.0, 2.0] {
                out.push((rule, width));
            }
        }
        out
    }

    /// The shape of a strip's cell.
    ///
    /// `Column` is a full-height column of a given width, the shape every candidate reads.
    /// `Today` is the cell the branch ships now: a small square at mid-height.
    /// `Bottom` is today's square cell anchored at the bottom of the picture, where Fire
    /// burns and the audio bars grow. It is a diagnostic, not a candidate.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Shape {
        /// A full-height column `width` units wide.
        Column(f32),
        /// Today's square cell at mid-height, at most `STRIP_CELL_MAX` units.
        Today,
        /// Today's square cell at the bottom of the picture.
        Bottom,
    }

    impl Shape {
        /// The name the tables print.
        fn name(self) -> String {
            match self {
                Shape::Column(width) => format!("column {width}"),
                Shape::Today => "today's square cell".to_string(),
                Shape::Bottom => "today's square cell at the bottom".to_string(),
            }
        }

        /// The cells of a strip of `n` LEDs in a slot.
        fn cells(self, slot_x: f32, slot_w: f32, n: usize) -> Vec<LedSample> {
            match self {
                Shape::Column(width) => column_cells(width, slot_x, slot_w, n),
                Shape::Today | Shape::Bottom => {
                    let share = slot_w / n as f32;
                    let side = share.min(CANVAS_HEIGHT as f32).min(7.0);
                    let y = match self {
                        Shape::Today => CANVAS_HEIGHT as f32 / 2.0,
                        Shape::Bottom => CANVAS_HEIGHT as f32 - side / 2.0,
                        Shape::Column(_) => unreachable!(),
                    };
                    let scale = SCENE_RENDER_SCALE;
                    let (y0, y1) = clamp_span(y - side / 2.0, y + side / 2.0, CANVAS_HEIGHT as f32);
                    (0..n)
                        .map(|j| {
                            let x = slot_x + share * (j as f32 + 0.5);
                            LedSample {
                                center_x: x * scale,
                                center_y: y * scale,
                                bounds: [
                                    (x - side / 2.0) * scale,
                                    y0 * scale,
                                    (x + side / 2.0) * scale,
                                    y1 * scale,
                                ],
                                sampled: true,
                            }
                        })
                        .collect()
                }
            }
        }
    }

    /// A strip's cells under a candidate: a column `width` units wide and the full
    /// height of the slot, centred on the LED.
    ///
    /// This is the old full-height band with a narrow column, which is candidate (a), and
    /// the cell candidates (b) and (c) read the same way.
    fn column_cells(width: f32, slot_x: f32, slot_w: f32, n: usize) -> Vec<LedSample> {
        let share = slot_w / n as f32;
        let scale = SCENE_RENDER_SCALE;
        (0..n)
            .map(|j| {
                let x = slot_x + share * (j as f32 + 0.5);
                let (x0, x1) = clamp_span(x - width / 2.0, x + width / 2.0, CANVAS_WIDTH as f32);
                LedSample {
                    center_x: x * scale,
                    center_y: CANVAS_HEIGHT as f32 / 2.0 * scale,
                    bounds: [x0 * scale, 0.0, x1 * scale, CANVAS_HEIGHT as f32 * scale],
                    sampled: true,
                }
            })
            .collect()
    }

    /// Clamps a span to the canvas and keeps it at least one logical unit wide.
    fn clamp_span(x0: f32, x1: f32, width: f32) -> (f32, f32) {
        let mut x0 = x0.clamp(0.0, width);
        let mut x1 = x1.clamp(0.0, width);
        if x1 - x0 < 1.0 {
            x1 = (x0 + 1.0).min(width);
            x0 = x1 - 1.0;
        }
        (x0, x1)
    }

    /// Reads one LED's cell out of the canvas under a rule.
    ///
    /// The pixel range is the one `sampling::sample_area` averages over: the pixels whose
    /// centres fall in the cell, one axis at a time, so a cell thinner than a pixel still
    /// reads the pixel it is over in the other axis.
    fn read_cell(
        canvas: &[[f32; 3]],
        width: u32,
        height: u32,
        led: &LedSample,
        rule: Rule,
    ) -> Rgb8 {
        let [bx0, by0, bx1, by1] = led.bounds;
        let max_x = width - 1;
        let max_y = height - 1;
        let x0 = ((bx0 - 0.5).ceil() as i64).clamp(0, i64::from(max_x)) as u32;
        let y0 = ((by0 - 0.5).ceil() as i64).clamp(0, i64::from(max_y)) as u32;
        let mut x1 = ((bx1 - 0.5).floor() as i64).clamp(0, i64::from(max_x)) as u32;
        let mut y1 = ((by1 - 0.5).floor() as i64).clamp(0, i64::from(max_y)) as u32;
        if x1 < x0 {
            x1 = x0;
        }
        if y1 < y0 {
            y1 = y0;
        }
        // Each row's mean colour across the cell's width.
        let mut rows: Vec<(f64, [f64; 3])> = Vec::new();
        for y in y0..=y1 {
            let mut sum = [0.0f64; 3];
            for x in x0..=x1 {
                let p = canvas[(y * width + x) as usize];
                sum[0] += f64::from(p[0]);
                sum[1] += f64::from(p[1]);
                sum[2] += f64::from(p[2]);
            }
            let n = f64::from(x1 - x0 + 1);
            let mean = [sum[0] / n, sum[1] / n, sum[2] / n];
            // Rec. 709 luminance of the row's mean colour.
            let lum = 0.2126 * mean[0] + 0.7152 * mean[1] + 0.0722 * mean[2];
            rows.push((lum, mean));
        }
        let keep = match rule {
            Rule::Mean => rows.len(),
            Rule::BrightestQuarter => rows.len().div_ceil(4),
            Rule::BrightestRow => 1,
        };
        // The brightest rows first, ties by row order: the earlier row comes first.
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by(|a, b| {
            rows[*b]
                .0
                .partial_cmp(&rows[*a].0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(b))
        });
        let mut sum = [0.0f64; 3];
        for index in order.iter().take(keep) {
            for channel in 0..3 {
                sum[channel] += rows[*index].1[channel];
            }
        }
        let n = keep.max(1) as f64;
        Rgb8::from_f32([
            (sum[0] / n) as f32,
            (sum[1] / n) as f32,
            (sum[2] / n) as f32,
        ])
    }

    /// Reads a strip's cells out of the canvas under a rule.
    fn sample_cells(
        canvas: &[[f32; 3]],
        width: u32,
        height: u32,
        cells: &[LedSample],
        rule: Rule,
    ) -> Vec<Rgb8> {
        cells.iter().map(|led| read_cell(canvas, width, height, led, rule)).collect()
    }

    /// Runs the scene and returns, for each strip, the per-channel (min, max) every LED
    /// showed under `rule` and the given cells.
    ///
    /// The cells are the candidate's, not the plan's: the runner samples the plan, so the
    /// candidate is measured on the same canvas the runner drew.
    fn run_ranges_over(
        scene: &mut SceneState,
        runner: &mut SceneRunner,
        devices: &[DeviceInfo],
        cells: &[Vec<LedSample>],
        rule: Rule,
        frames: usize,
    ) -> Vec<Vec<[(u8, u8); 3]>> {
        let mut ranges: Vec<Vec<[(u8, u8); 3]>> =
            cells.iter().map(|c| vec![[(255u8, 0u8); 3]; c.len()]).collect();
        for index in 0..RAIN_WARMUP_FRAMES + frames {
            frame(scene, runner, devices);
            if index < RAIN_WARMUP_FRAMES {
                continue;
            }
            let group = &runner.groups[0];
            for (slot, led_cells) in cells.iter().enumerate() {
                let colours = sample_cells(
                    &group.canvas.pixels,
                    group.canvas.width,
                    group.canvas.height,
                    led_cells,
                    rule,
                );
                for (range, colour) in ranges[slot].iter_mut().zip(colours) {
                    for (entry, v) in range.iter_mut().zip([colour.r, colour.g, colour.b]) {
                        entry.0 = entry.0.min(v);
                        entry.1 = entry.1.max(v);
                    }
                }
            }
        }
        ranges
    }

    /// Measures an effect on a strip with the candidate's cells: the stats of every LED.
    fn measure_cells(
        effect: Box<dyn Effect>,
        props: Option<Props>,
        magnitude: Option<f32>,
        cells: &[Vec<LedSample>],
        rule: Rule,
    ) -> Vec<Stats> {
        let mut scene = state_playing(&["d1#z0"], vec![light("d1")], Some(effect), 100);
        scene.groups[0].mode = GroupMode::EachItsOwn;
        scene.groups[0].needs = InputNeeds { screen: false, audio: magnitude.is_some() };
        if let Some(props) = props {
            scene.groups[0].pending_props = Some(props);
        }
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 8)];
        let mut stats: Vec<Stats> = cells.iter().map(|c| Stats::new(c.len())).collect();
        for index in 0..RAIN_WARMUP_FRAMES + 120 {
            let mut params = params();
            if let Some(magnitude) = magnitude {
                params.input_source = Some(Arc::new(LoudBands { magnitude }));
            }
            frame_with(&mut scene, &mut runner, &devices, &params);
            if index < RAIN_WARMUP_FRAMES {
                continue;
            }
            let group = &runner.groups[0];
            for (slot, led_cells) in cells.iter().enumerate() {
                let colours = sample_cells(
                    &group.canvas.pixels,
                    group.canvas.width,
                    group.canvas.height,
                    led_cells,
                    rule,
                );
                stats[slot].add(&colours);
            }
        }
        stats
    }

    /// A spectrum with a run of loud bands at a given magnitude.
    ///
    /// Bands 8 to 24 of 32 are loud. On the scene's 80 pixel canvas a band is two pixels
    /// wide, so the loud run covers the render pixels 16 to 50: the LEDs at 25, 35 and 45
    /// of an 8-LED strip stand under it.
    struct LoudBands {
        magnitude: f32,
    }

    impl InputSource for LoudBands {
        fn screen(&self) -> Option<Arc<ScreenFrame>> {
            None
        }

        fn audio(&self) -> Option<Arc<AudioSpectrum>> {
            let centers: Vec<f32> = (0..32).map(|i| 60.0 * 2f32.powf(i as f32 / 4.0)).collect();
            let magnitudes: Vec<f32> =
                (0..32).map(|i| if (8..24).contains(&i) { self.magnitude } else { 0.0 }).collect();
            Some(Arc::new(AudioSpectrum { centers_hz: centers, magnitudes }))
        }
    }

    /// The render pixels the loud bands cover.
    const LOUD_X0: f32 = 16.0;
    /// The render pixels the loud bands cover.
    const LOUD_X1: f32 = 50.0;

    /// What one candidate measured: the numbers of the four checks.
    struct Verdict {
        /// Rain's widest channel range per LED, each its own.
        rain_own: Vec<u8>,
        /// Rain's widest channel range per LED, across two strips.
        rain_flow: Vec<u8>,
        /// Fire's mean level on the strip, and the old full-height band's.
        fire: f64,
        fire_old: f64,
        /// Audio Spectrum's mean level under the loud bands, at 0.2, 0.5 and 1.0.
        audio: [f64; 3],
        /// The flat colour the strip read.
        flat: Rgb8,
        /// The biggest error a horizontal gradient showed, of 255.
        gradient: u32,
    }

    impl Verdict {
        /// Whether the candidate passes all four checks.
        fn passes(&self) -> bool {
            self.r1() && self.r2() && self.r3() && self.r4()
        }

        /// R1: every LED of a strip moves with Rain's thin lines.
        fn r1(&self) -> bool {
            self.rain_own.iter().chain(self.rain_flow.iter()).all(|w| *w >= 30)
        }

        /// R2: Fire is no darker on a strip than the old full-height band gave it.
        fn r2(&self) -> bool {
            self.fire >= self.fire_old
        }

        /// R3: Audio Spectrum is lit at every magnitude and rises with it.
        fn r3(&self) -> bool {
            self.audio[0] > 0.0
                && self.audio[1] > 0.0
                && self.audio[2] > 0.0
                && self.audio[0] < self.audio[1]
                && self.audio[1] < self.audio[2]
        }

        /// R4: a flat colour reads exactly, a gradient reads the LED's own position.
        fn r4(&self) -> bool {
            self.flat == Rgb8::from_f32([0.2, 0.4, 0.6]) && self.gradient <= 3
        }
    }

    /// Measures one shape under one rule against the four checks.
    fn measure_shape(rule: Rule, shape: Shape) -> Verdict {
        // R1: Rain, each its own and across two strips.
        let mut rain_own = Vec::new();
        let mut rain_flow = Vec::new();
        for (own, slots) in [
            (true, vec![(0.0f32, 320.0f32)]),
            (false, vec![(0.0f32, 160.0f32), (160.0f32, 160.0f32)]),
        ] {
            let mode = if own { GroupMode::EachItsOwn } else { GroupMode::FlowAcross };
            let mut scene = rain_scene(&["d1", "d2"], mode);
            let mut runner = SceneRunner::new();
            let devices = vec![strip_device("d1", 8), strip_device("d2", 8)];
            let cells: Vec<Vec<LedSample>> =
                slots.iter().map(|(x, w)| shape.cells(*x, *w, 8)).collect();
            let ranges = run_ranges_over(&mut scene, &mut runner, &devices, &cells, rule, 120);
            let all: Vec<u8> = ranges.iter().flat_map(|r| widest(r)).collect();
            if own {
                rain_own = all;
            } else {
                rain_flow = all;
            }
        }

        let cells = shape.cells(0.0, 320.0, 8);

        // R2: Fire's level on the strip, against the old full-height band.
        let fire_stats =
            measure_cells(Box::new(canvas_effects::fire::Fire::with_seed(372)), None, None, std::slice::from_ref(&cells), rule);
        let fire = fire_stats[0].levels().iter().sum::<f64>() / 8.0;
        let old_stats = measure_cells(
            Box::new(canvas_effects::fire::Fire::with_seed(372)),
            None,
            None,
            &[old_band_cells(8)],
            Rule::Mean,
        );
        let fire_old = old_stats[0].levels().iter().sum::<f64>() / 8.0;

        // R3: Audio Spectrum at three magnitudes, on the LEDs under the loud bands.
        let mut audio = [0.0f64; 3];
        for (index, magnitude) in [0.2f32, 0.5, 1.0].iter().enumerate() {
            let stats = measure_cells(
                Box::new(canvas_effects::audio_spectrum::AudioSpectrum::new()),
                None,
                Some(*magnitude),
                std::slice::from_ref(&cells),
                rule,
            );
            let levels = stats[0].levels();
            let under: Vec<f64> = cells
                .iter()
                .zip(&levels)
                .filter(|(led, _)| led.center_x >= LOUD_X0 && led.center_x < LOUD_X1)
                .map(|(_, level)| *level)
                .collect();
            audio[index] = under.iter().sum::<f64>() / under.len().max(1) as f64;
        }

        // R4: a flat colour reads exactly, a horizontal gradient within 3 of 255.
        let mut scene =
            state_playing(&["d1#z0"], vec![light("d1")], Some(Box::new(FlatColour)), 100);
        scene.groups[0].mode = GroupMode::EachItsOwn;
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 8)];
        frame(&mut scene, &mut runner, &devices);
        let flat = {
            let group = &runner.groups[0];
            sample_cells(
                &group.canvas.pixels,
                group.canvas.width,
                group.canvas.height,
                &cells,
                rule,
            )[0]
        };

        let mut scene =
            state_playing(&["d1#z0"], vec![light("d1")], Some(Box::new(HorizontalRamp)), 100);
        scene.groups[0].mode = GroupMode::EachItsOwn;
        let mut runner = SceneRunner::new();
        frame(&mut scene, &mut runner, &devices);
        let gradient = {
            let group = &runner.groups[0];
            let now = sample_cells(
                &group.canvas.pixels,
                group.canvas.width,
                group.canvas.height,
                &cells,
                rule,
            );
            let mut worst = 0u32;
            for (led, colour) in cells.iter().zip(&now) {
                let centre = led.center_x / SCENE_RENDER_SCALE;
                let want = ramp_value_at((centre - 0.5) / (CANVAS_WIDTH as f32 - 1.0));
                for (got, expect) in [colour.r, colour.g, colour.b].into_iter().zip(want) {
                    worst = worst.max(i32::from(got).abs_diff(expect));
                }
            }
            worst
        };

        Verdict { rain_own, rain_flow, fire, fire_old, audio, flat, gradient }
    }

    // ------------------------------------------- what the cap costs each effect (#372)

    /// The cells of a strip for a given cell side, in the scaled canvas units.
    fn strip_cells(side: f32, slot_x: f32, slot_w: f32, n: usize) -> Vec<LedSample> {
        let share = slot_w / n as f32;
        let scale = SCENE_RENDER_SCALE;
        let mid = CANVAS_HEIGHT as f32 / 2.0;
        (0..n)
            .map(|j| {
                let x = slot_x + share * (j as f32 + 0.5);
                LedSample {
                    center_x: x * scale,
                    center_y: mid * scale,
                    bounds: [
                        (x - side / 2.0) * scale,
                        (mid - side / 2.0) * scale,
                        (x + side / 2.0) * scale,
                        (mid + side / 2.0) * scale,
                    ],
                    sampled: true,
                }
            })
            .collect()
    }

    /// Sweeps the cell side and prints the widest channel range every LED of an 8-LED
    /// strip shows under Rain, for the two group modes.
    ///
    /// Then it sweeps the candidate rules and prints R1 for each of them: the range every
    /// LED shows under Rain, each its own and across two strips. R1 wants 30 of 255 on
    /// every LED.
    #[test]
    #[ignore = "#372: a sweep of the cap, run by hand"]
    fn sweep_the_cap_for_rain() {
        for (mode, slots) in [
            (GroupMode::EachItsOwn, vec![(0.0f32, 320.0f32)]),
            (GroupMode::FlowAcross, vec![(0.0f32, 160.0f32), (160.0f32, 160.0f32)]),
        ] {
            let mut scene = rain_scene(&["d1", "d2"], mode);
            let mut runner = SceneRunner::new();
            let devices = vec![strip_device("d1", 8), strip_device("d2", 8)];
            let sides = [12.0f32, 10.0, 9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0];
            let mut stats: Vec<Vec<Stats>> = sides
                .iter()
                .map(|_| slots.iter().map(|_| Stats::new(8)).collect())
                .collect();
            for index in 0..RAIN_WARMUP_FRAMES + 120 {
                frame(&mut scene, &mut runner, &devices);
                if index < RAIN_WARMUP_FRAMES {
                    continue;
                }
                let group = &runner.groups[0];
                for (side_index, side) in sides.iter().enumerate() {
                    for (slot_index, (slot_x, slot_w)) in slots.iter().enumerate() {
                        let cells = strip_cells(*side, *slot_x, *slot_w, 8);
                        let mut colours = Vec::new();
                        sampling::sample_leds(
                            &group.canvas.pixels,
                            group.canvas.width,
                            group.canvas.height,
                            &cells,
                            SCENE_SAMPLING,
                            &mut colours,
                        );
                        let entry = &mut stats[side_index][slot_index];
                        entry.add(&colours);
                    }
                }
            }
            for (side_index, side) in sides.iter().enumerate() {
                let mut all: Vec<u8> = Vec::new();
                for entry in &stats[side_index] {
                    all.extend(entry.widest());
                }
                println!(
                    "#372 sweep {mode:?} side {side}: min {} max {} all {all:?}",
                    all.iter().min().unwrap_or(&0),
                    all.iter().max().unwrap_or(&0)
                );
            }
        }

        // R1 for every candidate rule, and the cell the branch ships now as the
        // reference the candidates have to beat.
        println!("#372 R1: the widest channel range per LED under Rain, of 255");
        let today = measure_shape(Rule::Mean, Shape::Today);
        println!(
            "#372 R1 today's square cell: own {own:?} | flow {flow:?}",
            own = today.rain_own,
            flow = today.rain_flow,
        );
        for (rule, width) in candidates() {
            let verdict = measure_shape(rule, Shape::Column(width));
            println!(
                "#372 R1 {} width {width}: own {own:?} | flow {flow:?}",
                rule.name(),
                own = verdict.rain_own,
                flow = verdict.rain_flow,
            );
        }
    }

    /// What one sampling of one effect showed: for every LED the widest channel range it
    /// showed over the counted frames, the mean absolute frame-to-frame change it made
    /// (the flicker the cap can cost), and the mean level it showed.
    #[derive(Default)]
    struct Stats {
        /// The per-channel (min, max) every LED showed.
        ranges: Vec<[(u8, u8); 3]>,
        /// The sum of the frame-to-frame changes every LED made.
        changes: Vec<f64>,
        /// The sum of the mean channel level every LED showed.
        levels: Vec<f64>,
        /// The frames counted.
        counted: usize,
        /// The frame pairs counted.
        frames: usize,
        /// The last frame counted.
        last: Option<Vec<Rgb8>>,
    }

    impl Stats {
        /// The stats of a sampling of `leds` LEDs.
        fn new(leds: usize) -> Self {
            Stats {
                ranges: vec![[(255u8, 0u8); 3]; leds],
                changes: vec![0.0; leds],
                levels: vec![0.0; leds],
                counted: 0,
                frames: 0,
                last: None,
            }
        }

        /// Adds one frame's colours.
        fn add(&mut self, colours: &[Rgb8]) {
            for (index, (range, colour)) in self.ranges.iter_mut().zip(colours).enumerate() {
                for (slot, v) in range.iter_mut().zip([colour.r, colour.g, colour.b]) {
                    slot.0 = slot.0.min(v);
                    slot.1 = slot.1.max(v);
                }
                let level = (f64::from(colour.r) + f64::from(colour.g) + f64::from(colour.b)) / 3.0;
                self.levels[index] += level;
            }
            self.counted += 1;
            if let Some(previous) = self.last.replace(colours.to_vec()) {
                for (index, (a, b)) in previous.iter().zip(colours).enumerate() {
                    let change = (f64::from(a.r) - f64::from(b.r)).abs()
                        + (f64::from(a.g) - f64::from(b.g)).abs()
                        + (f64::from(a.b) - f64::from(b.b)).abs();
                    self.changes[index] += change / 3.0;
                }
                self.frames += 1;
            }
        }

        /// The widest channel range every LED showed.
        fn widest(&self) -> Vec<u8> {
            self.ranges
                .iter()
                .map(|r| r.iter().map(|(lo, hi)| hi.saturating_sub(*lo)).max().unwrap_or(0))
                .collect()
        }

        /// The mean absolute frame-to-frame change every LED made.
        fn mean_change(&self) -> Vec<f64> {
            let frames = self.frames.max(1) as f64;
            self.changes.iter().map(|c| c / frames).collect()
        }

        /// The mean level every LED showed, in 0..255.
        fn levels(&self) -> Vec<f64> {
            let counted = self.counted.max(1) as f64;
            self.levels.iter().map(|l| l / counted).collect()
        }

        /// The mean of the per-LED mean changes: the flicker of the whole strip.
        fn flicker(&self) -> f64 {
            let changes = self.mean_change();
            if changes.is_empty() {
                return 0.0;
            }
            changes.iter().sum::<f64>() / changes.len() as f64
        }
    }

    /// The old sampling of #372: a cell as wide as the LED's share of the slot and as
    /// tall as the whole slot, the full-height band.
    fn old_band_cells(n: usize) -> Vec<LedSample> {
        let share = CANVAS_WIDTH as f32 / n as f32;
        let scale = SCENE_RENDER_SCALE;
        (0..n)
            .map(|j| {
                let x = share * (j as f32 + 0.5);
                LedSample {
                    center_x: x * scale,
                    center_y: CANVAS_HEIGHT as f32 / 2.0 * scale,
                    bounds: [
                        (x - share / 2.0) * scale,
                        0.0,
                        (x + share / 2.0) * scale,
                        CANVAS_HEIGHT as f32 * scale,
                    ],
                    sampled: true,
                }
            })
            .collect()
    }

    /// Measures one effect on an 8-LED strip in "each its own", for the candidate's cells
    /// and for the old full-height band, over the same frames: the two samplings read the
    /// same canvas, so the difference is the sampling and nothing else.
    fn measure_effect(
        effect: Box<dyn Effect>,
        props: Option<Props>,
        audio: bool,
        cells: &[Vec<LedSample>],
        rule: Rule,
    ) -> (Vec<Stats>, Stats) {
        let mut scene = state_playing(&["d1#z0"], vec![light("d1")], Some(effect), 100);
        scene.groups[0].mode = GroupMode::EachItsOwn;
        scene.groups[0].needs = InputNeeds { screen: false, audio };
        if let Some(props) = props {
            scene.groups[0].pending_props = Some(props);
        }
        let mut runner = SceneRunner::new();
        let devices = vec![strip_device("d1", 8)];
        let band = old_band_cells(8);
        let mut candidate: Vec<Stats> = cells.iter().map(|c| Stats::new(c.len())).collect();
        let mut old = Stats::new(8);
        for index in 0..RAIN_WARMUP_FRAMES + 120 {
            let mut frame_params = params();
            if audio {
                frame_params.input_source = Some(Arc::new(LoudBands { magnitude: 1.0 }));
            }
            frame_with(&mut scene, &mut runner, &devices, &frame_params);
            // The old band reads the same canvas the candidate's cells read.
            let group = &runner.groups[0];
            let before = sample_cells(
                &group.canvas.pixels,
                group.canvas.width,
                group.canvas.height,
                &band,
                Rule::Mean,
            );
            if index >= RAIN_WARMUP_FRAMES {
                for (slot, led_cells) in cells.iter().enumerate() {
                    let now = sample_cells(
                        &group.canvas.pixels,
                        group.canvas.width,
                        group.canvas.height,
                        led_cells,
                        rule,
                    );
                    candidate[slot].add(&now);
                }
                old.add(&before);
            }
        }
        (candidate, old)
    }

    /// Measures every built-in effect on an 8-LED strip in "each its own" and prints the
    /// table: the widest channel range every LED shows over 120 frames after the warm-up,
    /// and the mean frame-to-frame change, for the capped cell and for the old band.
    ///
    /// This is a measurement, not a verdict, so it is ignored by the suite. Run it with:
    /// `cargo test -p canvas-core -- --ignored --nocapture --test-threads=1 measure_every_effect`
    /// One effect to measure: its name, the effect, the props it runs with, and whether
    /// it needs a spectrum.
    struct Measured {
        name: &'static str,
        effect: Box<dyn Effect>,
        props: Option<Props>,
        audio: bool,
    }

    #[test]
    #[ignore = "#372: a measurement of the cap's cost, run by hand"]
    fn measure_every_effect_on_a_strip() {
        // The candidates, in the order of preference, and the four checks each one has to
        // pass. The first that passes all four is the rule that ships.
        let today = measure_shape(Rule::Mean, Shape::Today);
        println!(
            "#372 candidate {}: R1 own {}..{} flow {}..{} | R2 {:.1} against the old band {:.1} | R3 {:.1} {:.1} {:.1} | R4 flat {:?} gradient {} | {}",
            Shape::Today.name(),
            today.rain_own.iter().copied().min().unwrap_or(0),
            today.rain_own.iter().copied().max().unwrap_or(0),
            today.rain_flow.iter().copied().min().unwrap_or(0),
            today.rain_flow.iter().copied().max().unwrap_or(0),
            today.fire,
            today.fire_old,
            today.audio[0],
            today.audio[1],
            today.audio[2],
            today.flat,
            today.gradient,
            if today.passes() { "PASSES" } else { "fails" },
        );
        let bottom = measure_shape(Rule::Mean, Shape::Bottom);
        println!(
            "#372 diagnostic {}: R1 own {}..{} flow {}..{} | R2 {:.1} against the old band {:.1} | R3 {:.1} {:.1} {:.1} | R4 flat {:?} gradient {} | {}",
            Shape::Bottom.name(),
            bottom.rain_own.iter().copied().min().unwrap_or(0),
            bottom.rain_own.iter().copied().max().unwrap_or(0),
            bottom.rain_flow.iter().copied().min().unwrap_or(0),
            bottom.rain_flow.iter().copied().max().unwrap_or(0),
            bottom.fire,
            bottom.fire_old,
            bottom.audio[0],
            bottom.audio[1],
            bottom.audio[2],
            bottom.flat,
            bottom.gradient,
            if bottom.passes() { "PASSES" } else { "fails" },
        );
        let mut winner: Option<(Rule, Shape)> = None;
        for (rule, width) in candidates() {
            let shape = Shape::Column(width);
            let verdict = measure_shape(rule, shape);
            println!(
                "#372 candidate {} {}: R1 own {}..{} flow {}..{} | R2 {:.1} against the old band {:.1} | R3 {:.1} {:.1} {:.1} | R4 flat {:?} gradient {} | {}",
                rule.name(),
                shape.name(),
                verdict.rain_own.iter().copied().min().unwrap_or(0),
                verdict.rain_own.iter().copied().max().unwrap_or(0),
                verdict.rain_flow.iter().copied().min().unwrap_or(0),
                verdict.rain_flow.iter().copied().max().unwrap_or(0),
                verdict.fire,
                verdict.fire_old,
                verdict.audio[0],
                verdict.audio[1],
                verdict.audio[2],
                verdict.flat,
                verdict.gradient,
                if verdict.passes() { "PASSES" } else { "fails" },
            );
            println!(
                "#372 candidate {} {}: R1 own {own:?} flow {flow:?}",
                rule.name(),
                shape.name(),
                own = verdict.rain_own,
                flow = verdict.rain_flow,
            );
            if verdict.passes() {
                winner = Some((rule, shape));
                break;
            }
        }
        let (rule, shape) = match winner {
            Some((rule, shape)) => {
                println!("#372 the rule that ships: {} {}", rule.name(), shape.name());
                (rule, shape)
            }
            None => {
                println!("#372 no candidate passes all four: the rule does not change");
                println!("#372 the cost of the rule that ships, against the old full-height band");
                (Rule::Mean, Shape::Today)
            }
        };

        let effects: Vec<Measured> = vec![
            Measured {
                name: "Solid",
                effect: Box::new(canvas_effects::solid::Solid::new()),
                props: None,
                audio: false,
            },
            Measured {
                name: "Gradient",
                effect: Box::new(canvas_effects::gradient::Gradient::new()),
                props: None,
                audio: false,
            },
            Measured {
                name: "Gradient (scrolling, 20)",
                effect: Box::new(canvas_effects::gradient::Gradient::new()),
                props: Some(Props::from([(
                    "scroll_speed".to_string(),
                    canvas_proto::props::PropValue::Number(20.0),
                )])),
                audio: false,
            },
            Measured {
                name: "Rainbow Wave",
                effect: Box::new(canvas_effects::rainbow_wave::RainbowWave::new()),
                props: None,
                audio: false,
            },
            Measured {
                name: "Spiral",
                effect: Box::new(canvas_effects::spiral::Spiral::new()),
                props: None,
                audio: false,
            },
            Measured {
                name: "Breathing",
                effect: Box::new(canvas_effects::breathing::Breathing::new()),
                props: None,
                audio: false,
            },
            Measured {
                name: "Color Cycle",
                effect: Box::new(canvas_effects::color_cycle::ColorCycle::new()),
                props: None,
                audio: false,
            },
            Measured {
                name: "Aurora",
                effect: Box::new(canvas_effects::aurora::Aurora::new()),
                props: None,
                audio: false,
            },
            Measured {
                name: "Rain",
                effect: Box::new(canvas_effects::rain::Rain::with_seed(372)),
                props: Some(purple_rain_props()),
                audio: false,
            },
            Measured {
                name: "Fire",
                effect: Box::new(canvas_effects::fire::Fire::with_seed(372)),
                props: None,
                audio: false,
            },
            Measured {
                name: "Audio Spectrum",
                effect: Box::new(canvas_effects::audio_spectrum::AudioSpectrum::new()),
                props: None,
                audio: true,
            },
        ];
        let cells = shape.cells(0.0, 320.0, 8);
        println!("#372 effect | new rule range | old range | new rule change | old change | new rule level | old level");
        for measured in effects {
            let (candidate, old) =
                measure_effect(measured.effect, measured.props, measured.audio, std::slice::from_ref(&cells), rule);
            let candidate_range = candidate[0].widest();
            let old_range = old.widest();
            let candidate_change = candidate[0].flicker();
            let old_change = old.flicker();
            let candidate_level = candidate[0].levels();
            let old_level = old.levels();
            println!(
                "#372 {} | {}..{} | {}..{} | {candidate_change:.3} | {old_change:.3} | {}..{} | {}..{}",
                measured.name,
                candidate_range.iter().min().unwrap_or(&0),
                candidate_range.iter().max().unwrap_or(&0),
                old_range.iter().min().unwrap_or(&0),
                old_range.iter().max().unwrap_or(&0),
                candidate_level.iter().cloned().fold(f64::MAX, f64::min).round() as i64,
                candidate_level.iter().cloned().fold(f64::MIN, f64::max).round() as i64,
                old_level.iter().cloned().fold(f64::MAX, f64::min).round() as i64,
                old_level.iter().cloned().fold(f64::MIN, f64::max).round() as i64,
            );
            println!(
                "#372 {} per LED: new rule {candidate_range:?} change {:?} level {:?} | old {old_range:?} change {:?} level {:?}",
                measured.name,
                candidate[0].mean_change(),
                candidate_level.iter().map(|l| l.round()).collect::<Vec<f64>>(),
                old.mean_change(),
                old_level.iter().map(|l| l.round()).collect::<Vec<f64>>(),
            );
        }
    }
}
