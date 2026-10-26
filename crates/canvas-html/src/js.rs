//! The JavaScript side of the runtime: a QuickJS context with the DOM-shaped globals a
//! SignalRGB effect expects, and a `CanvasRenderingContext2D` implemented in Rust.
//!
//! The host API an effect sees:
//! - `window` (the global object), `document.getElementById(id)`, `canvas.getContext("2d")`
//! - `requestAnimationFrame(cb)` / `cancelAnimationFrame(id)` — the callback is stored
//!   and called once per frame, exactly as the browser (and `tools/poison-compare`'s
//!   Chromium harness) drives it.
//! - `Math.random` (seedable, so a run is reproducible)
//! - `performance.now()` / `Date.now()` — driven by the effect's clock
//! - `console.log` — forwarded to `tracing`
//! - `setTimeout(fn, 0)` — a zero-delay queue drained at the end of each frame
//!
//! The canvas' 2D context is a plain JS object whose methods call back into the Rust
//! raster context.
//!
//! # Threading and lifetimes
//!
//! `Effect` requires `Send`, so the host state is behind an `Arc<Mutex<..>>`. The
//! animation-frame callback and the queued timers live in JavaScript globals, so
//! QuickJS owns them and their lifetime matches the runtime's.
//!
//! **No JavaScript closure may capture a [`Ctx`]**. `Ctx::clone` calls
//! `JS_DupContext`, and a closure that lives in the context's own object list would
//! hold that reference past the runtime's teardown, which trips QuickJS's
//! "list_empty(&rt->gc_obj_list)" assertion. The closures therefore take their
//! context from the call's `this` binding ([`rquickjs::function::This`]) and create
//! their objects through it.

use crate::color::CssColor;
use crate::raster::{Context as RasterContext, FillStyle, LinearGradientSpec, RadialGradientSpec};
use rquickjs::function::Opt;
use rquickjs::{Array, Context, Ctx, Function, Object, Runtime, TypedArray, U8Clamped, Value};

/// The shared, mutable Rust state the JS closures reach into.
pub type Shared<T> = std::sync::Arc<std::sync::Mutex<T>>;

/// The host's Rust state: the raster context, the clock and the random source.
pub struct Host {
    /// The canvas' pixel buffer. Drawing goes here.
    pub raster: RasterContext,
    /// The effect clock in milliseconds.
    pub now_ms: f64,
    /// The seeded random source.
    pub rng: Rng,
    /// The gradients the script created, indexed by the id the JS object carries.
    pub gradients: Vec<GradientSpec>,
}

/// A gradient's geometry and stops.
#[derive(Clone)]
pub enum GradientSpec {
    Linear(LinearGradientSpec, SharedStops),
    Radial(RadialGradientSpec, SharedStops),
}

impl GradientSpec {
    /// The gradient's stops, cloned out of the shared cell.
    pub fn stops(&self) -> Vec<(f32, CssColor)> {
        match self {
            GradientSpec::Linear(_, stops) | GradientSpec::Radial(_, stops) => {
                stops.lock().map(|s| s.clone()).unwrap_or_default()
            }
        }
    }

    /// Adds a color stop.
    pub fn push_stop(&self, offset: f32, color: CssColor) {
        let stops = match self {
            GradientSpec::Linear(_, stops) | GradientSpec::Radial(_, stops) => stops,
        };
        if let Ok(mut stops) = stops.lock() {
            stops.push((offset, color));
        }
    }
}

impl Host {
    fn new(raster: RasterContext) -> Self {
        Self {
            raster,
            now_ms: 0.0,
            rng: Rng::new(0x1234_5678),
            gradients: Vec::new(),
        }
    }
}

/// A small, fast, seedable PRNG. The reference harness uses mulberry32; matching it
/// makes a Rust run comparable with a Chromium run when the same seed is used.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u32,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// mulberry32, the generator the reference Chromium harness uses.
    pub fn next_unit(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0
    }
}

/// The gradient's stops. They live on the Rust side, in the host's gradient registry;
/// the JS object only carries the gradient's id, so the painter can find them again.
pub type SharedStops = std::sync::Arc<std::sync::Mutex<Vec<(f32, CssColor)>>>;

