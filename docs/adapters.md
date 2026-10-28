# Adding a device

This guide is for a developer adding a device to canvas-rgb. Every claim points at the code it
comes from.

## 1. Pick the trait

All three traits live in `crates/canvas-proto/src/traits.rs`. Pick by what the device has.

| The device | Trait | Template |
|---|---|---|
| Has LEDs you set colour by colour | `DeviceAdapter` | `crates/canvas-adapters/src/royuan.rs` |
| Shows one number (a pump display) | `NumericDisplayAdapter` | `crates/canvas-adapters/src/chione.rs` |
| Has settings and no LEDs of its own (a mouse, a controller) | `PeripheralAdapter` | `crates/canvas-adapters/src/attack_shark_x6/` |

The X6 and the 8BitDo expose their lights as settings (`led-mode`, `led-colour`), not frames,
so they use `PeripheralAdapter`.

The main methods, shortened from `traits.rs`:

```rust
#[async_trait]
pub trait DeviceAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError>;
    /// Puts the device in direct/custom mode if needed and writes all LEDs.
    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError>;
    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError>;
    // plus connect, disconnect, is_connected, set_zone_size, identify, capabilities
}

#[async_trait]
pub trait NumericDisplayAdapter: Send + Sync {
    async fn write_value(&mut self, device: &DeviceId, value: &DisplayValue) -> Result<(), AdapterError>;
    /// How often the device expects a write (the CHIONE: every 250 ms).
    fn write_interval_ms(&self) -> u32;
    // plus name, connect, disconnect, is_connected, discover
}

#[async_trait]
pub trait PeripheralAdapter: Send + Sync {
    async fn describe(&mut self, device: &DeviceId) -> Result<PeripheralSchema, AdapterError>;
    async fn read_settings(&mut self, device: &DeviceId) -> Result<Props, AdapterError>;
    async fn write_settings(&mut self, device: &DeviceId, props: &Props) -> Result<(), AdapterError>;
    /// True when this adapter may send config writes to the hardware.
    fn writes_enabled(&self) -> bool;
    // plus name, connect, disconnect, is_connected, discover
}
```

Return `AdapterError::Unsupported` for anything the device cannot take, and say why.

## 2. The templates

**`royuan.rs`: a native HID keyboard with one LED.** `direct_mode_report()` and `color_report(c)` build the two 65-byte feature reports. `write_at` skips an unchanged colour
for a second (`KEEPALIVE`). A failed send drops the handle, so the next connect re-sends direct
mode. It calls hidapi inline, since one feature report needs no blocking pool.

**`chione.rs`: a numeric display.** It sends only the packets the vendor's own app sends. The
module doc at the top says why: until a hardware test proves otherwise, nothing else goes out.
`write_value` encodes the value before it touches the device, so a value the digits cannot show
is refused with nothing written. A blank is refused too (`Unsupported`), and the caller holds
the last value.

**`attack_shark_x6/`: a peripheral, split into two files.**
- `codec.rs` is pure bytes in, bytes out. It builds the read requests and write layouts, checks
  the checksums and decodes the reports. It has no I/O, so its tests need no device.
- `mod.rs` is the adapter. It finds the mouse, reads settings with the `0xA0` handshake, decodes
  status events, maps numbers to `PropertyDef`s and writes through the gate in section 3.

**`eightbitdo_ultimate2/`: the same split for a controller.** `codec.rs` holds the framing, the
CRC and the profile image. `mod.rs` handles three personalities (XInput, idle receiver, DInput),
each with its own product id, and reports which one the pad is showing.

## 3. The shared pattern

**Transport and Opener traits over hidapi.** Each adapter wraps hidapi in private traits, so
the tests run on a fake. The royuan template:

```rust
trait Transport: Send + Sync {
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String>;
}
trait Opener: Send + Sync {
    fn present(&self) -> bool;
    fn open(&self) -> Result<Option<Box<dyn Transport>>, String>;
}
```

