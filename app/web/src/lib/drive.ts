// The daemon's drive rule, mirrored from crates/canvas-adapters/src/drive.rs, so the editor and
// the demo refuse to delete a light on a device the daemon drives just as the daemon does (#414,
// #497). A light on a driven device that is currently unplugged cannot be removed by the daemon —
// it answers 409 — so the editor must not offer Remove for it.

/** HID paths the app never writes to: OpenRGB's driver for each of these breaks the device. */
export const NOT_DRIVEN_DEVICES = ["VID_3151&PID_4015"];

/** True when the app may send frames to this device. */
export function driven(deviceId: string): boolean {
  const upper = deviceId.toUpperCase();
  return !NOT_DRIVEN_DEVICES.some((path) => upper.includes(path));
}