/// The host's JavaScript-side bookkeeping.
///
/// The animation-frame callback and the queued timers live in JavaScript globals
/// (`__rafCb` and `__timeouts`), so QuickJS owns them and their lifetime matches the
/// runtime's. Only the counter of handed-out ids lives here.
#[derive(Default)]
pub struct JsHandles {
    /// The id handed out to the next `requestAnimationFrame`.
    pub next_raf_id: u32,
    /// The id of the registered callback, so `cancelAnimationFrame(id)` only clears the
    /// callback it names.
    pub pending_raf_id: Option<u32>,
}

/// The handles, shared with the JS closures.
pub type SharedHandles = Shared<JsHandles>;

/// The global the animation-frame callback is stored in.
pub const RAF_SLOT: &str = "__rafCb";
/// The global the queued zero-delay timers are stored in.
pub const TIMEOUT_SLOT: &str = "__timeouts";

/// Sets up the whole JS environment: `window`, `document`, `console`, `Math.random`,
/// `performance`, `requestAnimationFrame` and `setTimeout`.
///
/// `host` is shared with the drawing methods; `handles` holds the id counter;
/// `canvas_id` is the id the effect looks the canvas up with.
pub fn setup<'js>(
    ctx: Ctx<'js>,
    host: Shared<Host>,
    handles: SharedHandles,
    canvas_id: &str,
    width: u32,
    height: u32,
    seed: u32,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();

    // The seed makes the effect's random sequence reproducible.
    if let Ok(mut host) = host.lock() {
        host.rng = Rng::new(seed);
    }

    // The slot the animation-frame callback lives in, and the queue of zero-delay
    // timers. Both are JavaScript values, so QuickJS owns their lifetime.
    globals.set(RAF_SLOT, Value::new_null(ctx.clone()))?;
    globals.set(TIMEOUT_SLOT, Array::new(ctx.clone()))?;
    // `window` is the global object.
    globals.set("window", globals.clone())?;

    // `Math.random` is replaced with a seeded generator, so a run is reproducible.
    let math = globals.get::<_, Object<'js>>("Math")?;
    let random_host = host.clone();
    let random_fn = Function::new(ctx.clone(), move || -> f64 {
        random_host
            .lock()
            .map(|mut host| host.rng.next_unit())
            .unwrap_or(0.0)
    })?;
    math.set("random", random_fn)?;

    // `console.log` forwards to tracing. The four names are aliases made in
    // JavaScript, so no function object has to be cloned (cloning one leaks a context
    // reference in rquickjs).
    let console = Object::new(ctx.clone())?;
    let log_fn = Function::new(ctx.clone(), |args: rquickjs::function::Rest<Value<'js>>| {
        let text = args
            .0
            .iter()
            .map(|v| match v.as_string() {
                Some(s) => s.to_string().unwrap_or_default(),
                None => "<value>".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        tracing::debug!("[effect] {text}");
        Ok::<(), rquickjs::Error>(())
    })?;
    let _ = console.set("log", log_fn);
    let _ = globals.set("console", console);
    // `info`, `warn` and `error` are the same function.
    ctx.eval::<(), _>(
        "console.info = console.log; console.warn = console.log; console.error = console.log;",
    )?;

    // The canvas element and its 2D context are built once, here, so the closures
    // never have to create an object.
    //
    // `addColorStop` is one function shared by every gradient: it finds the stops of
    // the gradient it is called on through the host's registry. It is exposed to the
    // script through a small JavaScript wrapper, so no JavaScript function object has
    // to be cloned (cloning one leaks a context reference in rquickjs).
    let gradient_host = host.clone();
    let add_color_stop = Function::new(ctx.clone(), move |id: u32, offset: f64, color: String| {
        let css = crate::color::parse(&color).unwrap_or(CssColor::BLACK);
        if let Ok(state) = gradient_host.lock() {
            if let Some(spec) = state.gradients.get(id as usize) {
                spec.push_stop(offset as f32, css);
            }
        }
        Ok::<(), rquickjs::Error>(())
    })?;
    globals.set("__addColorStop", add_color_stop)?;
    // The gradient factory is a JavaScript function, defined once. The Rust side calls it
    // through the global, so no function object is cloned into a closure.
    ctx.eval::<(), _>(
        "function __makeGradient() { return { addColorStop: function (offset, color) { __addColorStop(this.__gradient, offset, color); } }; }",
    )?;
    let context_2d = make_context_2d(ctx.clone(), host.clone())?;
    globals.set("__canvasContext", context_2d)?;
    let element = Object::new(ctx.clone())?;
    element.set("id", canvas_id)?;
    element.set("width", width)?;
    element.set("height", height)?;
    globals.set("__canvasElement", element)?;
    // `getContext("2d")` returns the canvas' 2D context. The wrapper is written in
    // JavaScript so no function object has to be cloned.
    ctx.eval::<(), _>(
        "__canvasElement.getContext = function (kind) { return kind === '2d' ? __canvasContext : null; };",
    )?;
    // `document.getElementById(id)` returns the canvas element. The element lives in a
    // global so no object has to be cloned (cloning one leaks a context reference in
    // rquickjs).
    let wanted_id = canvas_id.to_string();
    let get_element_by_id = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, id: String| -> rquickjs::Result<Object<'js>> {
        if id != wanted_id {
            return Object::new(_this.0.ctx().clone());
        }
        let globals = _this.0.ctx().globals();
        globals.get::<_, Object<'js>>("__canvasElement")
    })?;

    let document = Object::new(ctx.clone())?;
    document.set("getElementById", get_element_by_id)?;
    globals.set("document", document)?;

    // `requestAnimationFrame` stores the callback in a global; the host calls it once
    // per frame. The callback lives in JavaScript, so QuickJS owns it.
    //
    // The public names are thin JavaScript wrappers around the Rust functions, so no
    // function object has to be cloned (cloning one leaks a context reference in
    // rquickjs).
    let raf_handles = handles.clone();
    let raf = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, cb: Function<'js>| -> rquickjs::Result<u32> {
        let mut handles = raf_handles.lock().map_err(|_| {
            rquickjs::Error::new_from_js_message(
                "canvas-html",
                "canvas-html",
                "the host lock was poisoned",
            )
        })?;
        let id = handles.next_raf_id;
        handles.next_raf_id = handles.next_raf_id.wrapping_add(1);
        handles.pending_raf_id = Some(id);
        // The callback goes into the global object of the calling context.
        let globals = this.0.ctx().globals();
        globals.set(RAF_SLOT, cb)?;
        Ok(id)
    })?;

    // `cancelAnimationFrame(id)` clears the slot when `id` names the pending callback.
    let cancel_handles = handles.clone();
    let cancel_raf = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, id: u32| {
        let Ok(mut handles) = cancel_handles.lock() else { return };
        if handles.pending_raf_id != Some(id) {
            return;
        }
        handles.pending_raf_id = None;
        let globals = this.0.ctx().globals();
        let _ = globals.set(RAF_SLOT, Value::new_null(this.0.ctx().clone()));
    })?;

    globals.set("__requestAnimationFrame", raf)?;
    globals.set("__cancelAnimationFrame", cancel_raf)?;
    // The window is the global object, so the public names are defined there.
    ctx.eval::<(), _>(
        "this.requestAnimationFrame = function (cb) { return __requestAnimationFrame(cb); }; \
         this.cancelAnimationFrame = function (id) { return __cancelAnimationFrame(id); };",
    )?;

    // `performance.now()` returns the effect clock.
    let clock_host = host.clone();
    let performance = Object::new(ctx.clone())?;
    let now_fn = Function::new(ctx.clone(), move || -> f64 {
        clock_host.lock().map(|h| h.now_ms).unwrap_or(0.0)
    })?;
    performance.set("now", now_fn)?;
    globals.set("performance", performance)?;

    // `Date.now()` is driven by the effect clock too, so an effect that reads it stays
    // in step with `performance.now()`.
    let date_host = host.clone();
    let date = globals.get::<_, Object<'js>>("Date")?;
    let date_now = Function::new(ctx.clone(), move || -> f64 {
        date_host.lock().map(|h| h.now_ms).unwrap_or(0.0)
    })?;
    date.set("now", date_now)?;

    // `setTimeout(fn, 0)` queues the callback in a JavaScript array; the host drains
    // the queue each frame.
    let set_timeout = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, cb: Function<'js>, _delay: Opt<f64>| -> rquickjs::Result<()> {
        let globals = this.0.ctx().globals();
        let queue = globals.get::<_, Array<'js>>(TIMEOUT_SLOT)?;
        queue.set(queue.len(), cb)?;
        Ok(())
    })?;
    globals.set("setTimeout", set_timeout)?;

    Ok(())
}

