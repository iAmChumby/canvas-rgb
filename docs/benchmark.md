# canvas-rgb vs the vendor apps it replaces (2026-10-27, Ryzen 7 7800X3D, 16 threads, Windows 11)

Method: `tools/bench.ps1`. Each app's process tree (every process under its install folder or named,
plus all descendants), private memory (WMI PrivatePageCount) sampled every 2 s for 60 s, CPU = kernel+user
time over the window / (60 s x 16 threads). One app at a time; canvas-rgb paused while the others ran.

| App | Job | Processes | RAM avg (MB) | CPU % of machine | Disk (MB) |
|---|---|---|---|---|---|
| **canvas-rgb** 0.1.0 (daemon, service, OpenRGB, tray; editor closed) | lights, fans, pump display, mouse, controller | 5 | **54.9** | **0.49** (30 fps, Rain) | **117.2** |
| SignalRGB 2.5.80 (service disabled, so understated) | lights | 5 | 643.4 | 0.68 | 1842.4 |
| ZEUS CAST 2.4.2.2 | pump display | 1 | 160.9 | 1.95 | 504.2 |
| Attack Shark X6 app | mouse | 1 | 14.6 | 0.00 | 9.4 |
| **The three vendor apps together** | | 7 | **819.0** | **2.63** | **2356.0** |

On this PC, canvas-rgb used **14.9x less RAM**, **5.4x less CPU** and **20.1x less disk** than the three apps it replaces.

canvas-rgb's row is the v0.1.0 release, measured 2026-10-29 right after a reboot, run elevated so the SYSTEM service is counted, at the shipping defaults (30 fps, the built-in Rain effect). The vendor apps' rows are from 2026-10-27.

Caveats (each one favours the vendor apps):
- SignalRGB ran without its background service (disabled since the retirement); normally the service runs too.
- canvas-rgb ran at its default 30 fps; SignalRGB's frame rate was its own default.
- canvas-rgb also does what none of them does here: fan curves (service), mouse and controller settings.
- The 8BitDo Ultimate Software was not measured (I left it out); with it the gap only grows.
- Opening canvas-rgb's editor adds a WebView2 (about 359 MB, measured 2026-10-27 at 17:2x) while the window
  is open; it is released when the window closes (confirmed: no WebView2 child under the tray).
- One 60 s window each; CPU numbers of this size move with background load. RAM is the robust number.

## Scenes: cost per group (2026-11-12, GAMING, Ryzen 7 7800X3D, 8 cores / 16 threads)

Method: `crates/canvas-core/src/scene_bench.rs`, ignored tests, run through the build lock:
`cargo test -p canvas-core --release -- --ignored --nocapture --test-threads=1 bench_scene`

One scene frame: every group runs its own effect on its own canvas, then its LEDs are sampled from that
canvas (area average, the most expensive of the three sampling modes, so these numbers are an upper bound
on the sampling cost). 30 warm-up frames, then 300 timed frames. The times are for the whole frame, every
group together. The built-in effect is `aurora`; the HTML effect is the daemon's own stock file, one QuickJS
runtime per group. An HTML effect runs its script at 60 Hz, so at 30 fps each HTML group runs two script
frames per engine frame, which is why the 60 fps column is about half the 30 fps one for HTML.

The render scale is the engine's own setting (`Canvas::with_scale`): 1.0 is a 320x200 canvas, 0.5 is
160x100 and 0.25 is 80x50. The sample regions move with it, as the engine's own plan does. The three stock
HTML effects declare an 80x50 canvas and draw an 80x50 picture.

### The baseline, at 320x200

