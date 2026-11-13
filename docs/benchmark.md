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

Method: `crates/canvas-core/src/scene_bench.rs`, an ignored test, run through the build lock:
`cargo test -p canvas-core --release -- --ignored --nocapture --test-threads=1 bench_scene_groups`

One scene frame: every group runs its own effect on its own 320x200 canvas, then 60 LEDs are sampled
from that canvas (area average, the most expensive of the three sampling modes, so these numbers are an
upper bound on the sampling cost). 30 warm-up frames, then 300 timed frames. The times below are for the
whole frame, every group together. The built-in effect is `aurora`; the HTML effect is the daemon's own
stock `ink-drop.html`, one QuickJS runtime per group. An HTML effect runs its script at 60 Hz, so at
30 fps each HTML group runs two script frames per engine frame, which is why the 60 fps column is about
half the 30 fps one for HTML.

| Groups | Effect mix | mean ms (30 fps) | p95 ms (30 fps) | mean ms (60 fps) | p95 ms (60 fps) |
|---|---|---|---|---|---|
| 1 | all built-in | 0.425 | 0.502 | 0.414 | 0.481 |
| 1 | all HTML | 4.820 | 5.756 | 2.341 | 2.644 |
| 1 | half built-in, half HTML | 0.416 | 0.535 | 0.412 | 0.430 |
| 8 | all built-in | 3.311 | 3.876 | 3.285 | 3.918 |
| 8 | all HTML | 38.280 | 44.242 | 19.268 | 22.208 |
| 8 | half built-in, half HTML | 21.216 | 25.531 | 11.420 | 13.019 |
| 20 | all built-in | 8.401 | 9.347 | 8.274 | 9.319 |
| 20 | all HTML | 94.225 | 108.035 | 48.574 | 55.043 |
| 20 | half built-in, half HTML | 52.301 | 59.457 | 28.219 | 31.382 |

The mixed compositions, measured separately, because the limit on HTML groups per scene has to cover
them:

| Groups | Effect mix | mean ms (30 fps) | p95 ms (30 fps) | mean ms (60 fps) | p95 ms (60 fps) |
|---|---|---|---|---|---|
| 20 | 1 HTML + 19 built-in | 12.595 | 13.796 | 10.327 | 11.283 |
| 20 | 2 HTML + 18 built-in | 16.777 | 18.505 | 12.189 | 13.446 |

### Against the bar

The bar for 20 groups is a p95 of 16.6 ms (half a 30 fps frame).

| 20 groups | p95 at 30 fps | Result |
|---|---|---|
| all built-in | 9.347 ms | pass |
| 1 HTML + 19 built-in | 13.796 ms | pass |
| 2 HTML + 18 built-in | 18.505 ms | **fail** |
| half built-in, half HTML | 59.457 ms | **fail** |
| all HTML | 108.035 ms | **fail** |

A scene of built-in groups fits with room to spare. A scene with HTML groups in it misses the bar from
the second HTML group on: one HTML group alone costs 5.756 ms p95, a third of the whole budget, and the
cost is the QuickJS runtime rather than the pixels.

### The fallback Task 4 has to build

The spec's order, with what each one does to the measured numbers:

- (a) A smaller canvas for a group with 30 LEDs or fewer. The groups measured here hold 60 LEDs, so it
  does not touch them, and it cannot remove a QuickJS runtime from a frame.
- (b) Half frame rate for groups that are not shown in the preview. At best it halves the HTML cost: the
  60 fps row is that same workload with one script frame per group per engine frame, still 55.043 ms for
  20 HTML groups, 3.3x the bar.
- (c) A documented limit on HTML groups per scene. This one restores the bar, and the limit is measured
  rather than guessed.

**Chosen constant: `SCENE_HTML_MAX = 1`** (the most HTML groups one scene may hold, in
`crates/canvas-proto/src/scene.rs`). With one HTML group a scene of 20 groups costs 13.796 ms p95 at
30 fps, inside the bar. Two cost 18.505 ms, outside it. The stack already limits itself to 2 HTML layers
(`STACK_HTML_MAX` in `crates/canvas-proto/src/stack.rs`); a scene gets one, because a scene renders every
group at once.

Raising the limit to 2 needs fallback (a) as well: the 18 built-in groups then have to get about
0.11 ms per group cheaper to fit the bar. That saving is unmeasured until a smaller canvas is measured.

The numbers move a little between runs. The 20-group all-HTML p95 was 107.376, 108.035, 107.933 and
108.487 ms over four runs, the 20-group all-built-in p95 9.347, 9.469, 9.119 and 9.625 ms, the 1 HTML
+ 19 built-in p95 13.796, 14.569 and 15.380 ms, and the 2 HTML + 18 built-in p95 18.505, 18.777 and
18.717 ms. Every pass and fail call above holds in every run. The headroom for the one-HTML-group case is
2.8 ms on an idle machine, so a busy one can push it over the bar.

