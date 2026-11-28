//! The pieces of error recovery and reporting that both the daemon and the tray need.
//!
//! One scrubber removes personal names from every string that leaves the app, one bundle writer
//! packages logs and config for a bug report, and the path helpers they both use live here so
//! there is a single implementation. The daemon re-exports [`scrub::Scrubber`] for its older
//! callers; no second copy of any of this code exists.

pub mod bundle;
pub mod paths;
pub mod scrub;
pub mod service_logs;