| Groups | Effect mix | mean ms (30 fps) | p95 ms (30 fps) | mean ms (60 fps) | p95 ms (60 fps) |
|---|---|---|---|---|---|
| 1 | all built-in | 0.444 | 0.621 | 0.484 | 0.507 |
| 1 | all HTML | 4.993 | 6.042 | 2.901 | 3.471 |
| 1 | half built-in, half HTML | 0.435 | 0.544 | 0.483 | 0.518 |
| 8 | all built-in | 3.484 | 4.161 | 3.805 | 4.379 |
| 8 | all HTML | 44.545 | 50.994 | 22.845 | 26.506 |
| 8 | half built-in, half HTML | 23.978 | 27.321 | 13.222 | 14.946 |
| 20 | all built-in | 9.189 | 10.150 | 9.525 | 10.320 |
| 20 | all HTML | 111.241 | 126.500 | 57.364 | 65.272 |
| 20 | half built-in, half HTML | 60.367 | 68.426 | 33.574 | 37.439 |

The mixed compositions, measured separately, because the limit on HTML groups per scene has to cover
them:

| Groups | Effect mix | mean ms (30 fps) | p95 ms (30 fps) | mean ms (60 fps) | p95 ms (60 fps) |
|---|---|---|---|---|---|
| 20 | 1 HTML + 19 built-in | 14.545 | 15.697 | 11.838 | 12.672 |
| 20 | 2 HTML + 18 built-in | 19.635 | 21.489 | 15.413 | 16.880 |

### One HTML group alone, at each render scale

Each stock effect on its own, at 30 fps:

| Effect | scale 1.0 mean / p95 (ms) | scale 0.5 mean / p95 (ms) | scale 0.25 mean / p95 (ms) |
|---|---|---|---|
| `ink-drop` | 5.718 / 6.925 | 5.628 / 6.730 | 5.429 / 6.587 |
| `palette-knife` | 5.603 / 10.896 | 5.603 / 10.719 | 5.592 / 11.048 |
| `wet-smear` | 6.692 / 7.521 | 6.564 / 7.329 | 6.546 / 7.326 |

**An HTML group costs the same at every render scale.** A quarter of the canvas is a quarter of the
pixels and no less time: the cost is in the script, not in the picture.

### What an HTML group costs: the script against the runtime's floor

`canvas-html` gives no seam for this: `HtmlEffect::update` runs the script and copies the raster into the
engine's canvas in one call, and neither half is timed on its own. So the floor was measured with an
effect whose script draws nothing: the same QuickJS runtime, the same frame call, the same copy of the
raster, no painting. What the stock effect costs on top of that floor is what its script and its pixels
cost. The floor is a lower bound for the copy: a transparent pixel is cheaper to copy than an opaque one.

| Scale | the runtime's floor, mean / p95 (ms) | `ink-drop`, mean / p95 (ms) | the script's own work (the difference), mean / p95 (ms) |
|---|---|---|---|
| 1.0 | 0.050 / 0.063 | 5.569 / 6.476 | 5.519 / 6.414 |
| 0.5 | 0.017 / 0.017 | 5.423 / 6.506 | 5.406 / 6.488 |
| 0.25 | 0.007 / 0.007 | 5.373 / 6.336 | 5.366 / 6.329 |

The runtime's own floor is under 1% of an HTML group: a QuickJS round trip, the timer queue, the copy
of the raster into the canvas and the sampling of the group's 60 LEDs together cost 0.007 to 0.050 ms a
frame. Everything else is the effect's own script. The first bench called this "the QuickJS runtime rather
than the pixels"; measured, it is the script (its JavaScript and the pixels it paints), and the render
scale does not touch it.

### 20 groups, k of them HTML

`k` groups run `ink-drop` and 20-k run `aurora`, at 30 fps. The search walks k up until one k misses the
bar (16.6 ms, half a 30 fps frame), so the largest k that fits is measured and not guessed:

| Scale | all built-in p95 | 1 HTML p95 | 2 HTML p95 | 3 HTML p95 | 4 HTML p95 | largest k at or under 16.6 ms |
|---|---|---|---|---|---|---|
| 1.0 | 10.121 | 15.526 | 21.502 | (misses) | | **1** |
| 0.5 | 2.716 | 8.702 | 14.934 | 20.904 | | **2** |
| 0.25 | 0.698 | 7.027 | 13.221 | 19.763 | | **2** |