The real `HidOpener` finds the vendor interface by VID, PID, interface number, usage page and
usage. The tests pass a fake opener in (`royuan.rs`, `with_opener`). The X6 tests replace the
`opener` field with a `FakeOpener` and set the `Pace` waits to zero (`attack_shark_x6/mod.rs`,
`fn rig()`). Have the fake record every report, so tests can check the bytes.

**Blocking calls go on the blocking pool.** The X6, the 8BitDo and the CHIONE sit through
waits of 100 ms to several seconds. Their hidapi calls run inside `tokio::task::spawn_blocking`,
and the handle moves into the closure and back out. If the caller drops the future, the handle
drops with it and the adapter reads as disconnected.

**Exact-length feature reports (X6 only).** hidapi pads every feature report to the
collection's `FeatureReportByteLength` before it calls `HidD_SetFeature`. The X6's config reports
are 9 to 59 bytes, so the X6 opens its config collection a second time with `ExactFeature`
(`attack_shark_x6/mod.rs`, around line 423). That type opens a raw handle with `CreateFileW`
and sends each report with `HidD_SetFeature` at the length it was given. Reads stay on hidapi,
because `get_feature_report` does not pad. If hidapi's padding is harmless for your device, use hidapi alone.

**Config writes are off by default (peripherals).** The X6 keeps a `writes_enabled` flag that
starts `false` (`attack_shark_x6/mod.rs`, lines 827 and 852). `write_settings` refuses until a
caller calls `set_writes_enabled(true)` (line 1394). The 8BitDo has the same gate
(`eightbitdo_ultimate2/mod.rs`, line 993 to set it, line 1189 to refuse). The daemon turns the
gate on only when `peripherals.json` says `writes: true` (section 4). Keep the gate.
`DeviceAdapter` and `NumericDisplayAdapter` have no such gate in `traits.rs`.

## 4. Register the adapter in the daemon

The daemon builds its adapters in `crates/canvas-daemon/src/main.rs`, around lines 98 to 120.

**A native LED device.** The daemon sends frames through one `CompositeAdapter`
(`crates/canvas-daemon/src/composite.rs`). It routes by device id prefix and hard-codes one
prefix (`ROYUAN_PREFIX`). A second native LED device means extending `CompositeAdapter` in
every call it routes. If OpenRGB also detects the device, its driver may break the device when
frames stream to it. Then add its HID path to `NOT_DRIVEN` in
`crates/canvas-daemon/src/lib.rs` (line 69), as the keyboard's `VID_3151&PID_4015` is.

**A numeric display.** `main.rs` builds one `ChioneDisplay`, and the app's display hub takes one
adapter (`state.rs`). A second display means changing the hub.

**A peripheral.** Implement the daemon's `Peripheral` trait on your adapter. It is
`PeripheralAdapter` plus `set_config_writes(on)`, in `crates/canvas-daemon/src/peripherals.rs`
(lines 80 to 96). The X6 and 8BitDo impls each call `set_writes_enabled`. Then add the adapter
to the `peripherals` vector in `main.rs` (lines 117 to 120), and add `pub mod yourdevice;` to
`crates/canvas-adapters/src/lib.rs`.

The hub never opens a device on its own. `peripherals.json` holds `PeripheralAccess`
(`reads`, `writes`) per device id, and a device with no entry has both off.

## 5. How the settings reach the editor

A new peripheral needs no UI code: the editor's generic panels (`app/web/src/views/PropertyControls.tsx`)
render every section. The mouse's DPI chips and the controller outline in `PeripheralsView.tsx` are optional
extras for those two devices. Your `describe` returns a `PeripheralSchema`
(`crates/canvas-proto/src/peripheral.rs`): sections, each with a list of `PropertyDef`s, plus
`read_only` and `unreported` lists. Each `PropertyDef` has a key, a label and a `PropKind`
(`crates/canvas-proto/src/props.rs`: `Number { min, max, step, default }`, `Choice`, `Color`,
`Bool`, `Hue`). The X6 builds its DPI stages this way (`attack_shark_x6/mod.rs`, around line 901).

