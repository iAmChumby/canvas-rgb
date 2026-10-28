# canvas-rgb vs the vendor apps it replaces (2026-10-27, Ryzen 7 7800X3D, 16 threads, Windows 11)

Method: `tools/bench.ps1`. Each app's process tree (every process under its install folder or named,
plus all descendants), private memory (WMI PrivatePageCount) sampled every 2 s for 60 s, CPU = kernel+user
time over the window / (60 s x 16 threads). One app at a time; canvas-rgb paused while the others ran.

| App | Job | Processes | RAM avg (MB) | CPU % of machine | Disk (MB) |
|---|---|---|---|---|---|
| **canvas-rgb** (daemon, service, OpenRGB, tray; editor closed) | lights, fans, pump display, mouse, controller | 5 | **60.5** | **0.63** (60 fps, Poison) | **80.3** |
| SignalRGB 2.5.80 (service disabled, so understated) | lights | 5 | 643.4 | 0.68 | 1842.4 |
| ZEUS CAST 2.4.2.2 | pump display | 1 | 160.9 | 1.95 | 504.2 |
| Attack Shark X6 app | mouse | 1 | 14.6 | 0.00 | 9.4 |
| **The three vendor apps together** | | 7 | **819.0** | **2.63** | **2356.0** |

canvas-rgb uses **13.5x less RAM**, **4.2x less CPU** and **29x less disk** than the three apps it replaces.

Caveats (each one favours the vendor apps):
- SignalRGB ran without its background service (disabled since the retirement); normally the service runs too.
- canvas-rgb ran at 60 fps; SignalRGB's frame rate was its own default.
- canvas-rgb also does what none of them does here: fan curves (service), mouse and controller settings.
- The 8BitDo Ultimate Software was not measured (I left it out); with it the gap only grows.
- Opening canvas-rgb's editor adds a WebView2 (about 359 MB, measured 2026-10-27 at 17:2x) while the window
  is open; it is released when the window closes (confirmed: no WebView2 child under the tray).
- One 60 s window each; CPU numbers of this size move with background load. RAM is the robust number.

