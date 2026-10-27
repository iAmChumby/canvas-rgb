//! canvas-svc: the LocalSystem service behind `canvas-rgb-svc.exe`. Fans (PawnIO, NCT6687D-R),
//! temperature sources, the 1 Hz curve loop and its safety, the elevated OpenRGB server, daemon
//! supervision, the boot color, and the pipe server the daemon talks to.
//!
//! Everything that touches hardware or processes sits behind a trait (`PortIo`, `FanChip`,
//! `TempReader`, `ProcessSpawner`, `SessionLauncher`), so the test suite runs against fakes.

pub mod app;
pub mod chip;
pub mod daemon;
pub mod fakes;
pub mod fans;
pub mod gpu_fans;
pub mod hardware;
pub mod logging;
pub mod openrgb;
pub mod paths;
pub mod pawnio;
pub mod pipe_server;
pub mod process;
pub mod secure;
pub mod service;
pub mod store;
pub mod temps;
pub mod winsvc;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod fans_tests;
#[cfg(test)]
mod gpu_fans_tests;
#[cfg(test)]
mod openrgb_tests;
