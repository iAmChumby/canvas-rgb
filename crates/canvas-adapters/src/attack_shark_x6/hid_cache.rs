//! One HID context per adapter, scanned at most once per [`SCAN_TTL`].
//!
//! `hidapi::HidApi::new()` reads the metadata of every HID device on the machine each time it is
//! called. The adapters called it on every `present()` and every open, so one pass over the
//! Peripherals view scanned the whole machine several times in a row (issue #7). This cache
//! keeps one `HidApi` and rescans it with `refresh_devices()` only when its list is older than
//! [`SCAN_TTL`] or a caller says the list was wrong.
//!
//! What hidapi 2.6.7 says (docs.rs/hidapi/2.6.7/hidapi/struct.HidApi.html, and `src/lib.rs`):
//! - "The `hidapi` C library is lazily initialized when creating the first instance, and never
//!   deinitialized. Therefore, it is allowed to create multiple `HidApi` instances. Each
//!   instance has its own device list cache." The scan is in `HidApi::new()` (it ends with
//!   `add_devices(0, 0)`), so every `new()` is a full scan.
//! - `refresh_devices(&mut self)` is "`reset_devices()` followed by `add_devices(0, 0)`": a full
//!   rescan into the same instance. It needs `&mut`, so the cache keeps the instance behind a
//!   mutex.
//! - `HidApi` holds only a `Vec<DeviceInfo>` of owned data. `open_path` takes `&self` and never
//!   reads the device list, so a list that is a little old can still open a path that is still
//!   valid.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A device list is reused this long. It is longer than one connect (a `present()`, an
/// `open_config()` and an `open_events()` follow each other within milliseconds) and shorter
/// than the daemon's own enumeration cache (5 s), so the list can be at most this stale when
/// the daemon re-enumerates.
pub(crate) const SCAN_TTL: Duration = Duration::from_secs(2);

/// The scanner behind a [`HidCache`]: `hidapi::HidApi` in the product, a counting fake in the
/// tests.
pub(crate) trait Scan: Sized + Send {
    /// The first scan: builds the context.
    fn first() -> Result<Self, String>;
    /// A rescan into the same context.
    fn rescan(&mut self) -> Result<(), String>;
}

impl Scan for hidapi::HidApi {
    fn first() -> Result<Self, String> {
        hidapi::HidApi::new().map_err(|e| e.to_string())
    }

    fn rescan(&mut self) -> Result<(), String> {
        self.refresh_devices().map_err(|e| e.to_string())
    }
}

struct Slot<S> {
    scan: Option<S>,
    at: Option<Instant>,
}

/// A shared, age-limited HID context.
pub(crate) struct HidCache<S: Scan = hidapi::HidApi> {
    slot: Mutex<Slot<S>>,
    ttl: Duration,
}

impl<S: Scan> Default for HidCache<S> {
    fn default() -> Self {
        Self::with_ttl(SCAN_TTL)
    }
}

impl<S: Scan> HidCache<S> {
    /// A cache whose list is reused for `ttl`.
    pub(crate) fn with_ttl(ttl: Duration) -> Self {
        Self { slot: Mutex::new(Slot { scan: None, at: None }), ttl }
    }

    /// Runs `f` on the context, rescanning first when the list is older than the ttl.
    ///
    /// The lock is held while `f` runs, so callers on several threads share one scan.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&S) -> R) -> Result<R, String> {
        self.run(false, f)
    }

    /// Like [`HidCache::with`], but always rescans first: the caller's last answer from the
    /// list was wrong (a path that no longer opens).
    pub(crate) fn with_fresh<R>(&self, f: impl FnOnce(&S) -> R) -> Result<R, String> {
        self.run(true, f)
    }

    fn run<R>(&self, force: bool, f: impl FnOnce(&S) -> R) -> Result<R, String> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        let stale = force || slot.at.is_none_or(|at| at.elapsed() >= self.ttl);
        if stale {
            match slot.scan.as_mut() {
                Some(scan) => scan.rescan()?,
                None => slot.scan = Some(S::first()?),
            }
            slot.at = Some(Instant::now());
        }
        match slot.scan.as_ref() {
            Some(scan) => Ok(f(scan)),
            None => Err("the HID context is not there".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SCANS: AtomicU32 = AtomicU32::new(0);

    /// A scanner that counts its scans. The tests share the counter, so each one takes the
    /// lock first.
    struct Counting;

    impl Scan for Counting {
        fn first() -> Result<Self, String> {
            SCANS.fetch_add(1, Ordering::SeqCst);
            Ok(Counting)
        }

        fn rescan(&mut self) -> Result<(), String> {
            SCANS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    static SERIAL: Mutex<()> = Mutex::new(());

    fn counted(f: impl FnOnce()) -> u32 {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        SCANS.store(0, Ordering::SeqCst);
        f();
        SCANS.load(Ordering::SeqCst)
    }

    #[test]
    fn many_calls_inside_the_ttl_scan_once() {
        let n = counted(|| {
            let cache = HidCache::<Counting>::with_ttl(Duration::from_secs(60));
            for _ in 0..10 {
                cache.with(|_| ()).unwrap();
            }
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn a_call_after_the_ttl_rescans() {
        let n = counted(|| {
            let cache = HidCache::<Counting>::with_ttl(Duration::ZERO);
            cache.with(|_| ()).unwrap();
            cache.with(|_| ()).unwrap();
        });
        assert_eq!(n, 2);
    }

    #[test]
    fn a_fresh_call_rescans_inside_the_ttl() {
        let n = counted(|| {
            let cache = HidCache::<Counting>::with_ttl(Duration::from_secs(60));
            cache.with(|_| ()).unwrap();
            cache.with_fresh(|_| ()).unwrap();
        });
        assert_eq!(n, 2);
    }
}