All the numbers, verbatim, from the run:

```
20 groups, all built-in (aurora) at scale 1   : 300 frames -> mean 9.448 ms/frame, p95 10.121 ms/frame
20 groups, 1 HTML + 19 built-in at scale 1   : 300 frames -> mean 14.493 ms/frame, p95 15.526 ms/frame
20 groups, 2 HTML + 18 built-in at scale 1   : 300 frames -> mean 19.522 ms/frame, p95 21.502 ms/frame
scale 1: the largest k with a p95 at or under 16.6 ms is 1
20 groups, all built-in (aurora) at scale 0.5 : 300 frames -> mean 2.450 ms/frame, p95 2.716 ms/frame
20 groups, 1 HTML + 19 built-in at scale 0.5 : 300 frames -> mean 7.723 ms/frame, p95 8.702 ms/frame
20 groups, 2 HTML + 18 built-in at scale 0.5 : 300 frames -> mean 13.143 ms/frame, p95 14.934 ms/frame
20 groups, 3 HTML + 17 built-in at scale 0.5 : 300 frames -> mean 18.536 ms/frame, p95 20.904 ms/frame
scale 0.5: the largest k with a p95 at or under 16.6 ms is 2
20 groups, all built-in (aurora) at scale 0.25: 300 frames -> mean 0.647 ms/frame, p95 0.698 ms/frame
20 groups, 1 HTML + 19 built-in at scale 0.25: 300 frames -> mean 6.127 ms/frame, p95 7.027 ms/frame
20 groups, 2 HTML + 18 built-in at scale 0.25: 300 frames -> mean 11.410 ms/frame, p95 13.221 ms/frame
20 groups, 3 HTML + 17 built-in at scale 0.25: 300 frames -> mean 17.068 ms/frame, p95 19.763 ms/frame
scale 0.25: the largest k with a p95 at or under 16.6 ms is 2
```

The render scale is what the built-in groups cost: 20 built-in groups fall from 10.121 ms p95 at 320x200
to 2.716 ms at 160x100 and 0.698 ms at 80x50, a built-in group from 0.459 ms to 0.123 ms to 0.032 ms a
frame. It is not what the HTML groups cost.

### The LED count of a group

At scale 0.25, 12, 24 and 60 LEDs a group, the k-table again:

| LEDs a group | 1 HTML p95 | 2 HTML p95 | 3 HTML p95 | 4 HTML p95 | 6 HTML p95 | 8 HTML p95 |
|---|---|---|---|---|---|---|
| 12 | 6.883 | 13.178 | 19.301 | 25.570 | 37.627 | 50.435 |
| 24 | 7.064 | 13.249 | 19.399 | 25.936 | 37.858 | 50.739 |
| 60 | 7.024 | 13.407 | 21.367 | 25.738 | 38.177 | 50.830 |

The sampling does not decide the frame. 960 more LEDs a scene (60 a group against 12, over 20 groups)
move the p95 by 0.14 ms in the one-HTML-group scene and by 2.07 ms in the three-HTML-group scene, against
one HTML group's 5.4 ms script, and both are inside the run-to-run movement of this machine (the same
workload moved by 0.5 to 3 ms between two runs). A group of 8 to 24 LEDs, a fan ring for instance, costs
the same as a group of 60 as far as the frame is concerned.

### The conclusion: the fixed render scale and the HTML limit

The spec makes the render scale and the sampling one fixed, documented choice for scenes, so the numbers
above decide two constants:

```rust
// crates/canvas-proto/src/scene.rs (the spine, Task 2)
/// The most HTML groups one scene may hold: each is its own script and the slowest
/// group there is. From the Task 1 benchmark, see "Scenes: cost per group" in
/// `docs/benchmark.md`.
pub const SCENE_HTML_MAX: usize = 2;

// crates/canvas-core/src/scene_engine.rs (Task 4)
/// The render scale every group's canvas runs at: 0.25 is an 80x50 canvas, the size
/// the stock HTML effects draw at. From the Task 1 benchmark, see "Scenes: cost per
/// group" in `docs/benchmark.md`.
pub const SCENE_RENDER_SCALE: f32 = 0.25;
```

