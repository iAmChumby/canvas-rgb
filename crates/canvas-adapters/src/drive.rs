//! The drive rule: which devices the app drives itself, and which every writer must leave alone.
//!
//! The app drives some devices through a native adapter of its own, and OpenRGB also detects the
//! same hardware as a second device. Frames through OpenRGB's driver for these break the device,
//! so the copy is never written to: no frame from the daemon, no light for it, and no boot colour
//! or black paint from the service. Every place that writes to a device asks this one question
//! (#393, #413). It lives here, beside the adapters, because both `canvas-daemon` and `canvas-svc`
//! reach this crate and the answer must be the same for both.

use canvas_proto::DeviceId;

/// Devices the app never writes to, matched on the HID path inside the device id.
/// OpenRGB's driver for each of these breaks the device when it is streamed to:
///
/// - `VID_3151&PID_4015`, the ROYUAN MonsGeek keyboard: frames through OpenRGB blank its
///   lights and stop it typing (lighting session 2, 2026-10-26). OpenRGB's detector for it
///   cannot be turned off in its config, so the app drops OpenRGB's copy and drives the
///   keyboard through the native `royuan` adapter instead (see the daemon's `composite`).
pub const NOT_DRIVEN: &[&str] = &["VID_3151&PID_4015"];

/// True when the app may send frames to this device.
pub fn is_driven(id: &DeviceId) -> bool {
    let id = id.0.to_ascii_uppercase();
    !NOT_DRIVEN.iter().any(|path| id.contains(path))
}

#[cfg(test)]
mod tests {
    use canvas_proto::DeviceId;

    /// The rule the daemon has always applied (#393): the keyboard's OpenRGB copy is never
    /// driven, its native `royuan` id is, and any other device is.
    #[test]
    fn the_monsgeek_keyboard_is_never_driven_and_the_rest_are() {
        let keyboard = DeviceId(
            r"openrgb:HID: \\?\HID#VID_3151&PID_4015&MI_02#9&298b2e71&0&0000#{4d1e55b2}".into(),
        );
        let mic = DeviceId(r"openrgb:HID: \\?\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c".into());
        assert!(!super::is_driven(&keyboard));
        assert!(!super::is_driven(&DeviceId(keyboard.0.to_lowercase())));
        assert!(super::is_driven(&mic));
        assert!(super::is_driven(&DeviceId("openrgb:A1B2C3D4E5F6".into())));
        // The native adapter's id does not carry the HID path, so the keyboard it drives stays.
        assert!(super::is_driven(&DeviceId("royuan:3151:4015".into())));
    }
}
