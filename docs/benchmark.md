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

## Scenes: cost per group (2026-11-12 and 2026-11-14, GAMING, Ryzen 7 7800X3D, 8 cores / 16 threads)

Method: `crates/canvas-core/src/scene_bench.rs`, ignored tests, run through the build lock:
`cargo test -p canvas-core --release -- --ignored --nocapture --test-threads=1 bench_scene`

One scene frame: every group runs its own effect on its own canvas, then its LEDs are sampled from that
canvas (area average; whether it is the costliest of the three sampling modes for a scene is unknown).
30 warm-up frames, then 300 timed frames (900 in the stock-effect table, three runs each). The times
are every group's effect update and LED sampling together; the rest of the engine's frame (brightness,
warmth, white balance, device frames) is not timed here (Task 4 Step 5 timed it: see "The whole frame
through `CoreEngine`"). The built-in effect is `aurora`; the HTML effect is the daemon's own stock file,
one QuickJS runtime per group. An HTML effect runs its script at 60 Hz, so at 30 fps each HTML group
runs two script frames per engine frame, which is why the 60 fps column is about half the 30 fps one for
HTML.

Two clocks are measured. The tables below step every effect by a fixed 1/30 s, which is the engine's
step on a quiet machine. The "engine's clock" table steps every effect by the real time between two
frame starts, on the engine's own tick schedule (`crates/canvas-core/src/engine.rs`, the render loop,
and `crate::timer::FrameTimer`), which is what the engine does and what carries the timer jitter.

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

### 20 groups, k of them HTML: every stock effect

The limit on HTML groups per scene has to hold for every stock effect, not for `ink-drop` alone, so
this table measures 20 groups with `k` of them HTML and 20-k running `aurora`, for `k` from 0 to 4
with no early stop, for each of the three stock effects and for the worst case (two different
effects in one scene, the two with the highest p95: `wet-smear` and `palette-knife`, in turn), at the
two scales the decision uses.