/// Reads the fill style off the context object. It is either a color string or a
/// gradient object; anything else keeps the previous style, as a browser does.
fn fill_style_of(obj: &Object<'_>, state: &Host) -> FillStyle {
    style_of(obj, "fillStyle", state)
}

/// Reads the stroke style off the context object.
fn stroke_style_of(obj: &Object<'_>, state: &Host) -> FillStyle {
    style_of(obj, "strokeStyle", state)
}

/// Reads a style property, which may be a color string or a gradient object.
fn style_of(obj: &Object<'_>, name: &str, state: &Host) -> FillStyle {
    match obj.get::<_, Value<'_>>(name) {
        Ok(value) => {
            if let Some(text) = value.as_string() {
                let text = text.to_string().unwrap_or_default();
                return crate::color::parse(&text)
                    .map(FillStyle::Solid)
                    .unwrap_or(FillStyle::Solid(CssColor::BLACK));
            }
            if let Some(object) = value.as_object() {
                if let Some(style) = gradient_style(object, state) {
                    return style;
                }
            }
            FillStyle::Solid(CssColor::BLACK)
        }
        Err(_) => FillStyle::Solid(CssColor::BLACK),
    }
}

/// Copies the context object's state properties into the raster context. The script
/// writes them as plain properties (`ctx.globalAlpha = 0.5`), so they are read back
/// before every drawing operation.
fn sync_state(obj: &Object<'_>, raster: &mut RasterContext) {
    if let Ok(alpha) = obj.get::<_, f64>("globalAlpha") {
        raster.set_global_alpha(alpha as f32);
    }
    if let Ok(width) = obj.get::<_, f64>("lineWidth") {
        raster.set_line_width(width as f32);
    }
    if let Ok(op) = obj.get::<_, String>("globalCompositeOperation") {
        raster.set_composite(crate::raster::Composite::parse(&op));
    }
}

