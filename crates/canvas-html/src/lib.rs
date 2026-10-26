//! SignalRGB-format HTML effect runtime.
//!
//! An effect file is a small HTML document with a `<head>` that declares the properties
//! as `<meta property=... label=... type=...>` tags, a `<canvas>` and one or more inline
//! `<script>` blocks. This crate parses that file, runs the scripts in an embedded
//! QuickJS engine (no browser engine anywhere in the render path) and rasterizes the
//! 2D context in Rust with `tiny-skia`.
//!
//! The property convention is SignalRGB's: a property named `speedRaw` is a global
//! named `speedRaw`, a color is a `"#rrggbb"` string, a number is a number and a boolean
//! is a boolean. The values are installed as globals before the script runs and again
//! after its top-level code has run, because SignalRGB effects re-declare them with
//! `var` (`var speedRaw = 0;` in Poison) and SignalRGB applies the user's values after
//! loading.

mod color;
mod js;
mod parse;
mod raster;

use canvas_proto::{
    Canvas, Effect, InputNeeds, Inputs, PropKind, PropValue, PropertyDef, PropertySchema, Props,
};
use js::JsRuntime;
use rquickjs::Value;
use raster::{DEFAULT_HEIGHT, DEFAULT_WIDTH};

#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error("parse: {0}")]
    Parse(String),
    #[error("script: {0}")]
    Script(String),
}

/// The frame rate the effect's animation is defined at. SignalRGB effects are written
/// against `requestAnimationFrame`, i.e. 60 Hz.
const EFFECT_FPS: f32 = 60.0;
/// The most whole frames one `update` may run.
const MAX_FRAMES_PER_UPDATE: u32 = 4;
/// The most distinct error messages an instance keeps.
const MAX_REPORTED: usize = 32;
/// The largest canvas edge an effect file may declare. A bigger canvas is a parse error,
/// so a bad file cannot ask for a multi-gigabyte pixel buffer.
const MAX_CANVAS_EDGE: u32 = 4096;

/// One HTML effect instance: its own JS context and 2D canvas.
pub struct HtmlEffect {
    /// The effect's schema: id, name, description and the property definitions.
    schema: PropertySchema,
    /// The script bodies, in file order.
    scripts: Vec<String>,
    /// The canvas' logical size, as the file declares it.
    width: u32,
    height: u32,
    /// The canvas element's id, so `document.getElementById` finds it.
    canvas_id: String,
    /// The embedded JS engine.
    runtime: JsRuntime,
    /// The fractional frame accumulator.
    accumulator: f32,
    /// The effect clock in seconds, driven by the sum of `dt`.
    time: f64,
    /// The messages already reported, so a repeating error is logged once.
    reported: Vec<String>,
    /// The property values currently installed as globals.
    values: Props,
    /// Whether the script has run.
    loaded: bool,
    /// The source sizes and destination sizes the blit maps were built for.
    blit_key: (usize, usize, usize, usize),
    /// The source column for each destination column, built once per size pair.
    blit_cols: Vec<usize>,
    /// The source row for each destination row, built once per size pair.
    blit_rows: Vec<usize>,
}

impl HtmlEffect {
    /// `id` becomes `schema.id`; name/description/properties come from the file's
    /// `<title>` and `<meta>` tags. `default_overrides` replace the file's declared
    /// defaults for the keys given (keys are the file's `property` names, e.g. `speedRaw`).
    pub fn from_source(id: &str, html: &str, default_overrides: &Props) -> Result<Self, HtmlError> {
        Self::from_source_seeded(id, html, default_overrides, entropy_seed())
    }