Method: `bench_scene_stock_k_table` (quarter and half scale), 900 frames, release, through the build
lock. 900 frames is 30 s of scene time at 30 fps, which is what Palette Knife's 24 s cycle needs to
be inside the window (`crates/canvas-daemon/stock-effects/palette-knife.html`: one cycle is
`3 * (HOLD + TRANS)` T-units and the clock runs `sp / 60` a frame). The table is run three times and
no row is averaged: a row is a measurement only if every run agrees. Every row prints how many HTML
groups dropped a frame (a script that ran past the runtime's own budget): 0 in every row of every run.

p95 in ms, the three runs of 2026-11-14 in order, the bar is 16.6 ms:

| Scale | Mix | 0 HTML | 1 HTML | 2 HTML | 3 HTML | 4 HTML | largest k at or under 16.6 ms in all three runs |
|---|---|---|---|---|---|---|---|
| 0.25 | `ink-drop` | 0.634 / 0.642 / 0.631 | 4.993 / 5.079 / 5.027 | 9.314 / 9.258 / 9.315 | 13.476 / 13.525 / 13.587 | 17.905 / 17.812 / 17.934 | **3** |
| 0.25 | `palette-knife` | 0.560 / 0.566 / 0.574 | 7.700 / 7.589 / 7.652 | 14.699 / 14.970 / 15.147 | 22.093 / 21.949 / 22.268 | 29.112 / 29.492 / 29.640 | **2** |
| 0.25 | `wet-smear` | 0.598 / 0.604 / 0.575 | 7.051 / 7.018 / 7.145 | 13.462 / 13.333 / 13.524 | 19.823 / 20.018 / 19.696 | 26.195 / 26.425 / 26.681 | **2** |
| 0.25 | `wet-smear` + `palette-knife` | 0.572 / 0.572 / 0.623 | 7.093 / 7.273 / 7.370 | 12.579 / 12.811 / 13.139 | 18.606 / 17.927 / 18.049 | 24.803 / 24.985 / 24.874 | **2** |
| 0.5 | `ink-drop` | 2.151 / 2.336 / 2.262 | 6.531 / 6.604 / 6.405 | 10.923 / 10.552 / 11.019 | 15.165 / 14.861 / 15.277 | 19.479 / 19.261 / 20.114 | **3** |
| 0.5 | `palette-knife` | 2.261 / 2.102 / 2.215 | 9.034 / 9.064 / 9.280 | 16.339 / 15.940 / 16.369 | 27.646 / 23.125 / 23.725 | 31.137 / 31.492 / 30.832 | **2** |
| 0.5 | `wet-smear` | 2.220 / 2.158 / 2.119 | 8.461 / 8.650 / 8.602 | 15.141 / 15.097 / 15.556 | 21.280 / 21.451 / 21.817 | 28.244 / 28.428 / 28.672 | **2** |
| 0.5 | `wet-smear` + `palette-knife` | 2.253 / 2.278 / 2.192 | 8.264 / 8.334 / 8.756 | 14.160 / 14.372 / 14.329 | 20.042 / 19.667 / 19.466 | 26.777 / 26.891 / 26.665 | **2** |

Every line, verbatim, is in the report of the run (`.build/reports/scenes-t1c.md`, in the main
checkout); the shape of a row is:

```
20 groups, 2 HTML + 18 built-in, palette-knife at scale 0.25: 900 frames -> mean 8.045 ms/frame, p95 14.699 ms/frame, 0 dropped frames
```

**That table is an idle machine, and the limit has to hold on a busy one.** The same rows measured on
2026-11-12, with other work on the machine (about 31% of it busy), read up to 3.9 ms higher: at 0.25,
`palette-knife` at 2 HTML groups read 18.527, 18.205 and 18.187 ms (over the bar in all three runs)
against 14.699, 14.970 and 15.147 ms here, and `ink-drop` at 3 read 18.063, 16.808 and 16.307 ms against
13.476, 13.525 and 13.587 ms; at 0.5, `wet-smear` at 2 read 20.215, 19.436 and 19.185 ms against
15.141, 15.097 and 15.556 ms. A limit that holds only when the machine is idle is not a limit for a
shared PC, so the largest k that holds in every run, idle or busy, is:

| Scale | `ink-drop` | `palette-knife` | `wet-smear` | `wet-smear` + `palette-knife` |
|---|---|---|---|---|
| 0.25 | **2** | **1** | **2** | **2** |
| 0.5 | **2** | **1** | **1** | **1** |

**The evidence does not support 2 HTML groups for every stock effect.** `palette-knife` misses the bar
at 2 on a busy machine at 0.25, `wet-smear` misses it at 2 on a busy machine at 0.5, and the whole-frame
run through `CoreEngine` above misses it for both of them at the scene's own scale. The mixed row is
not the worst case: `wet-smear` costs about the same in every frame, while `palette-knife` costs much
more in the 3.2 s of every 8 s cycle in which it lays a new painting over the old one, so mixing the
two spreads the cost over the frames and lowers the peak. `palette-knife` on its own puts that peak
into every frame that costs.

### The same table on the engine's own clock

The table above steps every effect by a fixed 1/30 s. The engine does not: it starts every frame on a
tick schedule and steps the effect by the real time between two frame starts
(`crates/canvas-core/src/engine.rs`, the render loop, and `crate::timer::FrameTimer`, the timer it
sleeps on). That step carries the timer jitter, and the HTML runtime turns the step into whole 60 Hz
script frames (`canvas-html`'s accumulator: `accumulator += dt * 60`, four frames at most), so a step
that runs a little over 1/30 s makes the runtime run a third script frame now and then.

Method: `bench_scene_stock_k_table_engine_clock` (quarter and half scale), 900 frames, one run, release,
through the build lock. The frames are paced by the engine's own timer, so a row takes 30 s of
wall-clock time. p95 in ms, on the same (idle) machine as the table above:

| Scale | Mix | 0 HTML | 1 HTML | 2 HTML | 3 HTML | 4 HTML | largest k at or under 16.6 ms |
|---|---|---|---|---|---|---|---|
| 0.25 | `ink-drop` | 0.671 | 5.467 | 9.753 | 13.962 | 19.085 | **3** |
| 0.25 | `palette-knife` | 0.681 | 8.311 | 15.329 | 22.031 | 29.619 | **2** |
| 0.25 | `wet-smear` | 0.671 | 7.397 | 13.742 | 20.255 | 27.423 | **2** |
| 0.25 | `wet-smear` + `palette-knife` | 0.672 | 7.589 | 13.139 | 18.243 | 24.966 | **2** |
| 0.5 | `ink-drop` | 2.494 | 6.886 | 11.196 | 15.625 | 19.887 | **3** |
| 0.5 | `palette-knife` | 2.364 | 9.771 | 16.720 | 23.917 | 31.106 | **1** |
| 0.5 | `wet-smear` | 2.381 | 8.597 | 14.882 | 21.508 | 27.964 | **2** |
| 0.5 | `wet-smear` + `palette-knife` | 2.255 | 8.896 | 14.499 | 19.789 | 26.270 | **2** |

**The engine's own step costs 0.2 to 0.6 ms a frame more than the fixed step** (the mean of every row
that holds 1, 2 or 3 HTML groups: the HTML runtime runs a third script frame now and then), and it
changes the limit in one cell: `palette-knife` at 0.5 with 2 HTML groups reads 16.720 ms on the
engine's clock against 15.940 to 16.369 ms on the fixed step, which is over the bar. The difference is
inside the table's own run-to-run spread (up to 2.5 ms), so the engine's jitter does not move the
limit, but it does mean a limit set at the bar has no room in it.

The render scale with `ink-drop` alone, 300 frames, one run of 2026-11-14
(`bench_scene_html_mix_by_scale`, which now walks every k from 0 to 4 and stops at nothing, so no cell
is left unmeasured):

| Scale | all built-in p95 | 1 HTML p95 | 2 HTML p95 | 3 HTML p95 | 4 HTML p95 | largest k at or under 16.6 ms |
|---|---|---|---|---|---|---|
| 1.0 | 8.142 | 11.374 | 16.030 | 19.689 | 24.157 | **2** |
| 0.5 | 2.155 | 6.646 | 10.727 | 15.028 | 19.421 | **3** |
| 0.25 | 0.688 | 5.098 | 9.664 | 13.723 | 18.173 | **3** |

The numbers, verbatim, from the run:

```
20 groups, all built-in (aurora) at scale 1   : 300 frames -> mean 7.076 ms/frame, p95 8.142 ms/frame
20 groups, 0 HTML + 20 built-in at scale 1   : 300 frames -> mean 7.067 ms/frame, p95 7.913 ms/frame
20 groups, 1 HTML + 19 built-in at scale 1   : 300 frames -> mean 10.463 ms/frame, p95 11.374 ms/frame
20 groups, 2 HTML + 18 built-in at scale 1   : 300 frames -> mean 14.554 ms/frame, p95 16.030 ms/frame
20 groups, 3 HTML + 17 built-in at scale 1   : 300 frames -> mean 17.764 ms/frame, p95 19.689 ms/frame
20 groups, 4 HTML + 16 built-in at scale 1   : 300 frames -> mean 21.316 ms/frame, p95 24.157 ms/frame
scale 1: the largest k with a p95 at or under 16.6 ms is 2
20 groups, all built-in (aurora) at scale 0.5 : 300 frames -> mean 1.833 ms/frame, p95 2.155 ms/frame
20 groups, 0 HTML + 20 built-in at scale 0.5 : 300 frames -> mean 1.844 ms/frame, p95 2.183 ms/frame
20 groups, 1 HTML + 19 built-in at scale 0.5 : 300 frames -> mean 5.682 ms/frame, p95 6.646 ms/frame
20 groups, 2 HTML + 18 built-in at scale 0.5 : 300 frames -> mean 9.301 ms/frame, p95 10.727 ms/frame
20 groups, 3 HTML + 17 built-in at scale 0.5 : 300 frames -> mean 13.147 ms/frame, p95 15.028 ms/frame
20 groups, 4 HTML + 16 built-in at scale 0.5 : 300 frames -> mean 16.797 ms/frame, p95 19.421 ms/frame
scale 0.5: the largest k with a p95 at or under 16.6 ms is 3
20 groups, all built-in (aurora) at scale 0.25: 300 frames -> mean 0.507 ms/frame, p95 0.688 ms/frame
20 groups, 0 HTML + 20 built-in at scale 0.25: 300 frames -> mean 0.522 ms/frame, p95 0.664 ms/frame
20 groups, 1 HTML + 19 built-in at scale 0.25: 300 frames -> mean 4.373 ms/frame, p95 5.098 ms/frame
20 groups, 2 HTML + 18 built-in at scale 0.25: 300 frames -> mean 8.241 ms/frame, p95 9.664 ms/frame
20 groups, 3 HTML + 17 built-in at scale 0.25: 300 frames -> mean 11.928 ms/frame, p95 13.723 ms/frame
20 groups, 4 HTML + 16 built-in at scale 0.25: 300 frames -> mean 15.856 ms/frame, p95 18.173 ms/frame
scale 0.25: the largest k with a p95 at or under 16.6 ms is 3
```

The render scale is what the built-in groups cost: 20 built-in groups fall from 8.142 ms p95 at 320x200
to 2.155 ms at 160x100 and 0.688 ms at 80x50. A built-in group is a twentieth of the means of those
rows: 0.354 ms (7.076 ms over 20 groups), 0.092 ms (1.833) and 0.025 ms (0.507). It is not what the
HTML groups cost: an HTML group costs the same at every scale.

### The whole frame through `CoreEngine` (Scenes Task 4, Step 5)

Method: `crates/canvas-core/src/scene_frame_bench.rs`, ignored tests, release, through the build lock:
`cargo test -p canvas-core --release -- --ignored --nocapture --test-threads=1 bench_engine_scene_<builtin|ink_drop|palette_knife|wet_smear>`

A spawned `CoreEngine` (`spawn_core`) gets `set_scene` with 20 groups on 20 devices (one 60-LED strip each,
one light per group, a counting sink per device) and its render thread runs at 30 fps. `k` groups play
the stock HTML effect, the other `20 - k` play `aurora`. The global brightness is 80, the warmth 4500 K and
every device has a white balance, so those stages run. The time is the engine's own frame time: the same
`frame_time_ms` that `stats()` reports (frame start to the end of its bookkeeping, the sleep excluded), read
for every frame through a test-only hook in `finish_frame`. One frame covers taking the effects out of the
state, every group's update, sampling, the merge into device frames, brightness/warmth/white balance, the
fade check, packing and every sink's `submit`, with the state lock taken as production takes it. 30 warm-up
frames, then 900 timed frames, three runs per cell. Not in it: the light preview (no sink set), the real
adapters' writer tasks, the HTTP layer.

Dropped frames, two kinds: an HTML group whose script ran past the runtime's own budget
(`canvas_html::FRAME_BUDGET_MS`), read from the effect's flag; and a late frame, one longer than its 33.3 ms
slot at 30 fps. The first kind is 0 in every run below. Late frames are in the verbatim lines.

The machine was in use while this ran (CPU was 33% busy before the first run, and the owner works on it),
so a late frame or a large max in one run is the machine as much as the effect.

Worst p95 of the three runs, in ms (the bar is 16.6 ms):

| Effect | 0 HTML | 1 HTML | 2 HTML | 3 HTML | largest k at or under 16.6 ms p95 in all 3 runs |
|---|---|---|---|---|---|
| `ink-drop` | 1.030 | 7.876 | 16.333 | 22.433 | **2** (0.27 ms under the bar) |
| `palette-knife` | 0.878 | 12.314 | 23.929 | 36.978 | **1** |
| `wet-smear` | 1.302 | 10.491 | 18.350 | 28.141 | **1** |

Every stock effect fits at 1 and none fits at 3. At 2, `ink-drop` fits by 0.27 ms and the other two miss
by 7.3 and 1.8 ms, in all three runs each. The limit on HTML groups per scene is therefore **1**
(`SCENE_HTML_MAX`, unchanged): it would rise to 2 only if every stock effect fit at 2. The `ink-drop` number
is the nearest to the bar and its third run had 5 late frames (max 124.579 ms, a stall of the machine), so
even that row is not a safe 2.

Compared with the update-and-sampling bench above, the engine's whole frame adds little: `ink-drop` at 1
HTML + 19 built-in is 7.027 ms p95 there and 7.789 to 7.876 ms here. The 20 built-in groups alone are
0.70 ms there and 0.84 to 1.30 ms here.

The lines, verbatim (the `ink-drop` lines were printed before the closing line was reworded from "OVER the 16.6 ms bar in all 3 runs", which meant "in at least one run", to the wording of the other two effects; the numbers are unchanged):

```
ink-drop | 20 groups, 0 HTML + 20 built-in | run 1 | 900 frames | mean 0.707 ms | p95 0.846 ms | max 3.610 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 0 HTML + 20 built-in | run 2 | 900 frames | mean 0.707 ms | p95 0.844 ms | max 3.281 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 0 HTML + 20 built-in | run 3 | 900 frames | mean 0.775 ms | p95 1.030 ms | max 4.975 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 0 HTML | worst p95 of 3 runs 1.030 ms | at or under the 16.6 ms bar in all 3 runs
ink-drop | 20 groups, 1 HTML + 19 built-in | run 1 | 900 frames | mean 5.362 ms | p95 7.789 ms | max 15.657 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 1 HTML + 19 built-in | run 2 | 900 frames | mean 5.379 ms | p95 7.876 ms | max 10.897 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 1 HTML + 19 built-in | run 3 | 900 frames | mean 5.406 ms | p95 7.853 ms | max 15.859 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 1 HTML | worst p95 of 3 runs 7.876 ms | at or under the 16.6 ms bar in all 3 runs
ink-drop | 20 groups, 2 HTML + 18 built-in | run 1 | 900 frames | mean 10.314 ms | p95 15.471 ms | max 23.340 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 2 HTML + 18 built-in | run 2 | 900 frames | mean 10.563 ms | p95 16.333 ms | max 18.411 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 2 HTML + 18 built-in | run 3 | 900 frames | mean 10.722 ms | p95 15.584 ms | max 124.579 ms | late frames 5 | HTML groups that dropped a script frame 0
ink-drop | 2 HTML | worst p95 of 3 runs 16.333 ms | at or under the 16.6 ms bar in all 3 runs
ink-drop | 20 groups, 3 HTML + 17 built-in | run 1 | 900 frames | mean 14.398 ms | p95 21.583 ms | max 27.467 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 3 HTML + 17 built-in | run 2 | 900 frames | mean 14.608 ms | p95 22.080 ms | max 28.413 ms | late frames 0 | HTML groups that dropped a script frame 0
ink-drop | 20 groups, 3 HTML + 17 built-in | run 3 | 900 frames | mean 14.724 ms | p95 22.433 ms | max 40.190 ms | late frames 1 | HTML groups that dropped a script frame 0
ink-drop | 3 HTML | worst p95 of 3 runs 22.433 ms | OVER the 16.6 ms bar in all 3 runs
ink-drop | the largest k at or under 16.6 ms p95 in all 3 runs is 2
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 159 filtered out; finished in 372.39s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
palette-knife | 20 groups, 0 HTML + 20 built-in | run 1 | 900 frames | mean 0.715 ms | p95 0.853 ms | max 4.484 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 0 HTML + 20 built-in | run 2 | 900 frames | mean 0.707 ms | p95 0.844 ms | max 1.826 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 0 HTML + 20 built-in | run 3 | 900 frames | mean 0.734 ms | p95 0.878 ms | max 12.165 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 0 HTML | worst p95 of 3 runs 0.878 ms | at or under, in all runs, the 16.6 ms bar
palette-knife | 20 groups, 1 HTML + 19 built-in | run 1 | 900 frames | mean 5.792 ms | p95 11.779 ms | max 17.535 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 1 HTML + 19 built-in | run 2 | 900 frames | mean 5.672 ms | p95 11.805 ms | max 17.709 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 1 HTML + 19 built-in | run 3 | 900 frames | mean 6.103 ms | p95 12.314 ms | max 19.719 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 1 HTML | worst p95 of 3 runs 12.314 ms | at or under, in all runs, the 16.6 ms bar
palette-knife | 20 groups, 2 HTML + 18 built-in | run 1 | 900 frames | mean 11.382 ms | p95 23.929 ms | max 31.976 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 2 HTML + 18 built-in | run 2 | 900 frames | mean 11.424 ms | p95 23.272 ms | max 32.147 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 2 HTML + 18 built-in | run 3 | 900 frames | mean 11.258 ms | p95 23.098 ms | max 31.646 ms | late frames 0 | HTML groups that dropped a script frame 0
palette-knife | 2 HTML | worst p95 of 3 runs 23.929 ms | OVER, in at least one run, the 16.6 ms bar
palette-knife | 20 groups, 3 HTML + 17 built-in | run 1 | 900 frames | mean 17.468 ms | p95 36.978 ms | max 72.270 ms | late frames 56 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 3 HTML + 17 built-in | run 2 | 900 frames | mean 16.674 ms | p95 33.673 ms | max 48.057 ms | late frames 47 | HTML groups that dropped a script frame 0
palette-knife | 20 groups, 3 HTML + 17 built-in | run 3 | 900 frames | mean 16.693 ms | p95 35.534 ms | max 47.862 ms | late frames 52 | HTML groups that dropped a script frame 0
palette-knife | 3 HTML | worst p95 of 3 runs 36.978 ms | OVER, in at least one run, the 16.6 ms bar
palette-knife | the largest k at or under 16.6 ms p95 in all 3 runs is 1
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 159 filtered out; finished in 372.92s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
wet-smear | 20 groups, 0 HTML + 20 built-in | run 1 | 900 frames | mean 0.941 ms | p95 1.302 ms | max 30.860 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 0 HTML + 20 built-in | run 2 | 900 frames | mean 0.721 ms | p95 0.939 ms | max 3.323 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 0 HTML + 20 built-in | run 3 | 900 frames | mean 0.734 ms | p95 0.963 ms | max 6.601 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 0 HTML | worst p95 of 3 runs 1.302 ms | at or under, in all runs, the 16.6 ms bar
wet-smear | 20 groups, 1 HTML + 19 built-in | run 1 | 900 frames | mean 6.850 ms | p95 9.971 ms | max 13.361 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 1 HTML + 19 built-in | run 2 | 900 frames | mean 7.125 ms | p95 10.069 ms | max 13.539 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 1 HTML + 19 built-in | run 3 | 900 frames | mean 6.855 ms | p95 10.491 ms | max 22.243 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 1 HTML | worst p95 of 3 runs 10.491 ms | at or under, in all runs, the 16.6 ms bar
wet-smear | 20 groups, 2 HTML + 18 built-in | run 1 | 900 frames | mean 12.860 ms | p95 18.350 ms | max 29.746 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 2 HTML + 18 built-in | run 2 | 900 frames | mean 12.684 ms | p95 17.986 ms | max 25.633 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 2 HTML + 18 built-in | run 3 | 900 frames | mean 12.901 ms | p95 18.086 ms | max 24.696 ms | late frames 0 | HTML groups that dropped a script frame 0
wet-smear | 2 HTML | worst p95 of 3 runs 18.350 ms | OVER, in at least one run, the 16.6 ms bar
wet-smear | 20 groups, 3 HTML + 17 built-in | run 1 | 900 frames | mean 18.961 ms | p95 27.765 ms | max 36.350 ms | late frames 10 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 3 HTML + 17 built-in | run 2 | 900 frames | mean 18.977 ms | p95 27.588 ms | max 36.918 ms | late frames 11 | HTML groups that dropped a script frame 0
wet-smear | 20 groups, 3 HTML + 17 built-in | run 3 | 900 frames | mean 19.789 ms | p95 28.141 ms | max 113.496 ms | late frames 16 | HTML groups that dropped a script frame 0
wet-smear | 3 HTML | worst p95 of 3 runs 28.141 ms | OVER, in at least one run, the 16.6 ms bar
wet-smear | the largest k at or under 16.6 ms p95 in all 3 runs is 1
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 159 filtered out; finished in 372.46s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

The separate `aurora only` run (`bench_engine_scene_builtin`) also ran. Its first run's line was lost to a
filter, and its second run read p95 8.103 ms with a 41 ms max while the machine was busy; the `0 HTML`
rows above are the three clean readings of the same 20 built-in groups (0.844 to 1.302 ms p95).

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
/// The most HTML groups one scene may hold. 1: Task 4 Step 5 did not prove 2 through the
/// real engine (see "The decision" below).
pub const SCENE_HTML_MAX: usize = 1;

// crates/canvas-core/src/scene_engine.rs (Task 4)
/// The render scale every group's canvas runs at: 0.25 is an 80x50 canvas, the size
/// the stock HTML effects draw at. From the Task 1 benchmark, see "Scenes: cost per
/// group" in `docs/benchmark.md`.
pub const SCENE_RENDER_SCALE: f32 = 0.25;
```

**The decision.** `SCENE_RENDER_SCALE = 0.25` and `SCENE_SAMPLING = AreaAverage`, both fixed in
`scene_engine.rs`. `SCENE_HTML_MAX = 1`: the whole-frame run above puts two of the three stock effects over the bar at 2.

The reason for the scale: an HTML group costs 5.4 to 6.7 ms at every scale, so the scale cannot buy an HTML group back, and 20 built-in groups fall from 8.1 ms p95 at 320x200 to 0.69 ms at 80x50.
The reason for the sampling: area average is what every number here timed, so the numbers hold for it; nearest and bilinear were not timed.

Why 0.25 and not 0.5: the extra pixels of 0.5 buy no group. At 0.25 the two-HTML-group p95, the worst
of three runs, is 9.3 ms (`ink-drop`), 13.5 ms (`wet-smear`), 15.1 ms (`palette-knife`) and 13.1 ms (the
mixed row); at 0.5 the same four are 11.0, 15.6, 16.4 and 14.4 ms. The built-in groups cost about 14
times less at 0.25 than at 1.0.

Why 1 and not 2:
- The whole-frame run through `CoreEngine` with each stock HTML effect (Task 4 Step 5, the section
  above) found `palette-knife` and `wet-smear` over the bar at 2 HTML groups, in all three runs each.
- The group-level table says the same, effect by effect. The largest k with a p95 at or under 16.6 ms
  in every run, idle or busy, is, at 0.25, **2** for `ink-drop`, **1** for `palette-knife`, **2** for
  `wet-smear` and **2** for the mixed row; at 0.5 it is **2** for `ink-drop` and **1** for the other
  three. On an idle machine alone the table would allow 2 for every stock effect at 0.25 and 3 for
  `ink-drop`, but the limit has to hold on a shared machine too.
- The engine's own clock leaves no room either: `palette-knife` at 0.5 with 2 HTML groups reads
  16.720 ms on it, over the bar, against 15.940 to 16.369 ms on the fixed step.
- So the evidence does not support 2 HTML groups for every stock effect, and the limit stays at 1. It
  would rise to 2 on a machine where every stock effect fits at 2 in three clean runs, on the engine's
  own clock, and with room in the bar.

What a scene loses at 0.25, measured and not:

- **An HTML group:** the three stock effects draw an 80x50 picture, and `canvas-html` sizes its raster to
  the canvas (`set_device_size`, `blit`), so at 0.25 the copy is one to one. Whether the picture looks the
  same as at 1.0 is **UNKNOWN**: no pictures were compared.
- **A built-in group draws at 80x50 instead of 320x200.** Every built-in effect is scale-aware
  (`canvas.scale()`) and renders correctly at 0.25 (the effect crate's own test
  `every_effect_renders_finite_values_in_range_at_every_scale`). A smooth gradient looks the same; fine
  detail is lost. How much a given effect loses is **UNKNOWN**: no pictures were compared.
- **The LEDs smooth the picture over their own blocks, at any scale.** A group of 8 to 24 LEDs reads an
  area average of the 80x50 picture, so each LED covers a block of it: a strip of 24 LEDs over 80 picture
  pixels is 3 pixels wide. That is the sampling mode's doing, not the scale's.
- **The editor's preview of a group shows an 80x50 canvas.** How that reads is **UNKNOWN**: not looked at.

The scene's sampling mode: every number above is area average. Whether it is the most expensive of the
three for a scene, and what nearest and bilinear cost, is **UNKNOWN** (not measured).

### The fallback Task 4 has to build

The spec's order, with what each one does to the measured numbers:

- (a) A smaller canvas for a group with 30 LEDs or fewer. It does not touch an HTML group: an HTML group's
  cost is the same at 80x50 as at 320x200. It does help the built-in groups a lot: 20 of them cost
  8.142 ms p95 at 320x200 and 0.688 ms at 80x50. So it is worth doing for the built-in groups and it is
  worth nothing for the HTML ones.
- (b) Half frame rate for groups that are not shown in the preview. At best it halves the HTML cost: the
  60 fps rows are that same workload with one script frame per group per engine frame, still 65.272 ms for
  20 HTML groups.
- (c) A documented limit on HTML groups per scene. This one restores the bar, and the limit is measured
  rather than guessed.

### How the numbers move

The machine is shared, and the HTML group's cost moves with the load on it: the 20-group all-HTML p95 was
108.035 ms on the first bench's run and 126.500 ms on the second, which ran while other work was on the
machine; the 20-group all-built-in p95 was 9.347 ms and 10.150 ms. The mixed case moved less: 13.796 ms
against 15.697 ms for 1 HTML group + 19 built-in at 1.0. Readings from a run spoiled by another build are
left out. One run per case is not enough to call a limit that sits near the bar: Task 4 Step 5 ran three (see "The whole frame through `CoreEngine`").

The stock-effect table ran three times per row, and the three readings of a row differ by 0.05 to 2.5 ms
p95 (at most 1.8 ms for the rows that decide the limit at 0.25: `ink-drop` at 3 HTML groups, 13.476,
13.525 and 13.587 ms). Every row of every run had 0 dropped frames. The bigger movement is the machine:
the same rows read up to 3.9 ms higher on 2026-11-12, with other work on the machine (about 31% of it
busy), than on 2026-11-14 with it idle. The tables above this section are from the first bench's run
(2026-11-12, a busy machine): the 20 built-in groups at 320x200 read 9.189 ms mean and 10.150 ms p95
there, against 7.076 ms and 8.142 ms in the scale table's run on the idle machine.
