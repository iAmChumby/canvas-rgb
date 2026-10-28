<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.png">
  <img alt="canvas-rgb: lighting, fans and peripherals for your Windows PC. One small app instead of four." src="docs/assets/banner-light.png">
</picture>

[![CI](https://github.com/iAmChumby/canvas-rgb/actions/workflows/ci.yml/badge.svg)](https://github.com/iAmChumby/canvas-rgb/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-3b9bff)](LICENSE)
![Platform: Windows 11](https://img.shields.io/badge/platform-Windows%2011-0e0f11)
![Rust](https://img.shields.io/badge/built%20with-Rust-b45cff?logo=rust)
![Tauri 2](https://img.shields.io/badge/Tauri-2-ffb02e?logo=tauri)
![RAM: 60 MB](https://img.shields.io/badge/RAM-60%20MB-3ddc84)

canvas-rgb paints effects across every LED in your case, runs your fan curves and sets up your mouse and controller. It uses 60 MB of RAM. The apps it replaced used 819 MB. Your SignalRGB effects still work.

| Measured on one PC, 60 seconds per app | RAM | CPU | Disk |
|---|---|---|---|
| **canvas-rgb**: lights, fans, pump display, mouse, controller | **60 MB** | **0.6%** | **80 MB** |
| SignalRGB, ZEUS CAST and Attack Shark X6 app | 819 MB | 2.6% | 2.4 GB |
| SignalRGB alone (its background service was off) | 643 MB | 0.7% | 1.8 GB |

canvas-rgb uses 13.5 times less RAM than the three apps. The 8BitDo software was not measured.
Opening the editor adds about 360 MB while its window is open. The method and the caveats are in
[docs/benchmark.md](docs/benchmark.md).

## Your SignalRGB effects still work

canvas-rgb reads SignalRGB's HTML effect format. An effect that uses the supported calls runs
unchanged. The benchmark ran Poison, a SignalRGB effect, at 60 frames a second.

Copy the effect file into `%APPDATA%\canvas-rgb\effects\`, and the daemon picks it up within five
seconds. Or let canvas-rgb check and copy it with `canvas-rgb effects import my-effect.html`.
[docs/effects.md](docs/effects.md) lists the calls that are supported and the few that are not.

## Lighting

- One layout for every device. Place your devices on it and an effect flows across them as one
  picture.
- 11 built-in effects, from solid colour to screen ambience and audio spectrum.
- Runs SignalRGB-format HTML effects.
- Write your own: an effect is one HTML file. `canvas-rgb effects check my-effect.html` reports
  whether it parses, moves and shows more than one colour, before you install it.

## Fans

- Curves, fixed speeds or BIOS control, per header, with named profiles.
- Every fan goes back to BIOS control when the service shuts down, panics, or loses a temperature
  sensor for too long. No fan is driven below its floor.

## Mouse and controller

- **Attack Shark X6**: DPI, polling rate, lighting and power settings.
- **8BitDo Ultimate 2**: paddle and button mapping, with the stick and trigger ranges.
- Both work without their vendor apps.

## For tinkerers

- A local API on `127.0.0.1:9430`. Every route except the status and the live preview needs the
  API token. The CLI talks to the same API.
- A command line, `canvas-rgb`, with `status`, `devices`, `effects`, `output`, `fans`,
  `peripherals`, `diag` and `config`. Add `--json` for output a script can read.
- New devices are adapters: a Rust module behind the `DeviceAdapter` trait in
  `crates/canvas-proto/src/traits.rs`. How to add one: [docs/adapters.md](docs/adapters.md).
- Effects are one HTML file each. The guide is [docs/effects.md](docs/effects.md). The repository
  includes a Claude Code skill for writing and fixing them, in `.claude/skills/canvas-rgb-effect`.

## Hardware it is tested on

Tested on one PC: an MSI B850 board with Mystic Light ARGB and an NCT6687D fan chip, ENE RAM, a
MonsGeek keyboard, a HyperX QuadCast S, a GAMDIAS CHIONE pump display, an Attack Shark X6 and an
8BitDo Ultimate 2.

Lighting goes through [OpenRGB](https://codeberg.org/OpenRGB/OpenRGB), so OpenRGB-supported devices
should light up. Fan control works on the NCT6687D only.

## Install

Windows 11. Run the plan first. It changes nothing.

```powershell
powershell -ExecutionPolicy Bypass -File installer\setup.ps1 -WhatIf   # prints the plan, changes nothing
powershell -ExecutionPolicy Bypass -File installer\setup.ps1
```

The installer asks for one admin prompt. It builds canvas-rgb and downloads OpenRGB, and it checks
OpenRGB's SHA-256 before it runs anything.

Fan control needs the [PawnIO](https://github.com/namazso/PawnIO) driver, which the installer does
not ship. Pass the PawnIO installer you downloaded with `-PawnIoInstaller`.

A release also ships `canvas-rgb-setup-<version>.exe`, a normal Windows setup wizard (build it with
`tools\build-installer.ps1`; it needs Inno Setup 6). It is not code-signed, so SmartScreen warns on first
run. It runs the same installer scripts, looks for SignalRGB and PawnIO first and only asks about what it finds (or misses), and its
uninstaller hands the fans back to BIOS before it removes any file. Silent switches:
`/SILENT /SIGNALRGB=Y /PAWNIO="C:\path\PawnIO_setup.exe"`.

`installer\uninstall.ps1` hands every fan back to BIOS, then removes the service, the install
folder and the startup entries.

## Why I built this

SignalRGB and ZEUS CAST ran every day, for my lights and my pump display. The 8BitDo and Attack
Shark apps I never really ran, because they suck so much, so the paddles and mouse settings were
out of reach.

My RGB software shouldn't take a gigabyte of RAM on the machine meant to put its resources into
games. So I built one app that does all of it and stays out of the way.

Luke

## Thanks

canvas-rgb stands on other people's work: OpenRGB, Hypercolor, PawnIO, ATKsoft, the 8BitDo Linux
project, SDL and more. [docs/thanks.md](docs/thanks.md) says what each one made possible.

## License

Apache-2.0. [NOTICE](NOTICE) lists the code and research this builds on.
