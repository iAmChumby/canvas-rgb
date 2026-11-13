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

## 8. ROYUAN MonsGeek (3151:4015): per-key colours

Researched for #311, then checked on Luke's keyboard on 2026-11-13 ("Checked on the device", below). **Only the lines tagged `checked on the device` were run on it.** Every line is tagged
`checked on the device` or `source: <URL>`. A source's claim is the source's, not ours. The reports
are built by `per_key_report`, `per_key_reports` and `user_picture_mode_report` in
`crates/canvas-adapters/src/royuan.rs`, and the fake keyboard in that file's tests decodes them.
`write_frame` sends them for a still, when the effect holds still and the flash's budget allows
(#312).

### What the keyboard can do

- **Host-driven per-key colour at animation speed: no source shows it on stock firmware. Status: not
  evidenced.** The one open-source per-key project for this vendor id says "No such protocol was
  identified" and "The official MonsGeek software never transmitted continuous RGB framebuffer updates"
  (source: https://github.com/Lightning-13/monsgeek-rgb/blob/main/docs/limitations.md, lines 9 to 22; it
  was "verified only with the MonsGeek FUN60 Pro", line 90, whose PIDs 502F and 5026 are in its protocol.md,
  lines 18 to 19).
- **A stored picture: sources show it.** Seven `SET_USERPIC` (`0x0C`) reports upload 128 colours, and light
  mode 13 ("User Picture") shows them. The upload lands in the keyboard's flash (source:
  https://github.com/dniminenn/sharkfin/blob/master/docs/PROTOCOL.md, lines 521 to 558). The vendor's own web
  app offers per-key colour the same way: its light table has `LightUserPicture` with 3 to 5 numbered patterns
  and `LightScreenColor` (source: https://app.monsgeek.com/js/index.1e076e39.js, the strings
  `LightUserPicture` and `LightScreenColor`).
- **The only documented live per-key command is patch-only.** `LED_STREAM` (`0xE8`, 18 keys a report, pages 0
  to 6, `0xFF` commit) is added by a firmware patch and stock firmware does not recognise it (source:
  https://github.com/echtzeit-solutions/monsgeek-akko-linux/blob/master/docs/PROTOCOL.md, lines 559 to 568).
  Flashing firmware is out of scope for canvas-rgb. Whether stock firmware on PID `4015` has another live
  per-key path: UNKNOWN. No source tests this PID.
- **What canvas-rgb sends today is a whole-board colour.** `direct_mode_report` is `SET_LEDPARAM` (`0x07`)
  with light mode `0x15` = 21, "Screen Sync", and `color_report` is `SET_SCREEN_COLOR` (`0x0E`), "Send RGB
  for screen sync mode", one RGB that canvas-rgb uses for the whole board (source: the echtzeit PROTOCOL.md
  above, lines 418 and 685; the checksums recompute in the
  tests `direct_mode_bytes` and `color_bytes_and_checksums`). The sources document no per-key form of `0x0E`.

So the answer from the sources: **the only per-key path documented on stock firmware is a stored picture,
not a stream.** They give a driver no live path to moving per-key lighting, and none of them tested PID `4015`.
This keyboard accepts the picture upload (`checked on the device`, 2026-11-13); one project calls it blocked on
another board (below).

### Family and framing

- USB `3151:4015` is shared by more than 170 boards. Sharkfin's device table (the vendor app's records, as
  sharkfin reads them) gives most of them the `yc500` firmware family and leaves some "unknown"; `royuan.rs`
  talks the yc500 dialect (source:
  https://github.com/dniminenn/sharkfin/blob/master/app/src-tauri/data/devices.json, the records with
  `vendorId` 12625 and `productId` 16405; sharkfin PROTOCOL.md lines 1016 to 1018: "Over a hundred yc500
  boards share `3151:4015`"). This keyboard reports device id 1933 (`checked on the device`, below).
- Vendor interface 2, usage page `0xFFFF`, usage `0x0002`, feature reports with report id 0, 64 bytes of
  payload and 65 on the wire with the id first (source: echtzeit PROTOCOL.md, lines 66 to 85 and 277 to 285).
  `royuan.rs` opens exactly this. That the whole-board colour works on Luke's keyboard is stated in the #311
  issue text; it was not re-run for this section.
- Checksum `0xFF - (sum & 0xFF)`. "Bit7" stores it in payload byte 7 over bytes 0 to 6; "Bit8" in byte 8 over
  bytes 0 to 7 (source: sharkfin PROTOCOL.md, lines 124 to 131; sharkfin
  `app/src-tauri/src/protocol/mod.rs`, lines 16 to 36). The tables below number the 65-byte report, so
  payload byte `n` is report byte `n + 1`.

### The upload: `SET_USERPIC` (0x0C), 7 reports

Source: sharkfin `app/src-tauri/src/protocol/mod.rs` lines 497 to 516 (`userpic_write_packet`) and PROTOCOL.md
lines 521 to 535, whose yc500 column the author marks as seen on a plugged-in board. The layout is the same
on every report except byte 5 (the page) and byte 8 (its checksum).

| Byte | Value | Meaning |
|---|---|---|
| 0 | `0x00` | HID report id |
| 1 | `0x0C` | `SET_USERPIC` |
| 2 | `0x00` | reserved |
| 3 | `0x80` | picture length 384, low byte |
| 4 | `0x01` | picture length 384, high byte |
| 5 | `0..=6` | page |
| 6, 7 | `0x00` | reserved |
| 8 | `0x72 - page` | Bit7 checksum over bytes 1 to 7: `0xFF - (0x0C + 0x80 + 0x01 + page)` |
| 9 to 64 | colours | picture bytes `page * 56 ..`, 56 a page; page 6 carries the last 48, the rest is zero |

The picture is 128 slots, 3 bytes each in R, G, B order, slot `s` at picture bytes `3s .. 3s + 3`. A frame
shorter than 128 colours is padded with black; colours past slot 127 are dropped (the test
`per_key_report_drops_colours_past_the_last_slot`). Slot 9 is
bytes 27 to 29, in page 0. `GET_USERPIC` (`0x8C`) returns stale data and cannot confirm a write (source:
sharkfin PROTOCOL.md, lines 521 and 544). The test `per_key_report_bytes_and_checksum_for_a_known_frame`
pins the bytes for a frame with slot 9 red and slot 40 blue.

**The sources that differ.** The FUN60 Pro notes describe another header, `0C 00 FF page length apply 00
checksum`, with 378 bytes (126 slots), 56 data bytes a report, 42 in the last and the apply bit `1` on report 6
(source: https://github.com/Lightning-13/monsgeek-rgb/blob/main/docs/protocol.md, lines 71 to 142; its
length field is a "chunk length", line 128). Sharkfin
files that shape under the other firmware family, "gen2", and warns that "Sending the yc500 header on gen2 is
a live write" (PROTOCOL.md lines 526 to 533 and 1099 to 1100). Most 4015 records are in sharkfin's yc500 list (115 of 174; the rest are "unknown"),
so **try the sharkfin header first** and the FUN60 Pro shape second, never both in one session.

A third project's upload did nothing on a board with PID `5002` ("every guessed frame is a no-op"). Its script
sent a header it had guessed (`0C page 00 ...`, 18 keys a report), which matches neither source above
(source: https://github.com/roziscoding/akko-5075-rgb/blob/main/userpic.ts, lines 5 and 32, and README.md
lines 349 to 354). Its README concludes "Per-key RGB - BLOCKED on stock firmware. SET_USERPIC (0x0C) is
silently ignored" (lines 349 to 351). Its header was a guess and its board is not a 4015, so this does not
show that a correct upload fails; it is the one source that reports the stored picture not working on stock
firmware, and the device check settles it for this keyboard.

### Showing it: light mode 13

`00 07 0D 03 04 00 00 C8 C8 54` after the upload, built by `user_picture_mode_report`:

| Byte | Value | Meaning |
|---|---|---|
| 1 | `0x07` | `SET_LEDPARAM` |
| 2 | `0x0D` | mode 13, "User Picture" |
| 3 | `0x03` | speed as the yc500 wire counts it (host speed 2) |
| 4 | `0x04` | brightness 4 |
| 5 | `0x00` | 0; the yc500 user picture has no slot byte (sharkfin PROTOCOL.md lines 294 to 296) |
| 6 | `0x00` | R; the app sends 0 here; what the firmware does with it is UNKNOWN |
| 7, 8 | `0xC8 0xC8` | G and B; the app sends `(0, 200, 200)` in this mode |
| 9 | `0x54` | Bit8 checksum over bytes 1 to 8 |

Source: sharkfin `app/src-tauri/examples/paint_probe.rs` lines 62 to 80 and `protocol/mod.rs` lines 885 to
907. The FUN60 Pro packet is `07 0D 04 04 00 00 C8 C8 53`, one speed step different (source: Lightning-13
protocol.md, lines 51 to 67). Sharkfin's yc500 wire speed runs 1 to 5 and its gen2 family's 0 to 4 (PROTOCOL.md
lines 253 to 255); that the FUN60 Pro counts like gen2 is an inference, UNKNOWN. Try the sharkfin one.

After the upload the keyboard stays in mode 13. canvas-rgb's own mode is `direct_mode_report`
(`00 07 15 04 04 07 00 00 00 D4`); sending it hands the board back to canvas-rgb.

### Key map

`checked on the device`: none. `source:` the keymap images sharkfin read and the FUN60 Pro notes.

- **Slots run down each column: slot = `column * 6 + row`**, row 0 at the top, 6 rows, 128 slots (21 full
  columns and 2 more). Source: the keymap matrices in
  https://github.com/dniminenn/sharkfin/blob/master/app/src-tauri/data/matrix-evidence.json for three yc500
  boards (`Common83_3MRGB207`, `Common98_K2406B`, `Common68_MK902_2`), which hold the keycodes in this order;
  and the FUN60 Pro notes, where LED 9 is A and LED 40 is B (Lightning-13
  https://github.com/Lightning-13/monsgeek-rgb/blob/main/docs/led_map.md, lines 41 to 48): `(row 3, col 1)`
  and `(row 4, col 6)` in the table below. The test `slots_run_down_each_column` pins both. The keymap and
  the picture share one slot order ("indexed by matrix slot", sharkfin `protocol/mod.rs` line 497); no source
  tests that on a board with this PID.
- **Which key sits in which slot is per board**, "not this protocol" (sharkfin PROTOCOL.md line 241). The layout
  of this keyboard (the vendor's `keyLayout`) is UNKNOWN: 170 boards share the PID. Reads 1 and 2 below settle it.

For orientation, `Common83_3MRGB207` (device 1812, yc500, 82 keys, none past slot 89), derived from its
firmware keymap; `·` has no key. The 98-key `Common98_K2406B` uses the same grid and adds a numpad in columns
14 to 17.

| row \ col | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | Esc | F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12 | PrtSc | Del |
| 1 | ` | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 0 | - | = | Bksp | Ins |
| 2 | Tab | Q | W | E | R | T | Y | U | I | O | P | [ | ] | \ | PgUp |
| 3 | Caps | A | S | D | F | G | H | J | K | L | ; | ' | · | Enter | PgDn |
| 4 | LShift | · | Z | X | C | V | B | N | M | , | . | / | RShift | Up | End |
| 5 | LCtrl | · | Win | LAlt | · | · | Space | · | · | RAlt | · | RCtrl | Left | Down | Right |

A is slot `1 * 6 + 3 = 9`, B is `6 * 6 + 4 = 40`, Space is `6 * 6 + 5 = 41`. A board with no function row (the
`Common68_MK902_2` matrix) starts its number row at row 0, so the same key sits one row up. Do not reuse this
table for another board.

### Frame rate

- **Live frame rate: none documented on stock firmware; for 4015, UNKNOWN.** The upload is a flash write. In Lightning-13's notes the official software
  "applies changes only after Save to Layer" and continuous writes "produce behavior consistent with repeated
  flash programming" (limitations.md lines 26 to 36).
- **How fast uploads survive** (source: sharkfin PROTOCOL.md lines 547 to 558, a yc500 board, the author's
  measurements): 7 reports in 500 ms killed the control endpoint after about 13 reports; 7 reports every 3 s ran
  42 reports; 2 uploads in 10 s with pages 100 ms apart and a 2 s settle survived. A dead endpoint "does not
  recover on reopen" and needs a USB re-enumeration. Single reports needed about 12 ms between them. The
  author's probe sends the pages 100 ms apart and waits 2 s before the mode report (`paint_probe.rs` lines 62
  to 66).
- **A busy/ready poll** between pages: send `F7`, read the feature report, wait for payload byte 5 to be `1`
  (source: Lightning-13 protocol.md, lines 177 to 209, a FUN60 Pro). Sharkfin does not mention it for yc500.
  Status for 4015: UNKNOWN.
- **If a driver ever uploads:** pages at least 100 ms apart and uploads at least 10 s apart, one picture per
  change a person made, never one per animation frame. These are the cadences that survived, not a measured
  limit; the limit is UNKNOWN.
- **What the adapter does (#312, `crates/canvas-adapters/src/royuan.rs`).** A frame that holds within one
  step per channel for the still window is uploaded; the window is 2 s at first, doubles each time a shown
  picture is abandoned sooner than 30 s after it appeared (up to 10 min), and goes back to 2 s after a
  picture that stayed a minute. A started upload is always finished, so a moving frame never leaves a torn
  picture in the flash: it ends the picture on show instead. The picture already in the flash is shown again
  with light mode 13 and not written again when the new still is within a step of it. At most 4 attempts go
  out in a rolling hour and 24 in a rolling day (about 8.8k a year, against a flash rated near 100k erase
  cycles per sector), and an attempt counts from before its first page, so a keyboard that refuses every
  page is bounded the same way; past the caps a still keeps the whole-board colour until a slot frees.
  An attempt is saved to the file, durably, *before* page 0 goes out: a write that fails stops every upload
  for the rest of the run, because an attempt that is not on the disk must never reach the flash. The saved
  hash is cleared before page 0 and set again once page 6 lands, so a crash half way through the pages never
  shows the old picture on the next start. The attempts (wall-clock Unix milliseconds) and the hash are kept
  in `royuan-stills.json` under the daemon's user directory, so a restart neither resets the caps nor
  rewrites a picture the flash holds. A file that is not there at all (a first run, or one someone deleted)
  counts as unknown picture and waits ten minutes; a file that is there but unparsable, incomplete or
  clock-skewed has lost its history, so no picture is written for a day (closed, not open), and a stamp from
  the past-the-clock future counts as now. An upload abandoned before its last page (a disconnect, a resume,
  an identify) leaves the flash unknown, so the next still writes all seven pages and light mode 13 is never
  sent for a torn one.
- **Mode reports are bounded (#312 review passes 3 and 4).** A report is skipped when the board is already in that
  mode on this connection. Entering light mode 13 is capped at 12 entries in a rolling hour (the stamps are kept in
  the file, so a crash loop cannot exceed it), and that cap is the *only* report a budget refuses: the report that
  takes a picture off the board and the direct report on a (re)connect are always sent, because refusing them is
  what would leave a wrong picture on the board. Past the cap the board stays in direct mode and the whole-board
  colour streams, and no upload starts unless the board is confirmed in direct mode on this connection. What the
  driver knows about the board's mode is only ever "a send returned Ok": every resume makes it unknown and sends the
  mode again (one report per wake, bounded only by the OS's event rate), and so does any mode report whose send
  returned an error, because that report may have landed. The composite reopens a keyboard that keeps dropping on a
  wait that doubles from 5 s to 5 min while it drops within 30 s of connecting, and starts again from 5 s once it
  stays up five minutes. Whether the firmware persists `SET_LEDPARAM` reports (light mode 13 and the direct-mode
  report) to flash is not stated by any source: UNKNOWN. If it does, the entries are the ones that matter, and they
  are capped.
- **The whole-board colour keeps streaming through the pages**, and stops when light mode 13 goes out about
  3 s later. During the pages the board stays lit with the streamed colour (`checked on the device`,
  2026-11-13, below).
- The `0x0E` whole-board colour is not an upload. `royuan_probe` sends it about 30 times a second (a 33 ms
  sleep, `crates/canvas-adapters/examples/royuan_probe.rs`); the adapter resends it at least once a second
  (`KEEPALIVE` in `royuan.rs`). The sources give no rate for it.

### Checked on the device (2026-11-13)

Run with `crates/canvas-adapters/examples/royuan_check.rs` (`read`, `picture`, `restore`), canvas-rgb's output
paused, Luke watching the keyboard.

- **Per-key colour works on this keyboard** (`checked on the device`). The seven `per_key_reports` of a frame
  with slot 9 red and the rest black, 100 ms apart, then light mode 13 after 2 s: only the A key lit red, every
  other key dark (Luke, yes). So the yc500 header above is right for THIS board (device id 1933), and slot 9 is A on
  it. Sharkfin's `devices.json` lists 1933 as `yc3121_m1w_v5_soc` with key layout `Common82_SG9000_v3` (source: the
  devices.json linked above); the other boards sharing 3151:4015 are not checked.
- **Device id** (`8F`): reply `8F 8D 07 00 00`, id 1933 (`0x078D`) (`checked on the device`).
- **Light mode** (`87`): reply `87 15 04 04 07`, mode 21 (Screen Sync), canvas-rgb's own mode, read before the
  upload and again after the first restore (`checked on the device`).
- **The key map** (`89`, pages 0 to 8): the reply's byte 0 is the report id; from byte 1, 4 bytes a slot with the
  HID keycode in the slot's byte 2, 16 slots a page (page p holds slots 16p to 16p + 15); pages 6 to 8 are empty.
  Slot = `column * 6 + row` holds (`checked on the device`). Column 15 holds the HID consumer codes `E9`/`EA`
  (volume up and down), which suggests a knob; `0A01` (row 5, column 10) where an Fn key usually sits and `0A0E`
  (row 0, column 1) are vendor codes. The knob and the two vendor codes' functions are UNKNOWN (inferred, not
  checked). The map carries both the ISO keys (`ISO #`, `ISO \`) and the ANSI Backslash:
  which caps the board has fitted is UNKNOWN from the map alone.

| row \ col | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | Esc | `0A0E0000` | F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12 | Del |  |  |  |  |  |  |  |
| 1 | ` | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 0 | - | = | Bksp | Home |  |  |  |  |  |  |  |
| 2 | Tab | Q | W | E | R | T | Y | U | I | O | P | [ | ] | Backslash | PgUp |  |  |  |  |  |  |  |
| 3 | Caps | A | S | D | F | G | H | J | K | L | ; | ' | ISO # | Enter | PgDn |  |  |  |  |  |  |  |
| 4 | LShift | ISO \ | Z | X | C | V | B | N | M | , | . | / | RShift | Up | End | `0300EA00` |  |  |  |  |  |  |
| 5 | LCtrl |  | LWin | LAlt |  |  | Space |  |  | RAlt | `0A010000` | RCtrl | Left | Down | Right | `0300E900` |  |  |  |  |  |  |

- **The whole-board colour must be streamed** (`checked on the device`). One `0x0E` report sent 100 ms after the
  direct-mode report was not shown (the board stayed dark); the same report sent every 33 ms for 2 s was (white,
  then black). The adapter streams at the engine's rate and resends at least every second, so canvas-rgb
  should be unaffected (UNKNOWN: not run on the device with the adapter itself); a one-shot tool must stream.
- **The adapter itself, end to end** (`checked on the device`, 2026-11-13, PR #326's `RoyuanAdapter` driven by a
  scratch example at 30 fps, still counter in a throwaway file): a still horizontal rainbow streamed as one colour,
  one upload went out after the still window (all seven pages acknowledged, the picture hash stored), the board
  stayed lit through the pages, and light mode 13 then showed the rainbow left to right (Luke, yes). When the frame
  changed (solid red, green, blue, one second each) the board left the picture and followed the colour (Luke, yes);
  the light mode read after a run that ended mid-movement was `0x15` (direct). Green then blue as whole-board
  colours showed green then blue (Luke, yes), so `0x0E` takes R, G, B in that order. A moving rainbow shows as one
  nearly constant colour in direct mode (its mean barely changes), which is easy to mistake for the picture.
- **Restore** (`checked on the device`): direct mode, then the streamed whole-board colour: the board followed
  white then black (Luke, yes). The mode read after the restore was taken after the first (single-report)
  restore, not after the streamed one.

### Device check for Luke (run 2026-11-13; kept for a re-run)

Run on 2026-11-13 (results above) with Luke's OK. A re-run needs his OK in chat again and the real-hardware rules
in `CLAUDE.md`.

**Read only. An agent runs these and decodes the answers; Luke judges nothing.** The probe
(`crates/canvas-adapters/examples/royuan_check.rs`) reads with hidapi's `get_feature_report`; the adapter's
`Transport` has no get-feature call.

1. Which board is it? Send `00 8F 00 00 00 00 00 00 70` and read the feature report; the reply is `8F` then
   the device id as a 32-bit little-endian number (source: sharkfin PROTOCOL.md line 169). Look the id up in
   the `devices.json` linked above for the `keyLayout`. An id missing from the file leaves the map UNKNOWN.
2. The board's own key map. `00 89 00 PP 00 00 00 00 CK` for `PP` = 0 to 8, `CK = 0xFF - (0x89 + PP)`, and read
   each reply: 64 raw bytes, 4 per key, the keycode at offset 2 (source: sharkfin `protocol/cmd.rs` line 24;
   PROTOCOL.md line 42, "bulk page reads ... return raw page bytes"). Slot `s` is entry `s`; lay the keycodes
   out as the table above. Settles the key map for this board.
3. The current light mode, so it can be put back: `00 87 00 00 00 00 00 00 78`, read.

**One yes/no for Luke, who is the only one who sees the keys.** It overwrites the keyboard's stored User
Picture slot 0 (sharkfin's own probe says it "overwrites what the board holds"). Send, 100 ms apart (the gaps are
the probe's, no source guarantees them):

1. The seven `per_key_reports` of a frame with slot 9 red and every other slot black. Page 0 starts
   `00 0C 00 80 01 00 00 00 72`, its 56 data bytes hold `FF 00 00` at data offsets 27 to 29, and the rest of
   the bytes are the ones asserted in `per_key_report_bytes_and_checksum_for_a_known_frame`.
2. Wait 2 s, then `00 07 0D 03 04 00 00 C8 C8 54` (`user_picture_mode_report`).
3. Question: **is only the A key lit red, with every other key dark?** Yes confirms the yc500 header and the
   slot rule. No: stop, restore, and report what lit instead. Never try the FUN60 Pro header in the same
   session.
4. Restore with `00 07 15 04 04 07 00 00 00 D4` (`direct_mode_report`), then check the board follows a
   whole-board colour again. A picture Luke wants back comes from the vendor's app.

**Do not test frame rate.** In sharkfin's test a fast upload loop killed a keyboard's control endpoint until a
re-plug.

**The upload window (#312).** No agent has watched a picture upload with the adapter running, so what the board
shows during the ~3 s of pages is UNKNOWN. With canvas-rgb running and an effect that holds still, change
something so a new picture is written (switch to a still effect, or move the layout), then leave the frame
alone: the adapter writes the seven pages about three seconds after the effect settles. One yes/no for Luke,
who sees the keys: **while the new picture is being written, does the keyboard stay lit with the moving
colours instead of going dark or flickering?** Yes: the whole-board colour does cover the window. No: report
what it did, because the adapter may have to hold the last colour instead.

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