    /// The same as [`HtmlEffect::from_source`], with the random seed given. Two instances
    /// built with the same seed draw the same random sequence, so their pictures match.
    pub fn from_source_seeded(
        id: &str,
        html: &str,
        default_overrides: &Props,
        seed: u32,
    ) -> Result<Self, HtmlError> {
        let parsed = parse::parse(html);

        if parsed.scripts.is_empty() {
            return Err(HtmlError::Parse(format!("effect {id} has no script")));
        }

        let width = parsed
            .canvas
            .as_ref()
            .and_then(|c| c.attr("width"))
            .and_then(|w| w.trim().parse::<u32>().ok())
            .filter(|w| *w > 0)
            .unwrap_or(DEFAULT_WIDTH);
        let height = parsed
            .canvas
            .as_ref()
            .and_then(|c| c.attr("height"))
            .and_then(|h| h.trim().parse::<u32>().ok())
            .filter(|h| *h > 0)
            .unwrap_or(DEFAULT_HEIGHT);
        if width > MAX_CANVAS_EDGE || height > MAX_CANVAS_EDGE {
            return Err(HtmlError::Parse(format!(
                "effect {id}: canvas {width}x{height} exceeds {MAX_CANVAS_EDGE} per edge"
            )));
        }
        let canvas_id = parsed
            .canvas
            .as_ref()
            .and_then(|c| c.attr("id"))
            .unwrap_or("canvas")
            .to_string();

        // Build the property definitions.
        let mut properties = Vec::new();
        for meta in &parsed.metas {
            let Some(key) = meta.attr("property") else { continue };
            if key.is_empty() {
                continue;
            }
            let label = meta.attr("label").unwrap_or(key).to_string();
            let kind = property_kind(meta, &label)?;
            properties.push(PropertyDef { key: key.to_string(), label, kind });
        }

        let name = parsed.title.clone().unwrap_or_else(|| id.to_string());
        let description = parsed.description.clone().unwrap_or_default();

        // The defaults are the file's, with the overrides applied on top.
        let mut schema = PropertySchema {
            id: id.to_string(),
            name,
            description,
            // Screen and audio capture are out of scope for HTML effects in v1.
            needs: InputNeeds::default(),
            properties: properties.clone(),
            presets: Vec::new(),
        };
        apply_overrides(&mut schema, default_overrides);

        // The JS engine. `Math.random` is seeded from `seed`.
        let runtime = JsRuntime::new(width, height).map_err(|e| HtmlError::Script(e.to_string()))?;
        runtime
            .context
            .with(|ctx| {
                js::setup(
                    ctx,
                    runtime.host.clone(),
                    runtime.handles.clone(),
                    &canvas_id,
                    width,
                    height,
                    seed,
                )
            })
            .map_err(|e| HtmlError::Script(e.to_string()))?;

        let mut effect = Self {
            schema,
            scripts: parsed.scripts,
            width,
            height,
            canvas_id,
            runtime,
            accumulator: 0.0,
            time: 0.0,
            reported: Vec::new(),
            values: Props::new(),
            loaded: false,
            blit_key: (0, 0, 0, 0),
            blit_cols: Vec::new(),
            blit_rows: Vec::new(),
        };

        // Run the script now so a broken script is reported at load time, as the brief
        // asks. The property globals are installed first, then re-installed after the
        // script's top-level code has run.
        let defaults = effect.schema.defaults();
        effect.install_globals(&defaults)?;
        effect.run_scripts()?;
        effect.install_globals(&defaults)?;
        effect.values = defaults;
        effect.loaded = true;

        Ok(effect)
    }

    /// The canvas' logical size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The canvas element's id.
    pub fn canvas_id(&self) -> &str {
        &self.canvas_id
    }