Values travel as a `Props` map keyed by `PropertyDef::key` (`PropValue`). `read_settings` fills
it and `write_settings` takes it back. Use `read_only` for any key the device cannot take yet,
and `unreported` for settings the device holds that you cannot name. The schema also carries
the device's `PeripheralKind` (`Mouse`, `GameController` or `Keyboard`). The spine says the
editor needs no new code to render these (`traits.rs`, doc on `PeripheralAdapter`). This guide
did not read the editor's TypeScript, so check that claim there when you add a new kind.

## 6. Probe the device read-only first

Before any write path works, write a probe in `crates/canvas-adapters/examples/`. The probes
in this repo show the pattern:

- `x6_probe.rs` enumerates the receiver, then reads the mouse's settings once with the `0xA0`
  handshake. It never sends a config report. `--check` enumerates only and never opens the
  device. Run it with `cargo run --release -p canvas-adapters --example x6_probe`.
- `pad_probe.rs` does the same for the 8BitDo. Its profile read sends 36 output reports, which
  need Luke's OK. A write needs `--set-paddles` and `--i-mean-it` together.
- `chione_probe.rs` runs one named step per run (`check`, `query`, `show`, `hold`, `dot`).
  `show`, `hold` and `dot` take `--force`. Start with `check`.
- `royuan_probe.rs` writes colours to a real keyboard, with Luke watching.

Build and run through the lock script, which holds a machine-wide mutex (see `CLAUDE.md`):
`powershell -NoProfile -File tools/with-build-lock.ps1 cargo test --workspace`. Never add `-j`.

## 7. Rules for contributors

From `CLAUDE.md` in the repo root:

- **Test against fakes first.** Every adapter test runs on a fake transport or opener. A test
  must not reach a real device.
- **Never write to a device until a read works.** Write the read path, the decode and a probe
  that reads. Only then add a write, behind the gate in section 3.
- **Real hardware is a batched session with Luke's OK in chat.** Do not run a probe that touches
  a device while Luke is playing or working on that machine.
- **Agents do not edit `canvas-proto`.** The spine holds every name and shape that crosses an
  area boundary. If your adapter needs a change there, bring it back to the main agent.
- **Any new client needs the 9430 check.** No client sends the API token to a listener on
  port 9430 until it has checked that a `canvas-rgbd.exe` owned by the calling user's SID is
  listening there. The adapters do not connect to 9430. A new client that does must run the check.
- **Credit the people whose protocol work you used.** Add a `NOTICE` line and a `docs/thanks.md`
  paragraph. The X6 is credited in `NOTICE` lines 31 to 34 and `docs/thanks.md` lines 24 to 27.
  The 8BitDo is credited in `NOTICE` lines 36 to 40 and `docs/thanks.md` lines 29 to 35.

## Checklist

1. Pick the trait from section 1 and copy the closest template. Keep the codec in its own file.
2. Put hidapi behind a private `Transport` and `Opener`. Write a fake for each.
3. Write tests on the fake for every byte you send. Run them through the lock script.
5. Add the read-only probe in `crates/canvas-adapters/examples/`. Run it with Luke's OK.
6. Add the write path behind the gate. Refuse unknown keys and out-of-range values before a send.
7. Add `pub mod` in `crates/canvas-adapters/src/lib.rs`, and register the adapter in `main.rs`,
   plus `composite.rs` or `peripherals.rs` as section 4 says.
8. If OpenRGB also drives the device, add its HID path to `NOT_DRIVEN`.
9. Add a `NOTICE` entry and a `docs/thanks.md` paragraph for any protocol you learned from others.
10. Confirm the editor shows the new device's schema, with screenshots at 1440 and 390.
