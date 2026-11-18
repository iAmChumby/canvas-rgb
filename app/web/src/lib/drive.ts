// The daemon's drive rule, mirrored from crates/canvas-adapters/src/drive.rs, so the editor and
// the demo refuse to delete a light on a device the daemon drives just as the daemon does (#414,
// #497). A light on a driven device that is currently unplugged cannot be removed by the daemon —
// it answers 409 — so the editor must not offer Remove for it.
//
// The mirror is tied to the daemon by shared vectors (#501):
// app/web/src/lib/__tests__/drive.vectors.json holds device ids and the daemon's verdicts, printed
// by crates/canvas-adapters/examples/drive_vectors.rs. The editor's test runs `driven` over every
// row; the Rust test fails when the file is stale.

/** HID paths the app never writes to: OpenRGB's driver for each of these breaks the device. */
export const NOT_DRIVEN_DEVICES = ["VID_3151&PID_4015"];

/**
 * Rust's `str::to_ascii_uppercase`, not JavaScript's Unicode `toUpperCase`. The daemon folds only
 * ASCII letters: a dotless i stays a dotless i there, but `toUpperCase()` folds it to `I`, which
 * made this copy call a device "not driven" that the daemon drives (#501).
 */
function asciiUpper(value: string): string {
  return value.replace(/[a-z]/g, (c) => c.toUpperCase());
}

/** True when the app may send frames to this device. */
export function driven(deviceId: string): boolean {
  const upper = asciiUpper(deviceId);
  return !NOT_DRIVEN_DEVICES.some((path) => upper.includes(path));
}
