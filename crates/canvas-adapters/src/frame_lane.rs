//! The lane the daemon's frame writers use.
//!
//! The daemon holds its adapter behind a lock, and the OpenRGB supervisor holds that lock
//! while it waits for OpenRGB to answer: a discovery, the 5 s refresh, a rescan. A frame
//! written through the lock waits with it, so a stalled OpenRGB freezes the lights (#233).
//!
//! An adapter with an OpenRGB link hands out a second handle on that link. The frame goes out
//! on the socket itself and never waits for the lock. The handle shares the connection with the
//! adapter, so a frame is written on the same socket, in the order it was sent, and a request
//! that is waiting for its reply does not take the frame with it.

use crate::openrgb::OpenRgbAdapter;

/// The contract of an adapter that can write frames without the daemon's adapter lock.
///
/// The daemon asks for this trait wherever it takes an adapter (`App::new`,
/// `App::with_displays`), the same way it asks for [`crate::resume::ResumeBehavior`]: an
/// adapter that does not say whether it has a lane does not compile.
pub trait FrameLane {
    /// A handle on the OpenRGB link this adapter writes frames through, when it has one.
    ///
    /// `None` for an adapter with no OpenRGB link (a fake, the peripherals): its frames go
    /// through the adapter, under the lock.
    fn frame_lane(&self) -> Option<OpenRgbAdapter> {
        None
    }
}

impl FrameLane for OpenRgbAdapter {
    fn frame_lane(&self) -> Option<OpenRgbAdapter> {
        // The clone shares the link: the connection, the socket and the session.
        Some(self.clone())
    }
}
