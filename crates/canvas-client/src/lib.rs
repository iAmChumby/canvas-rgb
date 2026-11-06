//! Shared client code for talking to the daemon's loopback API.
//!
//! A client (the tray today, a future GPUI editor tomorrow) must not hand the API token to
//! whatever answers on the daemon's port: a local process that binds the port first could
//! collect it. Two pieces make that safe, and both live here so every client uses the exact
//! same code:
//!
//! - [`peer`]: who holds the port — every listener must be `canvas-rgbd.exe` running as the
//!   calling user's SID.
//! - [`token`]: reading the token file, only once the peer check has passed.

pub mod peer;
pub mod token;