/// Reads a gradient object back into a fill style, using the host's gradient registry.
fn gradient_style(obj: &Object<'_>, state: &Host) -> Option<FillStyle> {
    let id = obj.get::<_, u32>("__gradient").ok()? as usize;
    let spec = state.gradients.get(id)?;
    let stops = spec.stops();
    Some(match spec {
        GradientSpec::Linear(linear, _) => FillStyle::Linear(LinearGradientSpec {
            x0: linear.x0,
            y0: linear.y0,
            x1: linear.x1,
            y1: linear.y1,
            stops,
        }),
        GradientSpec::Radial(radial, _) => FillStyle::Radial(RadialGradientSpec {
            x0: radial.x0,
            y0: radial.y0,
            r0: radial.r0,
            x1: radial.x1,
            y1: radial.y1,
            r1: radial.r1,
            stops,
        }),
    })
}

/// Registers the 2D context's methods. Each one reads the object's style properties and
/// calls into the Rust raster context.
///
/// Every method takes its context from the call's `this` binding, because a closure
/// may not capture a [`Ctx`] (see the module docs).
fn define_context_methods<'js>(ctx: &Ctx<'js>, obj: &Object<'js>, host: Shared<Host>) {
    // The style properties live on the JS object; the drawing methods read them from
    // the `this` binding the script passes in.
    //
    // The host handle is called `state` throughout, because the drawing methods also
    // take a `height` argument.
    macro_rules! method {
        ($name:literal, $f:expr) => {{
            let f = $f;
            let _ = obj.set($name, f);
        }};
    }

    // --- shapes ---------------------------------------------------------

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, x: f64, y: f64, w: f64, h: f64| {
        let Some(this) = this.0.as_object() else {
            return Ok::<(), rquickjs::Error>(());
        };
        let Ok(mut state) = state.lock() else { return Ok::<(), rquickjs::Error>(()) };
        sync_state(this, &mut state.raster);
        let style = fill_style_of(this, &state);
        state.raster.set_fill_style(style);
        state.raster.fill_rect(x as f32, y as f32, w as f32, h as f32);
        Ok::<(), rquickjs::Error>(())
    });
    method!("fillRect", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, x: f64, y: f64, w: f64, h: f64| {
        let Some(this) = this.0.as_object() else {
            return Ok::<(), rquickjs::Error>(());
        };
        let Ok(mut state) = state.lock() else { return Ok::<(), rquickjs::Error>(()) };
        sync_state(this, &mut state.raster);
        state.raster.clear_rect(x as f32, y as f32, w as f32, h as f32);
        Ok::<(), rquickjs::Error>(())
    });
    method!("clearRect", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, x: f64, y: f64, w: f64, h: f64| {
        let Some(this) = this.0.as_object() else {
            return Ok::<(), rquickjs::Error>(());
        };
        let Ok(mut state) = state.lock() else { return Ok::<(), rquickjs::Error>(()) };
        sync_state(this, &mut state.raster);
        let style = stroke_style_of(this, &state);
        state.raster.set_stroke_style(style);
        state.raster.stroke_rect(x as f32, y as f32, w as f32, h as f32);
        Ok::<(), rquickjs::Error>(())
    });
    method!("strokeRect", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>| {
        if let Ok(mut state) = state.lock() {
            state.raster.begin_path();
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("beginPath", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>| {
        if let Ok(mut state) = state.lock() {
            state.raster.close_path();
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("closePath", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, x: f64, y: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.move_to(x as f32, y as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("moveTo", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, x: f64, y: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.line_to(x as f32, y as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("lineTo", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, cx: f64, cy: f64, x: f64, y: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.quadratic_curve_to(cx as f32, cy as f32, x as f32, y as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("quadraticCurveTo", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.bezier_curve_to(c1x as f32, c1y as f32, c2x as f32, c2y as f32, x as f32, y as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("bezierCurveTo", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, x: f64, y: f64, r: f64, start: f64, end: f64, ccw: Opt<bool>| {
        let ccw = ccw.0.unwrap_or(false);
        if let Ok(mut state) = state.lock() {
            state.raster.arc(x as f32, y as f32, r as f32, start as f32, end as f32, ccw);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("arc", f);

    // `ellipse` takes eight arguments. A JS function here can take seven parameters in
    // total, so the last two arrive in the rest arguments.
    let state = host.clone();
    let f = Function::new(ctx.clone(), move |x: f64, y: f64, rx: f64, ry: f64, rot: f64, start: f64, rest: rquickjs::function::Rest<Value<'js>>| {
        let end = rest.0.first().and_then(|v| v.as_number()).unwrap_or(0.0);
        let ccw = rest.0.get(1).and_then(|v| v.as_bool()).unwrap_or(false);
        if let Ok(mut state) = state.lock() {
            state.raster.ellipse(x as f32, y as f32, rx as f32, ry as f32, rot as f32, start as f32, end as f32, ccw);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("ellipse", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, x: f64, y: f64, w: f64, h: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.rect(x as f32, y as f32, w as f32, h as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("rect", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>| {
        let Some(this) = this.0.as_object() else {
            return Ok::<(), rquickjs::Error>(());
        };
        let Ok(mut state) = state.lock() else { return Ok::<(), rquickjs::Error>(()) };
        sync_state(this, &mut state.raster);
        let style = fill_style_of(this, &state);
        state.raster.set_fill_style(style);
        state.raster.fill();
        Ok::<(), rquickjs::Error>(())
    });
    method!("fill", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>| {
        let Some(this) = this.0.as_object() else {
            return Ok::<(), rquickjs::Error>(());
        };
        let Ok(mut state) = state.lock() else { return Ok::<(), rquickjs::Error>(()) };
        sync_state(this, &mut state.raster);
        let style = stroke_style_of(this, &state);
        state.raster.set_stroke_style(style);
        state.raster.stroke();
        Ok::<(), rquickjs::Error>(())
    });
    method!("stroke", f);

    // --- state ----------------------------------------------------------
    //
    // The script writes the state properties on the context object, so `save` and
    // `restore` snapshot them there.

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>| {
        if let Ok(mut state) = state.lock() {
            state.raster.save();
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("save", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>| {
        let Some(this) = this.0.as_object() else {
            return Ok::<(), rquickjs::Error>(());
        };
        let Ok(mut state) = state.lock() else { return Ok::<(), rquickjs::Error>(()) };
        state.raster.restore();
        // Write the restored values back onto the JS object.
        let alpha = state.raster.global_alpha();
        let width = state.raster.line_width();
        let composite = state.raster.composite();
        let _ = this.set("globalAlpha", alpha as f64);
        let _ = this.set("lineWidth", width as f64);
        let _ = this.set("globalCompositeOperation", composite);
        Ok::<(), rquickjs::Error>(())
    });
    method!("restore", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, x: f64, y: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.translate(x as f32, y as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("translate", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, x: f64, y: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.scale(x as f32, y as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("scale", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, angle: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.rotate(angle as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("rotate", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64| {
        if let Ok(mut state) = state.lock() {
            state.raster.set_transform(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("setTransform", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>| {
        if let Ok(mut state) = state.lock() {
            state.raster.reset_transform();
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("resetTransform", f);

    // --- gradients ------------------------------------------------------
    //
    // A gradient is a JS object carrying an id; the geometry lives in the host's
    // gradient registry and the stops in a shared cell, and the painter reads them
    // when the gradient is used as a fill or stroke style.

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, x0: f64, y0: f64, x1: f64, y1: f64| -> rquickjs::Result<Object<'js>> {
        // The gradient object is built in JavaScript, so no context is captured.
        let make: Function<'js> = this.0.ctx().globals().get("__makeGradient")?;
        let gradient: Object<'js> = make.call(())?;
        let id = {
            let mut state = state.lock().map_err(|_| {
                rquickjs::Error::new_from_js_message(
                    "canvas-html",
                    "canvas-html",
                    "the host lock was poisoned",
                )
            })?;
            let id = state.gradients.len() as u32;
            state.gradients.push(GradientSpec::Linear(
                LinearGradientSpec {
                    x0: x0 as f32,
                    y0: y0 as f32,
                    x1: x1 as f32,
                    y1: y1 as f32,
                    stops: Vec::new(),
                },
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            ));
            id
        };
        gradient.set("__gradient", id)?;
        Ok(gradient)
    });
    method!("createLinearGradient", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64| -> rquickjs::Result<Object<'js>> {
        let make: Function<'js> = this.0.ctx().globals().get("__makeGradient")?;
        let gradient: Object<'js> = make.call(())?;
        let id = {
            let mut state = state.lock().map_err(|_| {
                rquickjs::Error::new_from_js_message(
                    "canvas-html",
                    "canvas-html",
                    "the host lock was poisoned",
                )
            })?;
            let id = state.gradients.len() as u32;
            state.gradients.push(GradientSpec::Radial(
                RadialGradientSpec {
                    x0: x0 as f32,
                    y0: y0 as f32,
                    r0: r0 as f32,
                    x1: x1 as f32,
                    y1: y1 as f32,
                    r1: r1 as f32,
                    stops: Vec::new(),
                },
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            ));
            id
        };
        gradient.set("__gradient", id)?;
        Ok(gradient)
    });
    method!("createRadialGradient", f);

    // --- image data -----------------------------------------------------
    //
    // `ImageData.data` is a real `Uint8ClampedArray`, as in a browser, so an effect can
    // read and write individual bytes.

    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, w: f64, h: f64| -> rquickjs::Result<Object<'js>> {
        let (w, h) = (w.max(0.0) as u32, h.max(0.0) as u32);
        let bytes = vec![U8Clamped(0); (w * h * 4) as usize];
        let image = this.0.ctx().eval::<Object<'js>, _>("({})")?;
        image.set("width", w)?;
        image.set("height", h)?;
        image.set("data", TypedArray::<U8Clamped>::new_copy(this.0.ctx().clone(), &bytes)?)?;
        Ok(image)
    });
    method!("createImageData", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |this: rquickjs::function::This<Value<'js>>, x: f64, y: f64, w: f64, h: f64| -> rquickjs::Result<Object<'js>> {
        let bytes = match state.lock() {
            Ok(state) => state.raster.get_image_data(x as i32, y as i32, w as i32, h as i32),
            Err(_) => Vec::new(),
        };
        let (w, h) = (w.max(0.0) as u32, h.max(0.0) as u32);
        let clamped: Vec<U8Clamped> = bytes.into_iter().map(U8Clamped).collect();
        let image = this.0.ctx().eval::<Object<'js>, _>("({})")?;
        image.set("width", w)?;
        image.set("height", h)?;
        image.set("data", TypedArray::<U8Clamped>::new_copy(this.0.ctx().clone(), &clamped)?)?;
        Ok(image)
    });
    method!("getImageData", f);

    let state = host.clone();
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, image: Object<'js>, x: f64, y: f64| {
        let width = image.get::<_, u32>("width").unwrap_or(0);
        let bytes = match image.get::<_, TypedArray<'js, U8Clamped>>("data") {
            Ok(array) => match array.as_raw() {
                // SAFETY: the typed array's buffer is alive for the duration of the call.
                Some(raw) => unsafe { raw.as_ref() }.to_vec(),
                None => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        if let Ok(mut state) = state.lock() {
            state.raster.put_image_data_with_width(&bytes, x as i32, y as i32, width);
        }
        Ok::<(), rquickjs::Error>(())
    });
    method!("putImageData", f);

    // `fillText` is a no-op that logs: text rendering needs a font engine, which is out
    // of scope for v1.
    let f = Function::new(ctx.clone(), move |_this: rquickjs::function::This<Value<'js>>, _text: String| {
        tracing::debug!("[effect] fillText is not supported; the text was not drawn");
        Ok::<(), rquickjs::Error>(())
    });
    method!("fillText", f);
}

/// The 2D context object, handed to the effect.
fn make_context_2d<'js>(ctx: Ctx<'js>, host: Shared<Host>) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;

    // The style properties live on the JS object; the drawing methods read them.
    obj.set("fillStyle", "#000000")?;
    obj.set("strokeStyle", "#000000")?;
    obj.set("globalAlpha", 1.0f64)?;
    obj.set("lineWidth", 1.0f64)?;
    obj.set("globalCompositeOperation", "source-over")?;

    define_context_methods(&ctx, &obj, host);
    Ok(obj)
}

/// A QuickJS runtime, its context and the state they drive.
///
/// The fields are ordered by their drop order, which Rust takes as the declaration
/// order: the context must go before the runtime it borrows from.
pub struct JsRuntime {
    /// The QuickJS context. It borrows from the runtime, so it is dropped before it.
    pub context: Context,
    /// The QuickJS runtime. The context borrows from it, so it must outlive the
    /// context; it is kept here and dropped last.
    #[allow(dead_code)]
    pub runtime: Runtime,
    /// The host's Rust state.
    pub host: Shared<Host>,
    /// The host's JavaScript-side bookkeeping.
    pub handles: SharedHandles,
}

impl JsRuntime {
    /// Creates a runtime with a canvas of the given size.
    pub fn new(width: u32, height: u32) -> rquickjs::Result<Self> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        let host = std::sync::Arc::new(std::sync::Mutex::new(Host::new(RasterContext::new(
            width, height,
        ))));
        let handles = std::sync::Arc::new(std::sync::Mutex::new(JsHandles::default()));
        Ok(Self { context, runtime, host, handles })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runtime that has been set up must tear down cleanly.
    ///
    /// QuickJS asserts that every object is freed when the runtime goes, so a leaked
    /// context reference (which is what capturing a `Ctx` in a JavaScript closure
    /// causes) aborts the process here rather than at the end of the daemon's life.
    #[test]
    fn a_set_up_runtime_tears_down_cleanly() {
        let runtime = JsRuntime::new(64, 64).expect("a runtime");
        runtime
            .context
            .with(|ctx| {
                setup(
                    ctx.clone(),
                    runtime.host.clone(),
                    runtime.handles.clone(),
                    "c",
                    64,
                    64,
                    1,
                )
                .expect("the setup runs");
                ctx.eval::<(), _>("requestAnimationFrame(function () {});")
                    .expect("the script runs");
            });
        drop(runtime);
    }

    /// The same, with a script that draws and registers a gradient.
    #[test]
    fn a_drawing_runtime_tears_down_cleanly() {
        let runtime = JsRuntime::new(64, 64).expect("a runtime");
        runtime
            .context
            .with(|ctx| {
                setup(
                    ctx.clone(),
                    runtime.host.clone(),
                    runtime.handles.clone(),
                    "c",
                    64,
                    64,
                    1,
                )
                .expect("the setup runs");
                let result = ctx.eval::<(), _>(
                    "var c = document.getElementById('c'); \
                     var ctx = c.getContext('2d'); \
                     ctx.fillStyle = '#ff0000'; \
                     ctx.fillRect(0, 0, 10, 10); \
                     var g = ctx.createLinearGradient(0, 0, 10, 10); \
                     g.addColorStop(0, '#000000'); \
                     g.addColorStop(1, '#ffffff'); \
                     ctx.fillStyle = g; \
                     ctx.fillRect(0, 0, 10, 10);",
                );
                if let Err(e) = result {
                    let message = ctx.catch().as_exception().map(|ex| ex.to_string());
                    panic!("the script runs: {e:?} {message:?}");
                }
            });
        drop(runtime);
    }
}