**`SCENE_RENDER_SCALE = 0.25`, `SCENE_HTML_MAX = 2`.**

Why 0.25 and not 0.5: the HTML limit is 2 at both, so the extra pixels of 0.5 buy no group, and 0.25
leaves more of the bar free (13.221 ms p95 for 2 HTML groups against 14.934 ms at 0.5, of a 16.6 ms bar).
0.25 is also the size the stock HTML effects draw at, so their picture is copied into the canvas one to
one and is never resampled; at 1.0 it is blown up four times to 320x200 and then sampled back down into
the LEDs.

Why 2 and not 3: three HTML groups at 0.25 measured 16.102 ms p95 on a quieter run (inside the bar by
3%) and 19.763 ms on the run above (outside it by 19%). A limit that flips with the machine's load is not
a limit; two measured 11.231 ms and 13.221 ms, inside the bar in both runs.

What a scene loses at 0.25, measured and not:

- **An HTML group loses nothing.** The three stock effects draw an 80x50 picture and the runtime copies
  the raster into the canvas at the canvas's own size (`canvas-html`'s `set_device_size` and `blit`), so
  at 0.25 the picture lands on the canvas one pixel to one.
- **A built-in group draws at 80x50 instead of 320x200.** Every built-in effect is scale-aware
  (`canvas.scale()`) and renders correctly at 0.25 (the effect crate's own test
  `every_effect_renders_finite_values_in_range_at_every_scale`). A smooth gradient looks the same; fine
  detail is lost. How much a given effect loses is **UNKNOWN**: no pictures were compared.
- **The LEDs smooth the picture over their own blocks, at any scale.** A group of 8 to 24 LEDs reads an
  area average of the 80x50 picture, so each LED covers a block of it: a strip of 24 LEDs over 80 picture
  pixels is 3 pixels wide. That is the sampling mode's doing, not the scale's.
- **The editor's preview of a group shows an 80x50 canvas.** How that reads is **UNKNOWN**: not looked at.

The scene's sampling mode: every number above is area average, the most expensive of the three, so the
limit holds for it and for any cheaper choice. Nearest and bilinear were not measured for a scene
(**UNKNOWN**).

### The fallback Task 4 has to build

The spec's order, with what each one does to the measured numbers:

- (a) A smaller canvas for a group with 30 LEDs or fewer. It does not touch an HTML group: an HTML group's
  cost is the same at 80x50 as at 320x200. It does help the built-in groups a lot: 20 of them cost
  10.121 ms p95 at 320x200 and 0.698 ms at 80x50. So it is worth doing for the built-in groups and it is
  worth nothing for the HTML ones.
- (b) Half frame rate for groups that are not shown in the preview. At best it halves the HTML cost: the
  60 fps rows are that same workload with one script frame per group per engine frame, still 65.272 ms for
  20 HTML groups.
- (c) A documented limit on HTML groups per scene. This one restores the bar, and the limit is measured
  rather than guessed.

### How the numbers move

The machine is shared, and the HTML group's cost moves with the load on it: the 20-group all-HTML p95 was
107.376, 108.035, 107.933 and 108.487 ms over the four runs of the first bench, and 126.500 ms on the run
above, which ran while other work was on the machine. The 20-group all-built-in p95 was 9.347, 9.469,
9.119 and 9.625 ms over the first bench's four runs and 10.150 ms on the run above. The mixed case moved
by less: 13.796 ms (the first bench) against 15.697 ms for 1 HTML group + 19 built-in, and 18.505 ms
against 21.489 ms for 2 HTML groups + 18 built-in. Every pass and fail call in this section holds in every
run except one, the 3-HTML-group case at 0.25, which is why the limit is 2 and not 3.