    /// Installs the property values as globals.
    fn install_globals(&self, props: &Props) -> Result<(), HtmlError> {
        // The values are collected first, then written inside the context closure.
        let values: Vec<(String, PropValue)> = self
            .schema
            .properties
            .iter()
            .map(|def| {
                let value = props
                    .get(&def.key)
                    .cloned()
                    .unwrap_or_else(|| def.kind.default_value());
                (def.key.clone(), value)
            })
            .collect();
        self.runtime.context.with(|ctx| {
            let globals = ctx.globals();
            for (key, value) in &values {
                // SignalRGB's convention: a color is a "#rrggbb" string, a number is a
                // number, a boolean is a boolean and a choice is its string.
                match value {
                    PropValue::Color(c) => {
                        let text: std::string::String = c.to_hex();
                        globals
                            .set(key.as_str(), text)
                            .map_err(|e| HtmlError::Script(format!("cannot set {key}: {e}")))?;
                    }
                    PropValue::Number(n) => {
                        globals
                            .set(key.as_str(), *n)
                            .map_err(|e| HtmlError::Script(format!("cannot set {key}: {e}")))?;
                    }
                    PropValue::Bool(b) => {
                        globals
                            .set(key.as_str(), *b)
                            .map_err(|e| HtmlError::Script(format!("cannot set {key}: {e}")))?;
                    }
                    PropValue::Choice(s) => {
                        globals
                            .set(key.as_str(), s.clone())
                            .map_err(|e| HtmlError::Script(format!("cannot set {key}: {e}")))?;
                    }
                    PropValue::Hue(h) => {
                        globals
                            .set(key.as_str(), *h)
                            .map_err(|e| HtmlError::Script(format!("cannot set {key}: {e}")))?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Runs the script bodies in order.
    fn run_scripts(&mut self) -> Result<(), HtmlError> {
        let scripts = self.scripts.clone();
        for (i, body) in scripts.into_iter().enumerate() {
            // Browsers run page scripts in sloppy mode unless they say "use strict"; SignalRGB
            // effects rely on that (Poison assigns an undeclared `i` in a `for`). rquickjs
            // defaults to strict, so the option is turned off here.
            let result = self.runtime.context.with(|ctx| {
                let mut options = rquickjs::context::EvalOptions::default();
                options.strict = false;
                ctx.eval_with_options::<(), _>(body.as_bytes(), options)
            });
            if let Err(e) = result {
                // The exception message is on the context; turn it into a message.
                let message = self
                    .runtime
                    .context
                    .with(|ctx| ctx.catch().as_exception().map(|ex| ex.to_string()));
                return Err(HtmlError::Script(match message {
                    Some(m) => format!("script {i}: {m}"),
                    None => format!("script {i}: {e}"),
                }));
            }
        }
        Ok(())
    }

    /// Runs one animation frame: calls the stored `requestAnimationFrame` callback once
    /// and drains the zero-delay timer queue.
    fn run_frame(&mut self) -> Result<(), HtmlError> {
        // The callback and the queued timers live in JavaScript globals, so QuickJS
        // owns them and their lifetime matches the runtime's.
        let timestamp = self.time * 1000.0;
        let result = self.runtime.context.with(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();

            // Take the callback out of its slot: it is called once per frame, and the
            // script re-registers it.
            let callback = globals.get::<_, Value<'_>>(js::RAF_SLOT)?;
            globals.set(js::RAF_SLOT, Value::new_null(ctx.clone()))?;
            if let Some(func) = callback.as_function() {
                // The callback receives the frame timestamp in milliseconds.
                func.call::<_, Value<'_>>((timestamp,))?;
            }

            // Drain the zero-delay timers. The queue is swapped for an empty one first, so a
            // timer that queues another runs in the next frame rather than being dropped.
            let timers = globals.get::<_, rquickjs::Array<'_>>(js::TIMEOUT_SLOT)?;
            let count = timers.len();
            // No queued timers (the usual case): nothing to swap, so no allocation.
            if count == 0 {
                return Ok(());
            }
            globals.set(js::TIMEOUT_SLOT, rquickjs::Array::new(ctx.clone()))?;
            for i in 0..count {
                let timer = timers.get::<Value<'_>>(i)?;
                if let Some(func) = timer.as_function() {
                    func.call::<_, Value<'_>>(())?;
                }
            }
            Ok(())
        });
        if let Err(e) = result {
            // The exception is already on the context; turn it into a message.
            let message = self
                .runtime
                .context
                .with(|ctx| ctx.catch().as_exception().map(|ex| ex.to_string()));
            return Err(HtmlError::Script(message.unwrap_or_else(|| e.to_string())));
        }
        Ok(())
    }

    /// Reports an error once per distinct message. At most `MAX_REPORTED` distinct messages
    /// are kept, so a script that fails with a changing message cannot grow the list forever.
    fn report(&mut self, error: &HtmlError) {
        let message = error.to_string();
        if self.reported.contains(&message) {
            return;
        }
        tracing::warn!("[effect {}] {message}", self.schema.id);
        if self.reported.len() < MAX_REPORTED {
            self.reported.push(message);
        }
    }

    /// The distinct errors this instance has reported so far. Empty means no script error
    /// has occurred since the instance was created.
    pub fn errors(&self) -> &[String] {
        &self.reported
    }

    /// Copies the rendered pixels into the engine's canvas, resampling if the render
    /// target is a different size than the effect's canvas.
    ///
    /// The raster stores premultiplied alpha. The copied colour is straight (demultiplied),
    /// as `getImageData` reads a browser canvas, so the comparison with the Chromium
    /// reference uses the same convention.
    fn blit(&mut self, canvas: &mut Canvas) {
        let Ok(host) = self.runtime.host.lock() else { return };
        let raster = &host.raster;
        let pixels = raster.pixels();
        let src_w = raster.width() as usize;
        let src_h = raster.height() as usize;
        let dst_w = canvas.width as usize;
        let dst_h = canvas.height as usize;
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 || pixels.len() < src_w * src_h * 4 {
            return;
        }
        if canvas.pixels.len() < dst_w * dst_h {
            return;
        }

        // Nearest-neighbour resampling; the render scale is a factor of 2 or 4 at most,
        // so a box filter would not buy much. The source index of each destination row and
        // column is computed once per size pair, not per pixel.
        let key = (src_w, src_h, dst_w, dst_h);
        if self.blit_key != key {
            self.blit_cols = (0..dst_w).map(|x| (x * src_w / dst_w).min(src_w - 1)).collect();
            self.blit_rows = (0..dst_h).map(|y| (y * src_h / dst_h).min(src_h - 1)).collect();
            self.blit_key = key;
        }
        // A byte-to-float table replaces a division per channel.
        let lut: [f32; 256] = std::array::from_fn(|i| i as f32 * (1.0 / 255.0));
        for (y, &sy) in self.blit_rows.iter().enumerate() {
            let src_row = &pixels[sy * src_w * 4..(sy + 1) * src_w * 4];
            let dst_row = &mut canvas.pixels[y * dst_w..(y + 1) * dst_w];
            for (dst, &sx) in dst_row.iter_mut().zip(&self.blit_cols) {
                let si = sx * 4;
                let a = src_row[si + 3];
                *dst = if a == 255 {
                    [lut[src_row[si] as usize], lut[src_row[si + 1] as usize], lut[src_row[si + 2] as usize]]
                } else if a == 0 {
                    [0.0; 3]
                } else {
                    // The pixmap is premultiplied; a canvas read back (getImageData) is straight.
                    let straight = |v: u8| -> f32 {
                        let a = a as u32;
                        ((v as u32 * 255 + a / 2) / a).min(255) as f32 / 255.0
                    };
                    [straight(src_row[si]), straight(src_row[si + 1]), straight(src_row[si + 2])]
                };
            }
        }
    }
}

/// A seed for one instance's `Math.random`. The clock's nanoseconds are mixed with a
/// process-wide counter, so two instances created back to back never share a seed. The
/// mix is splitmix64.
fn entropy_seed() -> u32 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut x = nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x ^ (x >> 32)) as u32
}

/// Builds a property's kind from its `<meta>` tag.
fn property_kind(
    meta: &parse::MetaTag,
    label: &str,
) -> Result<PropKind, HtmlError> {
    let ty = meta.attr("type").unwrap_or("").trim().to_ascii_lowercase();
    let default = meta.attr("default").unwrap_or("").trim().to_string();
    let number = |v: &str| -> Option<f64> { v.trim().parse::<f64>().ok() };

    match ty.as_str() {
        "color" => {
            let hex = default.strip_prefix('#').unwrap_or(&default);
            let parsed = if hex.len() == 3 {
                // #rgb expands to #rrggbb.
                let mut out = String::new();
                for c in hex.chars() {
                    out.push(c);
                    out.push(c);
                }
                out
            } else {
                hex.to_string()
            };
            let rgb: canvas_proto::Rgb8 = format!("#{parsed}")
                .parse()
                .map_err(|_| HtmlError::Parse(format!("{label}: bad color default {default:?}")))?;
            Ok(PropKind::Color { default: rgb })
        }
        "number" => {
            let min = number(meta.attr("min").unwrap_or("0")).unwrap_or(0.0);
            let max = number(meta.attr("max").unwrap_or("100")).unwrap_or(100.0);
            let step = number(meta.attr("step").unwrap_or("1")).unwrap_or(1.0);
            let default = number(&default).unwrap_or(min);
            Ok(PropKind::Number { min, max, step, default })
        }
        "boolean" => {
            // SignalRGB writes booleans as "true" or "1".
            let default = matches!(default.to_ascii_lowercase().as_str(), "true" | "1");
            Ok(PropKind::Bool { default })
        }
        "combobox" | "list" | "select" => {
            let options: Vec<String> = meta
                .attr("values")
                .unwrap_or("")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if options.is_empty() {
                return Err(HtmlError::Parse(format!("{label}: no values")));
            }
            // The default must be one of the options; otherwise the first one wins.
            let default = if options.contains(&default) {
                default
            } else {
                options[0].clone()
            };
            Ok(PropKind::Choice { options, default })
        }
        "hue" => {
            let default = number(&default).unwrap_or(0.0);
            Ok(PropKind::Hue { default })
        }
        "textfield" | "text" => {
            // A text field is a free-form string; the editor shows it as a choice with
            // the single current value.
            Ok(PropKind::Choice { options: vec![default.clone()], default })
        }
        // An unknown type is treated as a text field rather than failing the load.
        _ => Ok(PropKind::Choice { options: vec![default.clone()], default }),
    }
}

/// Replaces the schema's declared defaults with the overrides.
fn apply_overrides(schema: &mut PropertySchema, overrides: &Props) {
    for def in &mut schema.properties {
        let Some(value) = overrides.get(&def.key) else { continue };
        def.kind = match (&def.kind, value) {
            (PropKind::Color { .. }, PropValue::Color(c)) => PropKind::Color { default: *c },
            (PropKind::Number { min, max, step, .. }, PropValue::Number(n)) => {
                PropKind::Number { min: *min, max: *max, step: *step, default: *n }
            }
            (PropKind::Bool { .. }, PropValue::Bool(b)) => PropKind::Bool { default: *b },
            (PropKind::Choice { options, .. }, PropValue::Choice(s)) => {
                let mut options = options.clone();
                if !options.contains(s) {
                    options.push(s.clone());
                }
                PropKind::Choice { options, default: s.clone() }
            }
            (PropKind::Hue { .. }, PropValue::Hue(h)) => PropKind::Hue { default: *h },
            // A type mismatch keeps the declared kind.
            _ => def.kind.clone(),
        };
    }
}

impl Effect for HtmlEffect {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        // The values are merged over the schema's defaults, so a partial props map keeps
        // the rest at their defaults.
        let mut merged = self.schema.defaults();
        for (key, value) in props {
            merged.insert(key.clone(), value.clone());
        }
        self.values = merged.clone();
        // A props change on a live instance only sets the globals; the script is not
        // reloaded.
        if let Err(e) = self.install_globals(&merged) {
            self.report(&e);
        }
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        let _ = inputs;
        if !self.loaded {
            return;
        }
        self.time += dt as f64;
        if let Ok(mut host) = self.runtime.host.lock() {
            host.now_ms = self.time * 1000.0;
            // The device pixmap follows the render scale: its size is the incoming canvas.
            // The script still sees the file's size; the transform maps between them.
            host.raster.set_device_size(canvas.width, canvas.height);
        }

        // The effect runs at its own rate; the accumulator turns the engine's variable
        // step into whole 60 Hz frames.
        self.accumulator += dt * EFFECT_FPS;
        let mut frames = 0u32;
        while self.accumulator >= 1.0 && frames < MAX_FRAMES_PER_UPDATE {
            self.accumulator -= 1.0;
            frames += 1;
            if let Err(e) = self.run_frame() {
                self.report(&e);
                // Keep the last good frame; the accumulator is reset so a broken script
                // does not spin.
                self.accumulator = 0.0;
                break;
            }
        }
        if self.accumulator > EFFECT_FPS * MAX_FRAMES_PER_UPDATE as f32 {
            self.accumulator = 0.0;
        }

        self.blit(canvas);
    }
}

