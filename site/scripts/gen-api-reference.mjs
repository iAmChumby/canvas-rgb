// Generates site/content/docs/api-reference.json from the spine and the daemon router.
// No dependencies. Run `node site/scripts/gen-api-reference.mjs` to write the file, or
// `node site/scripts/gen-api-reference.mjs --check` to fail if it is stale or if a route
// in the spine is missing. The same shape is used in site/tests/api-reference.test.mjs.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");
const repo = path.resolve(site, "..");

const API_RS = path.join(repo, "crates", "canvas-proto", "src", "api.rs");
const MOD_RS = path.join(repo, "crates", "canvas-daemon", "src", "api", "mod.rs");
const API_DIR = path.join(repo, "crates", "canvas-daemon", "src", "api");
const DAEMON_SRC = path.join(repo, "crates", "canvas-daemon", "src");
const PROTO_SRC = path.join(repo, "crates", "canvas-proto", "src");
const PROTO_DIR = path.join(repo, "app", "web", "src", "proto");
const OUT = path.join(site, "content", "docs", "api-reference.json");

const METHOD_ORDER = ["GET", "POST", "PUT", "PATCH", "DELETE"];

// The one route written as a literal in the router. It has a key inside the generator, never an
// id in the published reference.
const INLINE_ROUTE_KEY = "DIAG_FAIL_SENSOR";

// Every example names the getting-started check in plain text. The JSON is read on its own,
// so a markdown link here would be a dead link in the reference.
const LISTENER_NOTE = "# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page";

/**
 * Hand-written, one-line descriptions keyed by route constant and method. A route that has no
 * line here fails generation, and a line that restates its method, path or type fails generation
 * too. The handler's own doc comments are useful to the code's reader, but they name Rust types
 * and issue numbers, so they are not public copy.
 */
const DESCRIPTIONS = {
  "STATUS:GET": "Reports the daemon's version, power state, brightness, frame rate, connected devices, active scene and the links to OpenRGB and to the fan and update service.",
  "ABOUT:GET": "Reports the app version, build, Windows edition, service version and how the previous run ended.",
  "ABOUT_REPORT:GET": "Returns the scrubbed support report the editor shows and copies, with the fields the bug report template fills.",
  "SAFE_START:PUT": "Enters or leaves safe start. The daemon answers first, then the service restarts it in the asked mode; asking for the mode already in force changes nothing.",
  "OUTPUT:GET": "Reports the current power state, brightness and warmth of the lighting output.",
  "OUTPUT:PATCH": "Changes the power state, brightness or warmth of the lighting output.",
  "QUIT:POST": "Stops the daemon after applying its exit lighting setting and telling the fan and update service the stop was deliberate.",
  "DEVICES:GET": "Lists the devices the daemon knows, with their zones and LED counts.",
  "DEVICE_IDENTIFY:POST": "Flashes one target LED so the device holding it can be found in the room. An unknown zone or an LED past the end of the zone answers 422.",
  "DEVICE_WHITE_BALANCE:PUT": "Sets the red, green and blue gains of one device. Every gain must be a number from 0 to 1; any gain outside that range answers 422.",
  "DEVICE_WHITE_BALANCE:DELETE": "Resets one device's gains to neutral.",
  "WHITE_BALANCE:GET": "Lists each device's current colour gains.",
  "DEVICE_ZONE:PATCH": "Changes the LED count of one zone on a device. An LED count outside the zone's range or a zone that cannot be resized answers 422.",
  "LIGHTS:GET": "Lists every light with its id, LED count and connection state.",
  "LIGHT:PATCH": "Changes one light's name, arrangement or other saved settings. A name that is empty or longer than the group name limit answers 422; clear a name with null instead.",
  "LIGHT:DELETE": "Removes a leftover light from an older configuration; a light on a device the daemon drives answers 409.",
  "LIGHTS_FLASH:POST": "Flashes the named lights white and leaves every other LED dark.",
  "EFFECTS:GET": "Lists the effects and saved patterns in the library.",
  "EFFECT:GET": "Returns one effect's details, including its properties and presets.",
  "EFFECT:PUT": "Saves an effect's property values, user presets and favourite; the effect must already exist.",
  "EFFECT:DELETE": "Removes one user HTML effect from the library.",
  "EFFECT_ACTIVATE:POST": "Plays an effect on the groups of the active scene, using saved values or a named preset.",
  "EFFECT_DEFAULTS:PATCH": "Changes the starting property values of an effect.",
  "EFFECT_FAVORITE:PUT": "Marks an effect as a favourite or clears the mark.",
  "EFFECT_PRESETS:POST": "Saves a named preset of an effect's property values.",
  "EFFECT_PRESET:PUT": "Replaces or renames one saved preset.",
  "EFFECT_PRESET:DELETE": "Deletes one saved preset.",
  "EFFECT_SOURCE:GET": "Returns the HTML source of one effect and the file name a save would use.",
  "EFFECT_THUMBNAIL:GET": "Returns the effect's thumbnail as bytes: three 16-bit little-endian counts (width, height, frame count) followed by raw RGB frames, rendered without touching the devices. The answer is sent as application/octet-stream.",
  "EFFECT_DRAFT:GET": "Reports the effect draft playing on the lights and its remaining lease.",
  "EFFECT_DRAFT:PUT": "Starts or renews an effect, stack or pattern draft on the lights.",
  "EFFECT_DRAFT:DELETE": "Ends the current effect draft.",
  "EFFECTS_IMPORT:POST": "Validates an effect file and installs it in the library.",
  "EFFECT_PREVIEW:POST": "Renders a draft HTML effect with the given property values.",
  "STACKS:GET": "Lists the saved stacks.",
  "STACK:GET": "Returns one stack and the state of each of its layers.",
  "STACK:PUT": "Creates or replaces a stack.",
  "STACK:DELETE": "Deletes a stack; one a group plays needs ?force=true.",
  "STACK_PREVIEW:PUT": "Starts or updates the stack used by the preview engine, for a group id the active scene has.",
  "STACK_PREVIEW:DELETE": "Stops the stack preview.",
  "PATTERNS:GET": "Lists the saved patterns.",
  "PATTERN:GET": "Returns one pattern's frames and transitions.",
  "PATTERN:PUT": "Creates or updates a pattern.",
  "PATTERN:DELETE": "Deletes a pattern; one a group plays needs ?force=true, and the answer names the scenes that played it.",
  "SCENES:GET": "Lists the scenes and marks the active one.",
  "SCENES:POST": "Creates a scene from the groups in the request.",
  "SCENES:PUT": "Writes several scenes or their order in one save.",
  "SCENE:PUT": "Replaces one scene and its groups.",
  "SCENE:DELETE": "Deletes one scene.",
  "GROUP_BRIGHTNESS:PUT": "Sets the brightness of one group of a scene.",
  "GROUP_MOVE:POST": "Moves or copies one group between two scenes in one save.",
  "GROUP_PROPS:PATCH": "Changes the property values of one group's effect.",
  "SCENE_ACTIVE:PUT": "Makes a scene active, with a fade.",
  "CHAINS:GET": "Lists the resizable headers and how their zones are set up.",
  "CHAIN_SETUP:POST": "Starts a setup session for one header.",
  "CHAIN_SETUP:DELETE": "Ends a header setup, restoring the earlier length and the scene.",
  "CHAIN_SETUP_COMMIT:POST": "Applies the chosen zone sizes to a header and saves them.",
  "CHAIN_SETUP_EMPTY:POST": "Records that nothing is plugged into the header.",
  "CHAIN_SETUP_LIGHT:POST": "Shows the coloured runs used to measure a header.",
  "FANS:GET": "Reports the fan headers, the temperatures and the active profile.",
  "FAN_HEADER:PATCH": "Changes the mode and settings of one fan header. A percent below the configured floor answers 422: the floor is 20 percent, or 30 for the CPU and GPU headers, and a higher value can be configured. `fixed` needs `percent`, `source` needs its fields, and `curve` needs 2 to 8 `points` sorted by temperature.",
  "FAN_HEADER_AVAILABILITY:PUT": "Chooses whether the fan service shows one fan header; null leaves the choice to the board's own table.",
  "FAN_HOT:PUT": "Sets the temperatures at which fans are driven at full duty.",
  "FAN_PROFILE:GET": "Returns the name of the active fan profile.",
  "FAN_PROFILE:PATCH": "Changes the active fan profile.",
  "FAN_PROFILES:PUT": "Creates or replaces a named fan profile. A name in the body that differs from the path, or a value below the configured fan floor, answers 422.",
  "FAN_PROFILES:DELETE": "Deletes a named fan profile.",
  "DISPLAYS:GET": "Lists the numeric displays with their settings and current values.",
  "DISPLAY:PUT": "Changes the settings of one display.",
  "DISPLAY:DELETE": "Removes one display's saved config and closes the device.",
  "DISPLAY_VALUE:PUT": "Sets the number shown on a display whose value comes from this API. A display whose saved source is not the API answers 422.",
  "PERIPHERALS:GET": "Lists the peripherals the adapters see.",
  "PERIPHERAL:GET": "Reports one peripheral's schema and last-read values; ?fresh=true reads the device now.",
  "PERIPHERAL:PATCH": "Changes one peripheral's settings.",
  "PERIPHERAL_ACCESS:PUT": "Turns reads and configuration writes on or off for one peripheral. `writes` without `reads` answers 422, because a write patches what the device last reported.",
  "PERIPHERAL_READ:GET": "Reports the progress of the read running on one peripheral.",
  "AUDIO_SOURCES:GET": "Lists the audio endpoints available as sources and the resolved current choice.",
  "AUDIO_SOURCE:PUT": "Chooses the audio source.",
  "AUDIO_TEST:PUT": "Tests an audio endpoint and issues a lease. Set listen true for an input so the microphone opens, or leave it false to test an output; every PUT issues a lease.",
  "AUDIO_TEST:GET": "Reads and renews an audio test lease with the id the PUT returned; an unknown or expired lease answers 410.",
  "AUDIO_TEST:DELETE": "Ends the audio test lease with the id the PUT returned.",
  "PREVIEW:GET": "Streams the whole rendered picture. Each binary frame holds a 16-bit little-endian width, a 16-bit little-endian height, then width times height RGB triples. With stack=true it streams the stack preview; without it, the running effect.",
  "PREVIEW_LIGHTS:GET": "The opening message is JSON mapping each light index to its id, repeated when the lights change. Each later binary frame holds a 16-bit little-endian light index, a 16-bit little-endian LED count, then that many RGB triples.",
  "PREVIEW_GROUP:GET": "Streams the picture one group draws at the group's brightness. Each binary frame holds a 16-bit little-endian width, a 16-bit little-endian height, then width times height RGB triples.",
  "SETTINGS:GET": "Returns the current settings.",
  "SETTINGS:PUT": "Replaces the settings.",
  "UPDATE_INSTALL:POST": "Installs a signed update package.",
  "CONFIG_EXPORT:GET": "Exports the configuration as one JSON bundle.",
  "CONFIG_IMPORT:POST": "Validates and imports a configuration bundle. The bundle replaces every saved setting, scene, light, fan, display, stack and pattern with the one it carries, so export the current configuration first.",
  "ONBOARDING_COMPLETE:POST": "Records that first-run onboarding has finished.",
  "DISMISS_ZONE_PANEL:POST": "Dismisses the first-run prompt to size the ARGB headers.",
  "FAULT_DISMISS:POST": "Hides one hardware note on every viewer. The note comes back when its cause clears and then returns.",
  "DIAG_READBACK:GET": "Reads the colours the hardware reports back from the devices.",
  "DIAG_FAIL_SENSOR:POST": "Makes a temperature source report a failure, for a safety test. The source is `cpu`, `gpu` or `board:<name>`; any other value answers 422.",
};

/**
 * Hand-written human titles keyed by route path. A title is the short name of a route: one to four
 * words, with the first word capitalised and every other word lower-case. It never quotes a method,
 * a path or a Rust constant name. A route with no title fails generation.
 */
const TITLES = {
  "/api/v1/status": "Status",
  "/api/v1/about": "About",
  "/api/v1/about/report": "About report",
  "/api/v1/safe-start": "Safe start",
  "/api/v1/output": "Output",
  "/api/v1/quit": "Quit",
  "/api/v1/devices": "Devices",
  "/api/v1/devices/{id}/identify": "Identify device",
  "/api/v1/devices/{id}/white-balance": "Device white balance",
  "/api/v1/devices/{id}/zones/{zone}": "Device zone",
  "/api/v1/white-balance": "White balance",
  "/api/v1/lights": "Lights",
  "/api/v1/lights/{id}": "Light",
  "/api/v1/lights/flash": "Flash lights",
  "/api/v1/effects": "Effects",
  "/api/v1/effects/{id}": "Effect",
  "/api/v1/effects/{id}/activate": "Activate effect",
  "/api/v1/effects/{id}/defaults": "Effect defaults",
  "/api/v1/effects/{id}/favorite": "Effect favourite",
  "/api/v1/effects/{id}/presets": "Effect presets",
  "/api/v1/effects/{id}/presets/{name}": "Effect preset",
  "/api/v1/effects/{id}/source": "Effect source",
  "/api/v1/effects/{id}/thumbnail": "Effect thumbnail",
  "/api/v1/effects/draft": "Effect draft",
  "/api/v1/effects/import": "Import effect",
  "/api/v1/effects/preview": "Effect preview",
  "/api/v1/stacks": "Stacks",
  "/api/v1/stacks/{id}": "Stack",
  "/api/v1/stacks/preview": "Stack preview",
  "/api/v1/patterns": "Patterns",
  "/api/v1/patterns/{id}": "Pattern",
  "/api/v1/scenes": "Scenes",
  "/api/v1/scenes/{id}": "Scene",
  "/api/v1/scenes/{id}/groups/{group}/brightness": "Group brightness",
  "/api/v1/scenes/{id}/groups/{group}/move": "Move group",
  "/api/v1/scenes/{id}/groups/{group}/props": "Group properties",
  "/api/v1/scenes/active": "Active scene",
  "/api/v1/chains": "Chains",
  "/api/v1/chains/{device}/{zone}/setup": "Header setup",
  "/api/v1/chains/{device}/{zone}/setup/commit": "Setup commit",
  "/api/v1/chains/{device}/{zone}/setup/empty": "Empty header",
  "/api/v1/chains/{device}/{zone}/setup/light": "Setup light runs",
  "/api/v1/fans": "Fans",
  "/api/v1/fans/headers/{h}": "Fan header",
  "/api/v1/fans/headers/{h}/availability": "Fan header availability",
  "/api/v1/fans/hot": "Fan hot temperatures",
  "/api/v1/fans/profile": "Fan profile",
  "/api/v1/fans/profiles/{name}": "Fan profiles",
  "/api/v1/displays": "Displays",
  "/api/v1/displays/{id}": "Display",
  "/api/v1/displays/{id}/value": "Display value",
  "/api/v1/peripherals": "Peripherals",
  "/api/v1/peripherals/{id}": "Peripheral",
  "/api/v1/peripherals/{id}/access": "Peripheral access",
  "/api/v1/peripherals/{id}/read": "Peripheral read",
  "/api/v1/audio/sources": "Audio sources",
  "/api/v1/audio/source": "Audio source",
  "/api/v1/audio/test": "Audio test",
  "/api/v1/preview": "Preview",
  "/api/v1/preview/group": "Group preview",
  "/api/v1/preview/lights": "Light preview",
  "/api/v1/settings": "Settings",
  "/api/v1/update/install": "Install update",
  "/api/v1/config/export": "Config export",
  "/api/v1/config/import": "Config import",
  "/api/v1/onboarding/complete": "Complete onboarding",
  "/api/v1/first-run/dismiss-zone-panel": "Dismiss zone panel",
  "/api/v1/faults/{key}/dismiss": "Dismiss fault",
  "/api/v1/diag/readback": "Readback",
  "/api/v1/diag/fail-sensor": "Fail sensor",
};

/**
 * Hand-written shapes for the answers and requests whose Rust signature says only `Value` or a
 * bare response wrapper. Each one is a JSON object with its real fields, read from the handler.
 * A method with one of these types is published as "JSON object" and carries its shape beside the
 * entry; a reader never sees the wrapper type. A missing shape fails generation.
 *
 * These answers have no named type in the spine, so each shape is written from the answer the
 * daemon sends: the note under it says so, and the test in site/tests reads the handler's own
 * return expression and fails when a shape's field names drift from it. Where the answer is a
 * named type with the same fields, the note names that type instead.
 */
const HAND_SHAPES = {
  "AUDIO_TEST:DELETE:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the lease was released.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the lease was released." }],
  },
  "CHAIN_SETUP:DELETE:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the setup ended and the header's length after the restore.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the setup ended." },
      { name: "leds", type: "number", meaning: "The header's length after the restore." },
    ],
  },
  "CHAIN_SETUP_EMPTY:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the header was recorded empty and the length it was restored to.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the header was recorded empty." },
      { name: "leds", type: "number", meaning: "The header's restored length." },
    ],
  },
  "CHAIN_SETUP_LIGHT:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the coloured runs were shown.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the runs were shown." }],
  },
  "CONFIG_IMPORT:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the bundle was imported.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the bundle was imported." }],
  },
  "DEVICE_IDENTIFY:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the flash was started or stopped.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the flash was started or stopped." }],
  },
  "DEVICE_ZONE:PATCH:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the zone was resized, with the device and the zone it belongs to.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the zone was resized." },
      { name: "device_id", type: "string", meaning: "The device the zone belongs to." },
      { name: "zone", type: "number", meaning: "The zone's index." },
      { name: "led_count", type: "number", meaning: "The zone's new LED count." },
    ],
  },
  "DISMISS_ZONE_PANEL:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the panel was dismissed.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the panel was dismissed." }],
  },
  "FAULT_DISMISS:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the hardware note was hidden.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the note was hidden." }],
  },
  "DISPLAY:DELETE:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the display whose config was removed.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the saved config was removed." },
      { name: "device_id", type: "string", meaning: "The display whose config was removed." },
    ],
  },
  "EFFECT:PUT:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the effect whose values were saved.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the values were saved." },
      { name: "id", type: "string", meaning: "The effect whose values were saved." },
    ],
  },
  "EFFECT_ACTIVATE:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the effect that was played and the values it played with.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the effect was activated." },
      { name: "id", type: "string", meaning: "The effect that was played." },
      { name: "props", type: "JSON object", meaning: "The property values put on the lights." },
    ],
  },
  "EFFECT_DEFAULTS:PATCH:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the effect whose starting values changed and the values they now hold.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the starting values were saved." },
      { name: "id", type: "string", meaning: "The effect whose starting values changed." },
      { name: "props", type: "JSON object", meaning: "The merged starting values." },
    ],
  },
  "EFFECT_FAVORITE:PUT:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the effect that was marked and whether it is a favourite now.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the mark was saved." },
      { name: "id", type: "string", meaning: "The effect that was marked." },
      { name: "favorite", type: "boolean", meaning: "True when the effect is a favourite now." },
    ],
  },
  "FAN_HEADER:PATCH:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the header whose mode changed and the mode that was set.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the mode was accepted." },
      { name: "header", type: "string", meaning: "The header whose mode changed." },
      { name: "mode", type: "JSON object", meaning: "The mode that was set." },
    ],
  },
  "FAN_HEADER_AVAILABILITY:PUT:response": {
    label: "JSON object",
    note: "The fields of `FansStatus`, the fan status after the change. When the status cannot be read again, the answer carries the saved status with this header changed; when no status is saved, the answer carries only `ok`.",
    fields: [
      { name: "chip", type: "string or null", meaning: "The fan chip once it is detected." },
      { name: "hardware_watchdog", type: "boolean or null", meaning: "Whether a hardware watchdog is reachable; null while that is still being settled." },
      { name: "headers", type: "array of JSON objects", meaning: "One entry per fan header, with its mode and state." },
      { name: "temps", type: "array of JSON objects", meaning: "The temperature readings the service reports." },
      { name: "active_profile", type: "string", meaning: "The profile in force." },
      { name: "profiles", type: "array of strings", meaning: "The profiles the service knows." },
      { name: "floor_percent", type: "number", meaning: "The lowest duty the service allows." },
      { name: "hot", type: "array of JSON objects", meaning: "The hot line in force for each source kind." },
      { name: "chip_problem", type: "string or null", meaning: "Why the fan chip could not be reached, when it could not." },
      { name: "chip_checked", type: "boolean", meaning: "True once the fan service tried to open the chip. An older service leaves it false." },
    ],
  },
  "FAN_PROFILES:DELETE:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the profile that was deleted.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the profile was deleted." },
      { name: "name", type: "string", meaning: "The profile that was deleted." },
    ],
  },
  "DIAG_FAIL_SENSOR:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the temperature source that was told to fail and whether it now reports a failure.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the failure flag was applied." },
      { name: "source", type: "JSON object", meaning: "The temperature source told to fail." },
      { name: "failed", type: "boolean", meaning: "True when the source now reports a failure." },
    ],
  },
  "LIGHTS_FLASH:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the flash was started or stopped.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the flash was started or stopped." }],
  },
  "ONBOARDING_COMPLETE:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the flag was saved.",
    fields: [{ name: "ok", type: "boolean", meaning: "True when the flag was saved." }],
  },
  "QUIT:POST:response": {
    label: "JSON object",
    note: "The fields the daemon sends: the stop request was accepted.",
    fields: [{ name: "quitting", type: "boolean", meaning: "True when the stop request was accepted." }],
  },
  "STACK_PREVIEW:DELETE:response": {
    label: "JSON object",
    note: "The fields the daemon sends: whether a preview was dropped, and whether the page that sent the epoch owned it.",
    fields: [
      { name: "ok", type: "boolean", meaning: "True when the request was handled." },
      { name: "running", type: "boolean", meaning: "True when a preview was dropped by this call." },
      { name: "applied", type: "boolean", meaning: "True when the page that sent the epoch owned the preview." },
    ],
  },
};

// Three handlers answer with bytes, a small object or a streamed body instead of a named type.
const RESPONSE_OVERRIDES = {
  "EFFECT_THUMBNAIL:GET": "Binary thumbnail",
  "QUIT:POST": "JSON object",
  // Safe start streams its body so the restart waits for the reply to be written (#577).
  "SAFE_START:PUT": "SafeStartAnswer",
};

// Two handlers read a plain value and validate it into a type the spine publishes. Show that type
// to the reader, with its own fields, instead of a hand-written stand-in.
const REQUEST_TYPE_OVERRIDES = {
  "SETTINGS:PUT": "Settings",
  "FAN_HEADER_AVAILABILITY:PUT": "PutFanHeaderAvailability",
};

// One handler lives with the scenes but its route belongs with the lights.
const ROUTE_AREA_OVERRIDES = {
  LIGHTS_FLASH: "Devices and lights",
};

// Every body-taking route has a hand-written example. Generation fails when one is missing, so a
// new route cannot ship with a placeholder that its handler would refuse. The ids below match the
// fixtures in the example test that sends all of these through the real router.
const SAMPLE_EFFECT_HTML = '<head><title>Demo</title><meta description="A short demo." /></head><body><canvas id="stage" width="320" height="200"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>';
// Property values are tagged objects, so a bare number is refused by every route that reads them.
const NUMBER_PROP = { type: "number", value: 30 };
const NUMBER_PROP_LATER = { type: "number", value: 40 };
const EXAMPLE_BODIES = {
  "AUDIO_SOURCE:PUT": { kind: "follow_default" },
  "AUDIO_TEST:PUT": { listen: false },
  "CHAIN_SETUP:DELETE": { session: "setup-7f3a91c2" },
  "CHAIN_SETUP_COMMIT:POST": {
    session: "setup-7f3a91c2",
    parts: [{ name: "Fan run", kind: "fan", leds: 12 }],
  },
  "CHAIN_SETUP_EMPTY:POST": { session: "setup-7f3a91c2" },
  "CHAIN_SETUP_LIGHT:POST": {
    session: "setup-7f3a91c2",
    runs: [{ start: 0, count: 4, color: "#ff0000" }],
  },
  "CONFIG_IMPORT:POST": {
    schema_version: 1,
    exported_at: "2026-11-21T12:00:00Z",
    app_version: "0.18.1",
    settings: { fps: 30, black_on_lock: false, on_quit: "black", boot_color: "#1b2c3d", brightness: 80, power: "running", warmth: 6500 },
    layouts: { schema_version: 1, active: "", layouts: [] },
    zones: { schema_version: 1, sizes: {} },
    effects: { schema_version: 1, saved: {}, presets: {}, favorites: [] },
    fans: {
      schema_version: 1,
      floor_percent: 20,
      active_profile: "Default",
      profiles: [{ name: "Default", headers: {} }],
      boot_color: "#1b2c3d",
      hot_limits: { cpu_c: null, gpu_c: null, board_c: null },
    },
    displays: [{
      device: "display:5f6a:7b8c",
      source: { kind: "api" },
      unit: "celsius",
      decimals: 0,
      blink_above: null,
    }],
    stacks: [{ id: "stack:example", layers: [] }, { id: "stack:other", layers: [] }],
    patterns: [
      {
        id: "pattern:Example",
        built_on: { led_count: 4, arrangement: "ring" },
        frames: [{ colors: ["#d21e28", "#0f9d58", "#1a73e8", "#fbbc04"], hold: 0 }],
        motion: { type: "none" },
      },
      {
        id: "pattern:Other",
        built_on: { led_count: 4, arrangement: "ring" },
        frames: [{ colors: ["#101010", "#202020", "#303030", "#404040"], hold: 0 }],
        motion: { type: "none" },
      },
    ],
    lights: {
      schema_version: 1,
      chains: [],
      lights: [
        { id: "openrgb:2a3b4c5d#z0", device_id: "openrgb:2a3b4c5d", zone_index: 0, arrangement: "strip", reversed: false },
        { id: "openrgb:6e7f8091#z0", device_id: "openrgb:6e7f8091", zone_index: 0, arrangement: "strip", reversed: false },
        // A leftover light on the one device the daemon never drives: the delete route takes it
        // away, and a light on a driven device would answer 409. The name marks it as the
        // person's own, so the daemon keeps it in the file.
        { id: "openrgb:HID:VID_3151&PID_4015#z0", device_id: "openrgb:HID:VID_3151&PID_4015", zone_index: 0, arrangement: "strip", reversed: false, name: "Old keyboard lights" },
      ],
    },
    scenes: {
      schema_version: 1,
      active: "everything",
      scenes: [
        {
          id: "everything", name: "Everything", rev: 1,
          groups: [{ id: "everything", name: "Everything", members: ["openrgb:2a3b4c5d#z0"], plays: { type: "effect", effect: "solid", props: {} }, mode: "flow_across", brightness: 100 }],
        },
        { id: "scene:other", name: "Other", rev: 1, groups: [] },
        { id: "scene:retired", name: "Retired", rev: 1, groups: [] },
      ],
    },
  },
  "DEVICE_IDENTIFY:POST": { zone: 0, led: 0, stop: false },
  "DEVICE_WHITE_BALANCE:PUT": { red: 1.0, green: 1.0, blue: 1.0 },
  "DEVICE_ZONE:PATCH": { led_count: 512 },
  "DIAG_FAIL_SENSOR:POST": { source: "cpu", failed: true },
  "DISPLAY:PUT": {
    device: "display:2b1c:3d4e",
    source: { kind: "api" },
    unit: "celsius",
    decimals: 0,
    blink_above: 80.0,
  },
  "DISPLAY_VALUE:PUT": { value: 23.5 },
  "EFFECT:PUT": {
    schema: { id: "html:demo", name: "Demo", description: "A short demo.", needs: { screen: false, audio: false }, properties: [], presets: [] },
    favorite: true,
    user_presets: [{ name: "Cold", props: { speed: NUMBER_PROP } }],
    props: { speed: NUMBER_PROP },
  },
  "EFFECT_ACTIVATE:POST": { props: { speed: NUMBER_PROP }, preset: null },
  "EFFECT_DEFAULTS:PATCH": { speed: NUMBER_PROP },
  "EFFECT_FAVORITE:PUT": { favorite: true },
  "EFFECT_PRESETS:POST": { name: "Warm", props: { speed: NUMBER_PROP } },
  "EFFECT_PRESET:PUT": { rename_to: null, props: { speed: NUMBER_PROP_LATER }, expected: { speed: NUMBER_PROP } },
  "EFFECT_DRAFT:PUT": { html: SAMPLE_EFFECT_HTML, props: {} },
  "EFFECTS_IMPORT:POST": { file_name: "new-effect.html", html: SAMPLE_EFFECT_HTML, replace: false },
  "EFFECT_PREVIEW:POST": { html: SAMPLE_EFFECT_HTML, props: {} },
  "FAN_HEADER:PATCH": { mode: "bios" },
  "FAN_HEADER_AVAILABILITY:PUT": { available: false },
  "FAN_HOT:PUT": { cpu_c: 75.0, gpu_c: null, board_c: null },
  "FAN_PROFILE:PATCH": { name: "Default" },
  "FAN_PROFILES:PUT": { name: "Default", headers: {} },
  "LIGHT:PATCH": { name: "Front fans", arrangement: "ring" },
  "LIGHTS_FLASH:POST": { lights: ["openrgb:2a3b4c5d#z0"], stop: false },
  "OUTPUT:PATCH": { power: "running", brightness: 80, warmth: 6500 },
  "PATTERN:PUT": {
    id: "pattern:Example",
    built_on: { led_count: 4, arrangement: "ring" },
    frames: [{ colors: ["#d21e28", "#0f9d58", "#1a73e8", "#fbbc04"], hold: 0 }],
    motion: { type: "none" },
  },
  "PERIPHERAL:PATCH": { brightness: NUMBER_PROP },
  "PERIPHERAL_ACCESS:PUT": { reads: true, writes: true },
  "SCENES:POST": { name: "Gaming" },
  "SCENES:PUT": {},
  "SCENE:PUT": {
    scene: {
      id: "everything", name: "Everything", rev: 1,
      groups: [{ id: "everything", name: "Everything", members: ["openrgb:2a3b4c5d#z0"], plays: { type: "effect", effect: "solid", props: {} }, mode: "flow_across", brightness: 100 }],
    },
    expected_rev: 2,
  },
  "GROUP_BRIGHTNESS:PUT": { brightness: 80 },
  "GROUP_MOVE:POST": { to: "scene:other", copy: true, expected_rev_from: 4, expected_rev_to: 1 },
  "GROUP_PROPS:PATCH": { speed: NUMBER_PROP },
  "SCENE_ACTIVE:PUT": { id: "everything" },
  "SAFE_START:PUT": { mode: "normal" },
  "SETTINGS:PUT": {
    fps: 30,
    black_on_lock: false,
    on_quit: "black",
    boot_color: "#1b2c3d",
    brightness: 80,
    power: "running",
    warmth: 6500,
  },
  "STACK:PUT": { id: "stack:example", layers: [] },
  "STACK_PREVIEW:PUT": { stack: { id: "stack:example", layers: [] }, epoch: "a1b2c3d4e5f60718", seq: 1 },
  // The route insists on an absolute path. This one names no user folder: a reader pastes it
  // after downloading a build to the same drive. The signature is a placeholder of the right
  // shape (non-empty text under 4096 bytes); the service checks the real signature again.
  "UPDATE_INSTALL:POST": {
    installer: "D:\\Downloads\\canvas-rgb-0.18.1-setup.exe",
    signature: "untrusted comment: minisign signature\nRWQf6X8s1m4n3p2q1r0s7t6u5v4w3x2y1z0a9b8c7d6e5f4g3h2j1k0l9m8n7p6q5r4s3t2u1",
  },
};

/**
 * Every query parameter the reference publishes, with a value a reader can paste and a plain
 * description. The names come from the query structs in the router; the types are the types a
 * caller sends, so a reader knows what a value means without reading the handler.
 */
const QUERY_PARAM_INFO = {
  keep_presets: { value: "true", type: "boolean", meaning: "True keeps a removed effect's saved presets in the library, so importing the file again brings them back." },
  force: { value: "true", type: "boolean", meaning: "True confirms a change that a scene still depends on." },
  rescan: { value: "true", type: "boolean", meaning: "True asks the daemon to read the devices again before it answers. The answer carries the list it found." },
  fresh: { value: "true", type: "boolean", meaning: "True reads the device now. Without it the values are the ones the daemon kept from the last read." },
  "DeletePatternQuery.force": { value: "true", type: "boolean", meaning: "True deletes a pattern that a scene still plays." },
  "DeleteStackQuery.force": { value: "true", type: "boolean", meaning: "True deletes a stack that a scene still plays." },
  epoch: { value: "a1b2c3d4e5f60718", type: "string", meaning: "The page's epoch: the editor page's session counter. A preview stops only for the page whose epoch matches." },
  "DeleteDraftQuery.epoch": { value: "a1b2c3d4e5f60718", type: "string", meaning: "The page's epoch: the editor page's session counter. A delete from a page that does not own the draft answers applied false and leaves the draft running." },
  lease: { value: "<lease_id>", type: "string", meaning: "The lease_id returned by the audio test PUT. Paste that value into the URL." },
  token: { value: "$TOKEN", type: "string", meaning: "The API token, in the query string because a browser cannot set a header on a WebSocket handshake." },
  stack: { value: "true", type: "boolean", meaning: "True streams the stack preview engine." },
  group: { value: "everything", type: "string", meaning: "The id of a group in the active scene. The default scene's only group has the id everything." },
};

/**
 * A path placeholder's reader-facing type and meaning. An entry keyed `ROUTE.param` wins over the
 * plain name, so `{id}` says what it names on each route.
 */
const PATH_PARAM_INFO = {
  device: { type: "string", meaning: "The id of the device the route names." },
  zone: { type: "number", meaning: "The index of the zone, starting at 0." },
  h: { type: "string", meaning: "The id of a fan header, such as cpu_fan." },
  group: { type: "string", meaning: "The id of a group in the active scene." },
  id: { type: "string", meaning: "The id of the item the route names." },
  name: { type: "string", meaning: "The name of the saved item the route names." },
  "DEVICE_IDENTIFY.id": { type: "string", meaning: "The id of the device to flash." },
  "DEVICE_WHITE_BALANCE.id": { type: "string", meaning: "The id of the device whose gains are set." },
  "DEVICE_ZONE.id": { type: "string", meaning: "The id of the device whose zone is resized." },
  "DISPLAY.id": { type: "string", meaning: "The id of the display to change." },
  "DISPLAY_VALUE.id": { type: "string", meaning: "The id of the display to write to." },
  "EFFECT.id": { type: "string", meaning: "The id of the effect the route names." },
  "EFFECT_ACTIVATE.id": { type: "string", meaning: "The id of the effect to play." },
  "EFFECT_DEFAULTS.id": { type: "string", meaning: "The id of the effect whose starting values change." },
  "EFFECT_FAVORITE.id": { type: "string", meaning: "The id of the effect to mark or unmark." },
  "EFFECT_PRESETS.id": { type: "string", meaning: "The id of the effect the preset belongs to." },
  "EFFECT_PRESET.id": { type: "string", meaning: "The id of the effect the preset belongs to." },
  "EFFECT_PRESET.name": { type: "string", meaning: "The name of the saved preset to change." },
  "EFFECT_SOURCE.id": { type: "string", meaning: "The id of the effect whose source is read." },
  "EFFECT_THUMBNAIL.id": { type: "string", meaning: "The id of the effect whose tile is rendered." },
  "LIGHT.id": { type: "string", meaning: "The id of the light the route names." },
  "PATTERN.id": { type: "string", meaning: "The id of the pattern the route names." },
  "PERIPHERAL.id": { type: "string", meaning: "The id of the peripheral the route names." },
  "PERIPHERAL_ACCESS.id": { type: "string", meaning: "The id of the peripheral to open or close." },
  "PERIPHERAL_READ.id": { type: "string", meaning: "The id of the peripheral whose read is watched." },
  "SCENE.id": { type: "string", meaning: "The id of the scene the route names." },
  "GROUP_BRIGHTNESS.id": { type: "string", meaning: "The id of the scene that holds the group." },
  "GROUP_MOVE.id": { type: "string", meaning: "The id of the scene that holds the group." },
  "GROUP_PROPS.id": { type: "string", meaning: "The id of the scene that holds the group." },
  "GROUP_BRIGHTNESS.group": { type: "string", meaning: "The id of a group in the scene named by the `id` path parameter. Any scene, active or not." },
  "GROUP_MOVE.group": { type: "string", meaning: "The id of a group in the scene named by the `id` path parameter. Any scene, active or not." },
  "GROUP_PROPS.group": { type: "string", meaning: "The id of a group in the scene named by the `id` path parameter. Any scene, active or not." },
  "STACK.id": { type: "string", meaning: "The id of the stack the route names." },
  "FAN_PROFILES.name": { type: "string", meaning: "The name of the fan profile to change." },
  "FAULT_DISMISS.key": { type: "string", meaning: "The dismissal key of the note to hide, read from the status response." },
};

function pathParam(routeName, name) {
  const info = PATH_PARAM_INFO[`${routeName}.${name}`] || PATH_PARAM_INFO[name];
  return { name, type: info.type, meaning: info.meaning };
}

/**
 * A reader-facing meaning for a field with no doc comment. Every key names one type and one field,
 * so a shared field name never supplies the wrong meaning on a different type. Response fields say
 * what the server puts there.
 */
const FIELD_MEANINGS = {
  "ActiveEffect.id": "The id of the effect that is playing.",
  "ActiveEffect.props": "The property values the effect runs with, by property key.",
  "AudioEndpoint.flow": "Whether the endpoint is an input or an output.",
  "AudioEndpoint.id": "The id of the audio endpoint.",
  "AudioEndpoint.name": "The name of the endpoint: the friendly name Windows holds for it, or its description, or its id.",
  "AudioEndpoint.is_default": "True for the Windows default endpoint of its flow.",
  "AudioEndpoint.state": "Whether the endpoint can be opened now.",
  "AudioFallback.message": "The reason the capture gave, when it gave one.",
  "AudioFallback.reason": "Why the capture is not using the endpoint that was picked.",
  "AudioSource.flow": "Whether the endpoint is an input or an output.",
  "AudioSource.id": "The id of the endpoint to use.",
  "AudioSource.kind": "The kind of source to use: `follow_default` follows the system pick, `endpoint` names a device.",
  "AudioSource.name_hint": "A label to show for the chosen endpoint.",
  "AudioSources.current": "The source the capture resolved the choice to.",
  "AudioSources.endpoints": "The audio endpoints available as sources.",
  "AudioTestLease.lease_id": "The id returned when the test lease started.",
  "AudioTestRequest.listen": "Listen to the picked input. An output is tested without it; an input needs it. Leaving it out means false.",
  "AudioTestState.bits_per_sample": "The bit depth the capture device reports.",
  "AudioTestState.bands": "The bands the effects read, from 0 to 1. Empty when nothing is captured.",
  "AudioTestState.channels": "The channel count the capture reports.",
  "AudioTestState.format_tag": "The audio format the capture device reports.",
  "AudioTestState.packets": "How many packets arrived in the last interval.",
  "AudioTestState.peak": "The peak level the capture meter reports, from 0 to 1, before the endpoint's own volume.",
  "AudioTestState.peak_db": "The peak level in dBFS, floored at the silence floor.",
  "AudioTestState.sample_rate": "The sample rate the capture reports, in hertz.",
  "AudioTestState.silent_packets": "How many of those packets were silent.",
  "AudioTestState.source": "The audio source the test is using.",
  "AudioTestState.status": "The state of the audio test.",
  "BuiltOn.arrangement": "How the LEDs are laid out, such as `ring` or `strip`.",
  "BuiltOn.grid": "The size of the grid, for a grid pattern. It is left out for every other arrangement.",
  "BuiltOn.led_count": "How many LEDs the zone holds.",
  "ChainRecord.device_id": "The device the header is on.",
  "ChainRecord.state": "Whether the header was set up or marked empty.",
  "ChainRecord.zone_index": "The zone's index on the device.",
  "ChainView.device_id": "The device the header is on.",
  "ChainView.device_name": "The name of the device holding the header.",
  "ChainView.leds": "The length the header has now.",
  "ChainView.max_leds": "The largest LED count the zone allows.",
  "ChainView.min_leds": "The smallest LED count the zone allows.",
  "ChainView.parts": "The lights in chain order: the parts, or the whole-zone light.",
  "ChainView.state": "Whether the header was set up or marked empty.",
  "ChainView.zone_index": "The zone's index on the device.",
  "ChainView.zone_name": "The name of the zone on that device.",
  "ChainsResponse.chains": "The headers the daemon knows, with their zones.",
  "Conflict.kind": "The kind of conflict: `signal_rgb` means SignalRGB is running and will fight canvas-rgb for the devices.",
  "Conflict.process": "True while SignalRgb.exe is running.",
  "Conflict.service": "True while the SignalRgb.Service service is running.",
  "CurvePoint.percent": "The fan duty at this temperature, from 0 to 100.",
  "CurvePoint.temp_c": "The temperature in degrees Celsius.",
  "DeleteDraftQuery.epoch": "The page's epoch: the editor page's session counter. A delete from a page that does not own the draft leaves the draft running.",
  "DeleteEffectQuery.keep_presets": "True keeps a removed effect's saved presets in the library.",
  "DeleteEffectResponse.draft_ended": "Set when the delete ended a draft on the lights, so the editor can say the lights fell back. A delete with no draft on the lights leaves it out.",
  "DeleteEffectResponse.id": "The id of the effect that was deleted.",
  "DeleteEffectResponse.ok": "True when the effect was deleted.",
  "DeleteEffectResponse.presets_removed": "How many presets went with the effect.",
  "DeletePatternQuery.force": "True deletes a pattern that a scene still plays.",
  "DeletePatternResponse.favorite_removed": "True when the delete also removed the pattern from the favourites.",
  "DeletePatternResponse.id": "The id of the pattern that was deleted.",
  "DeletePatternResponse.note": "The sentence about the group that went dark, when a group played the pattern: `pattern:Example was deleted while a group played it, so that group is dark`. It is left out when no group played the pattern.",
  "DeletePatternResponse.ok": "True when the pattern was deleted.",
  "DeleteStackPreviewQuery.epoch": "The page's epoch: the editor page's session counter. A delete from a page that does not own the preview leaves the preview running.",
  "DeleteStackQuery.force": "True deletes a stack that a scene still plays.",
  "DeleteStackResponse.favorite_removed": "True when the stack was one of the person's favourites.",
  "DeleteStackResponse.id": "The id of the stack that was deleted.",
  "DeleteStackResponse.note": "Set when the deleted stack was on the lights and they went black. The text says the stack was deleted while a group played it, so that group is dark.",
  "DeleteStackResponse.ok": "True when the stack was deleted.",
  "DeleteStackResponse.presets_removed": "How many presets of the stack went with it.",
  "DeviceInfo.adapter": "The name of the adapter that found the device, such as `openrgb`. An adapter is the part of canvas-rgb that talks to one kind of hardware.",
  "DeviceInfo.adapter_index": "The index the adapter gave the device, such as the OpenRGB controller index. It changes across reconnects.",
  "DeviceInfo.connected": "True when the device is attached right now.",
  "DeviceInfo.id": "The id of the device.",
  "DeviceInfo.kind": "What the device is: a motherboard header, memory, a keyboard, a mouse, a microphone, a headset, an LED strip, a graphics card or another kind.",
  "DeviceInfo.location": "Where the device is attached.",
  "DeviceInfo.name": "The name of the device.",
  "DeviceInfo.serial": "The device's serial number, when it reports one.",
  "DeviceInfo.vendor": "The device's manufacturer.",
  "DeviceInfo.zones": "The zones the device has.",
  "DevicesResponse.devices": "The devices the daemon knows, with their zones.",
  "DisplayConfig.blink_above": "The value at or above which the display blinks; null leaves it steady.",
  "DisplayConfig.decimals": "How many digits follow the decimal point.",
  "DisplayConfig.device": "The display the config belongs to.",
  "DisplayConfig.source": "Where the display's number comes from.",
  "DisplayConfig.unit": "The unit the number is shown in: `celsius` or `fahrenheit`.",
  "DisplayInfo.blocked_by": "The name of the vendor app holding the display right now. The daemon finds it by checking whether the app's program is running, and leaves the display closed while it runs.",
  "DisplayInfo.connected": "True when the display is attached right now.",
  "DisplayInfo.digits": "How many digits the display has.",
  "DisplayInfo.id": "The id of the display.",
  "DisplayInfo.max_decimals": "The most decimal places the display can show.",
  "DisplayInfo.modes": "The modes the display supports.",
  "DisplayInfo.name": "The name of the display.",
  "DisplaySource.header": "The fan header whose temperature or speed is read.",
  "DisplaySource.kind": "Where the number comes from: a sensor, a fan speed, a fixed value or this API.",
  "DisplaySource.sensor": "The sensor a source reads.",
  "DisplaySource.value": "The number to show for a fixed source.",
  "DisplayStatus.config": "How the display is set.",
  "DisplayStatus.info": "The display itself.",
  "DisplayStatus.shown": "The value shown on the display, when one has been written.",
  "DisplayValue.blink": "True when the display blinks above its threshold.",
  "DisplayValue.decimals": "How many digits follow the decimal point.",
  "DisplayValue.mode": "The mode the display shows: a temperature or a fan speed.",
  "DisplayValue.unit": "The unit the number is shown in: `celsius` or `fahrenheit`.",
  "DisplayValue.value": "The number shown on the display.",
  "DraftEnded.note": "The sentence a stop carries when the effect that was playing cannot be restored: its file is gone with the delete, so the lights are black.",
  "DraftRequest.epoch": "The page's epoch: the editor page's session counter. A stack draft needs it.",
  "DraftRequest.html": "The draft HTML, at most 1 MiB of bytes. A stack draft leaves it out.",
  "DraftRequest.pattern": "A pattern as the draft. Exactly one of `html`, `stack` and `pattern` is set. A pattern draft needs `epoch` and `seq`, as a stack draft does.",
  "DraftRequest.props": "The property values the draft runs with.",
  "DraftRequest.seq": "The page's sequence number in that epoch; a dropped PUT answers 200 with applied false.",
  "DraftRequest.stack": "A whole stack as the draft. It plays on the lights, with its layer settings from this document and its property values under `props`.",
  "DraftStatus.active": "True while a draft is on the lights.",
  "DraftStatus.applied": "Whether the PUT left a draft on the lights. A stack draft that a newer epoch or sequence superseded answers false, and a DELETE that carries an older epoch answers false and leaves the draft running.",
  "DraftStatus.epoch": "The epoch that owns a stack draft, when one is running: the editor page's session counter.",
  "DraftStatus.heartbeat_ms": "How often a client PUTs to keep the draft alive.",
  "DraftStatus.id": "The id of the draft effect, made from `draft:` and the source hash.",
  "DraftStatus.lease_ms": "How long the draft survives without a PUT.",
  "DraftStatus.lease_remaining_ms": "Milliseconds left in the lease; 0 when no draft runs. A PUT renews it.",
  "DraftStatus.note": "A sentence about the ending, when it needs saying: the prior effect could not be restored, so the lights are black.",
  "DraftStatus.props": "The property values the draft runs with. Empty when no draft runs.",
  "DraftStatus.seq": "The highest sequence accepted for that epoch.",
  "DraftStatus.source_hash": "The hash of the HTML the draft was built from.",
  "EffectDetail.favorite": "True marks the effect as a favourite.",
  "EffectDetail.props": "The values the effect runs with when it is active, or its saved values, or the schema's defaults.",
  "EffectDetail.schema": "The effect's property schema.",
  "EffectDetail.user_presets": "The user's saved presets for this effect. The built-in ones are in the schema.",
  "EffectError.column": "The column in the draft's script, when the runtime gave one.",
  "EffectError.line": "The line in the draft's script, when the runtime gave one.",
  "EffectError.message": "The error text the runtime reported.",
  "EffectPreviewRequest.html": "The draft HTML, at most 1 MiB of bytes.",
  "EffectPreviewRequest.props": "The property values the preview is rendered with.",
  "EffectPreviewResponse.errors": "What the runtime reported while the draft was parsed and drawn. A preview with errors still carries the loop, so the last good frame stays drawn.",
  "EffectPreviewResponse.schema": "The draft's property schema.",
  "EffectPreviewResponse.source_hash": "The hash of the exact HTML the preview was rendered from.",
  "EffectPreviewResponse.thumbnail": "The 64 by 40, 24-frame loop rendered with the editor's property values, as base64. Decoded it is the thumbnail format: three 16-bit little-endian counts (width, height, frame count) followed by the raw RGB frames.",
  "EffectPreviewResponse.warnings": "The check's warnings: a picture that does not move, one that is one flat colour, and anything the file declared that the loader ignored.",
  "EffectSource.file_name": "The file name an import with `replace: true` would use: the file that is there for a user effect, or `<stem>.html` for a stock effect.",
  "EffectSource.source": "The effect's HTML text.",
  "EffectSummary.built_on": "The kind of device a pattern was built on. It is left out for a canvas effect.",
  "EffectSummary.description": "One line describing the effect.",
  "EffectSummary.favorite": "True marks the effect as a favourite.",
  "EffectSummary.id": "The id of the effect.",
  "EffectSummary.kind": "A canvas effect or a pattern.",
  "EffectSummary.name": "The name of the effect.",
  "EffectSummary.needs": "Which inputs the effect needs.",
  "EffectsFile.favorites": "The ids of the effects marked as favourites.",
  "EffectsFile.presets": "The saved presets, by effect id and then by preset name.",
  "EffectsFile.saved": "The last property values used, by effect id and then by property key.",
  "EffectsFile.schema_version": "The format version of the saved file.",
  "EffectsResponse.active": "The id of the active effect, or null when none is active.",
  "EffectsResponse.effects": "The effects in the library.",
  "EffectsResponse.failed": "User effect files in the effects folder that did not load, with the reason why.",
  "EmptiedGroup.group_id": "The id of the group that was removed.",
  "EmptiedGroup.name": "The name of the group, as it was in the scene.",
  "EmptiedGroup.scene_id": "The id of the scene that held the group.",
  "ExportBundle.app_version": "The version of the build that wrote the bundle.",
  "ExportBundle.displays": "The numeric displays' configs. They are absent in bundles from before displays, and import then replaces the saved display configs with these, like the other files.",
  "ExportBundle.effects": "The saved effects in the bundle.",
  "ExportBundle.exported_at": "The time the bundle was exported.",
  "ExportBundle.fans": "The saved fan settings in the bundle.",
  "ExportBundle.layouts": "The saved layouts in the bundle.",
  "ExportBundle.lights": "The saved lights. They are absent in older bundles, and import then migrates their layouts.",
  "ExportBundle.patterns": "The saved patterns. They are absent in older bundles.",
  "ExportBundle.schema_version": "The format version of the saved file.",
  "ExportBundle.scenes": "The saved scenes. They are absent in older bundles, as the lights are.",
  "ExportBundle.settings": "The saved application settings.",
  "ExportBundle.stacks": "The saved effect stacks. They are absent in bundles from before stacks, and import then replaces the saved stacks with these, like the other files.",
  "ExportBundle.zones": "The saved LED counts in the bundle.",
  "FailSensorRequest.failed": "True marks the source as failed.",
  "FailSensorRequest.source": "The temperature source to change: `cpu`, `gpu` or `board:<name>`.",
  "FanHeaderStatus.available": "False when this board does not route the channel to a header. Such a header stays in the list, is never written, and cannot be ticked in bulk. Old payloads without it parse as available.",
  "FanHeaderStatus.id": "The id of the fan header.",
  "FanHeaderStatus.label": "The name shown for the header.",
  "FanHeaderStatus.mode": "The mode the header was asked for.",
  "FanHeaderStatus.on_bios": "What the chip is doing: true when it is under BIOS control right now.",
  "FanHeaderStatus.override_available": "The person's own choice about whether the header is shown: false means the board does not carry it, true means show it anyway, null leaves the board's own table in charge.",
  "FanHeaderStatus.override_reason": "Why the header is not doing what the mode asks, when that happens.",
  "FanHeaderStatus.pwm_percent": "The PWM duty the chip reports, from 0 to 100.",
  "FanHeaderStatus.rpm": "The speed the chip reports, in revolutions per minute.",
  "FanMode.mode": "The mode the header runs in: `bios`, `fixed` or `curve`.",
  "FanMode.percent": "The duty the header runs at, from 0 to 100. Required for `fixed` and at or above the header's floor.",
  "FanMode.points": "The points of a `curve`, 2 to 8, sorted by temperature.",
  "FanMode.source": "The temperature source a `curve` reads: `cpu`, `gpu` or a named board sensor. Required for `curve`.",
  "FanProfile.name": "The name of the fan profile.",
  "FanProfile.headers": "The mode of each fan header, by header id. Headers that are not listed stay on BIOS.",
  "FanProfileName.name": "The name of the fan profile.",
  "FansFile.active_profile": "The fan profile in force.",
  "FansFile.boot_color": "The boot colour, mirrored here so the fan service has it before anyone signs in to Windows.",
  "FansFile.floor_percent": "The lowest duty the fan service allows, in percent.",
  "FansFile.hot_limits": "The temperatures the person set as the hot limits. Old files without them read as all automatic.",
  "FansFile.profiles": "The saved fan profiles.",
  "FansFile.schema_version": "The format version of the saved file.",
  "FansStatus.active_profile": "The fan profile in force.",
  "FansStatus.chip": "The name of the fan chip, once the fan service has detected it.",
  "FansStatus.chip_problem": "Why the fan chip could not be reached, when it could not.",
  "FansStatus.chip_checked": "True once the fan service tried to open the chip. An older service leaves it false.",
  "Fault.affected": "What does not work.",
  "Fault.cause": "Why it does not work, when the cause is known.",
  "Fault.device": "The device the note names, when it names one.",
  "Fault.dismissed": "True when this note was hidden by the person using the app.",
  "Fault.fix": "What to do about it.",
  "Fault.ignore_if": "When the note can be ignored.",
  "Fault.key": "The stable key used to dismiss the note.",
  "Fault.kind": "What the note is about: the fan chip or OpenRGB.",
  "FansStatus.floor_percent": "The lowest duty the fan service allows, in percent.",
  "FansStatus.hardware_watchdog": "Whether the chip has a hardware fan watchdog reachable via PawnIO; null while that is still being settled.",
  "FansStatus.headers": "One entry per fan header, with its mode and state.",
  "FansStatus.hot": "The hot line in force per kind of source, for the curve chart. Old payloads without it parse as empty.",
  "FansStatus.profiles": "The names of the profiles the fan service knows.",
  "FansStatus.temps": "The temperature readings the fan service reports.",
  "FlashRequest.lights": "The ids of the lights to flash.",
  "FlashRequest.stop": "True stops the flash.",
  "Frame.colors": "The colour of each LED, in position order.",
  "Frame.hold": "How long the frame holds, in milliseconds.",
  "Frame.transition": "How this frame passes to the next. A frame with no transition cuts to the next.",
  "GridSize.height": "The height of the grid, in rows.",
  "GridSize.width": "The width of the grid, in columns.",
  "Group.id": "The id of the group.",
  "Group.brightness": "The brightness of the group, from 0 to 100, multiplied with the global brightness.",
  "Group.members": "The lights in play order; the order replaces the position.",
  "Group.mode": "How the picture reaches the group's lights: `flow_across` shares one picture across them, `each_its_own` shows the whole picture on every light.",
  "Group.name": "The name of the group.",
  "Group.plays": "What the group plays: `off` or an effect.",
  "Group.sequence": "How the group's members run in order. Leave it out for each light to play the pattern on its own; `ring` and `line` run one pattern through the members in order.",
  "GroupBrightnessPut.brightness": "The brightness of the group, from 0 to 100.",
  "GroupMove.copy": "True copies the group: the source scene keeps the group and its revision, and the copy gets a fresh group id.",
  "GroupMove.dry_run": "True runs every check and saves nothing; the answer says saved false.",
  "GroupMove.expected_rev_from": "The revision of the source scene the request was made from.",
  "GroupMove.expected_rev_to": "The revision of the target scene the request was made from.",
  "GroupMove.to": "The id of the scene the group goes to.",
  "GroupMoved.before": "The scenes before the move, the source and then the target: what an undo writes back.",
  "GroupMoved.emptied": "The names of the groups that held no light afterwards and were removed.",
  "GroupMoved.group_id": "The id the group has in the target scene: its own when the target lacks it, else a fresh one; a copy always has a fresh one.",
  "GroupMoved.saved": "False for a dry run: nothing was written and the scenes below are what would be.",
  "GroupMoved.scenes": "The scenes after the move, the source and then the target. A copy leaves the source scene as it was.",
  "GroupMoved.taken": "The lights taken out of the target's other groups.",
  "GroupQuery.group": "The id of a group in the active scene.",
  "GroupQuery.token": "The API token, in the URL because a browser cannot set a handshake header.",
  "GroupState.failed": "The text of the error in the effect, or why it could not be built; the group stays dark while it is set.",
  "GroupState.fits": "How a pattern lands on each light of the group. It is empty for a canvas effect.",
  "GroupState.id": "The id of the group.",
  "GroupTaken.group_id": "The group in the target scene the lights were taken from.",
  "GroupTaken.group_name": "The name of the group the lights came from.",
  "GroupTaken.lights": "The ids of the lights taken from another group.",
  "HotLimitStatus.auto_c": "The automatic value, which is what a reset to automatic gives.",
  "HotLimitStatus.ceiling_c": "At or above this temperature every manual header on this source goes to 100 %. It is fixed.",
  "HotLimitStatus.kind": "Which temperature the limit applies to: `cpu`, `gpu` or `board`.",
  "HotLimitStatus.limit_c": "The line in force: the person's value or the automatic one.",
  "HotLimitStatus.max_settable_c": "The highest value the person may set.",
  "HotLimitStatus.rated_max_c": "The part's rated maximum temperature, when it is known.",
  "HotLimitStatus.user_set": "True when the person set it; false when it is automatic.",
  "HotLimits.board_c": "The board temperature for the hot limit, in degrees Celsius.",
  "HotLimits.cpu_c": "The CPU temperature for the hot limit, in degrees Celsius.",
  "HotLimits.gpu_c": "The GPU temperature for the hot limit, in degrees Celsius.",
  "IdentifyRequest.count": "How many LEDs to flash from `led`, such as a whole fan. Leave it out for one.",
  "IdentifyRequest.led": "The index of the LED to flash.",
  "IdentifyRequest.stop": "True stops the flash.",
  "IdentifyRequest.zone": "The zone index to flash, starting at 0.",
  "ImportEffect.file_name": "The file name to save it as: letters, digits, space, `-`, `_` and `.`, ending in `.html`. No folders. The name is refused when it holds `..`, when it starts or ends with a space or a dot, when the part before the first dot is a name Windows reserves, or when it would carry a built-in effect's id.",
  "ImportEffect.html": "The file's contents, at most 1 MiB of bytes.",
  "ImportEffect.replace": "True overwrites a user effect of the same name; without it a file of that name is refused with a conflict.",
  "ImportEffectResponse.effect": "The effect that was imported.",
  "ImportEffectResponse.replaced": "True when an existing file was overwritten.",
  "InputNeeds.audio": "True when the effect needs audio input.",
  "InputNeeds.screen": "True when the effect needs a screen capture.",
  "InstallUpdateRequest.installer": "The absolute path of the installer, readable by the calling user.",
  "InstallUpdateRequest.signature": "The text of the installer's signature file.",
  "InstallUpdateResponse.version": "The signed version the update service started installing.",
  "Landmark.facing": "The direction the part faces.",
  "Landmark.height": "The height of the part in canvas units.",
  "Landmark.kind": "What the part is, such as a fan or a graphics card.",
  "Landmark.label": "The name shown for the part.",
  "Landmark.locked": "True keeps the part from being dragged or edited.",
  "Landmark.rotation_deg": "The rotation of the part in degrees.",
  "Landmark.visible": "True shows the part.",
  "Landmark.width": "The width of the part in canvas units.",
  "Landmark.x": "The horizontal position of the part.",
  "Landmark.y": "The vertical position of the part.",
  "LayerArea.device": "The device the area covers.",
  "LayerArea.height": "The height of the area in canvas units.",
  "LayerArea.kind": "What the area covers: the whole canvas, a device, a zone or a rectangle.",
  "LayerArea.width": "The width of the area in canvas units.",
  "LayerArea.x": "The horizontal position of the area.",
  "LayerArea.y": "The vertical position of the area.",
  "LayerArea.zone": "The zone index the area covers, starting at 0.",
  "LayerMask.amount": "How strongly the mask applies, from 0 to 100. 0 leaves the layer whole; 100 applies it fully.",
  "LayerMask.effect": "The id of the effect the mask draws.",
  "LayerMask.invert": "True flips the mask: what was hidden shows and what showed is hidden.",
  "LayerMask.props": "The values the mask effect runs with; missing keys take the schema's defaults.",
  "LayerMask.source": "Which part of the mask's picture is read.",
  "FailedEffect.error": "Why the file did not load: the parser's own message, or a sentence saying the script ran out of time.",
  "FailedEffect.file": "The name of the file inside the effects folder, such as `new-effect.html`.",
  "Layout.case_form": "The PC's form factor, drawn behind the zones as a generic case with standard slots plus a desk area for peripherals. Layouts saved before this existed load as `none`, a free layout with no case drawn, so nothing moves.",
  "Layout.effect": "The effect playing on this layout, when the layout has one.",
  "Layout.id": "The id of the layout.",
  "Layout.landmarks": "The parts the person drew with no lights. Layouts saved before landmarks existed load with none.",
  "Layout.name": "The name of the layout.",
  "Layout.render_scale": "The render scale as a percentage of the logical canvas: 25, 50 or 100.",
  "Layout.sampling": "How each LED takes its colour from the rendered picture.",
  "Layout.zones": "The zones placed on this layout.",
  "LayoutsFile.active": "The id of the active layout.",
  "LayoutsFile.layouts": "The saved layouts.",
  "LayoutsFile.schema_version": "The format version of the saved file.",
  "LedSlice.count": "How many LEDs the range holds.",
  "LedSlice.start": "The index where the range starts.",
  "Light.arrangement": "How the LEDs are laid out, such as `ring` or `strip`.",
  "Light.copies": "How many times the header shows this light's samples: fans that all show the same LEDs are one light with this set to how many were counted. Leave it out, or set it to 1, to show it once.",
  "Light.device_id": "The device the light is on.",
  "Light.first_led": "Which of this light's LEDs is the first in its run: the physical LED, counted in the order the hardware wires them, that a pattern and every effect treat as position 0. It is left out for the first LED, as every light saved before this field existed.",
  "Light.id": "The id of the light.",
  "Light.kind": "What the person said this part is. It is left out for a whole-zone light and for a light saved before this field existed.",
  "Light.leds": "The run of the zone this light owns. Leave it out to own the whole zone.",
  "Light.name": "The name the person gave the light. Leave it out to show the device and zone name.",
  "Light.reversed": "The LEDs run the other way.",
  "Light.zone_index": "The zone's index on the device.",
  "LightDeleted.emptied_groups": "The groups the light was the last member of; each was dropped with it.",
  "LightDeleted.id": "The id of the light that was removed.",
  "LightDeleted.ok": "True when the light was removed.",
  "LightFit.fit": "How the pattern lands on that light.",
  "LightFit.light": "The id of the light this fit applies to.",
  "LightPatch.arrangement": "How the LEDs are laid out, such as `ring` or `strip`.",
  "LightPatch.first_led": "Which LED of the run is the first one. 0 starts the run at the first LED. It must be below the light's LED count.",
  "LightPatch.kind": "What the part is. Setting it sets `arrangement` from it; setting `arrangement` alone leaves `kind` as it was.",
  "LightPatch.name": "The name to give the light.",
  "LightPatch.reversed": "True walks the LEDs the other way.",
  "LightView.connected": "True when the light's device is attached right now.",
  "LightView.label": "The name the person gave the light, or the device and zone name.",
  "LightView.led_count": "The LEDs the light has now: its run clamped to the zone, or the zone's count.",
  "LightView.light": "The light itself, with its id, device, zone and run of LEDs.",
  "LightsFile.chains": "The headers the person set up or marked empty. They are absent in older files.",
  "LightsFile.lights": "The saved lights in the bundle.",
  "LightsFile.schema_version": "The format version of the saved file.",
  "LightsQuery.token": "The API token, in the URL because a browser cannot set a handshake header.",
  "LightsResponse.lights": "The lights with what the editor shows for each.",
  "Motion.type": "The kind of movement the frames have: `none` holds the first frame, `spin` turns the pattern along its LEDs at `turns_per_s` laps a second.",
  "NewGroup.brightness": "The brightness of the group, from 0 to 100; leave it out for 100.",
  "NewGroup.members": "The ids of the lights in play order.",
  "NewGroup.mode": "How the picture reaches the group's lights. Leave it out for one picture to flow across them.",
  "NewGroup.name": "The name of the group.",
  "NewGroup.plays": "What the group plays: `off` or an effect.",
  "NewGroup.sequence": "How the group's members run in order. Leave it out for each light to play the pattern on its own; `ring` and `line` run one pattern through the members in order.",
  "OutputState.brightness": "The brightness of the output, from 0 to 100.",
  "OutputState.power": "Whether the render loop runs or pauses.",
  "OutputState.warmth": "The warmth of the output in kelvin, from 1800 to 6500; 6500 leaves the picture unchanged.",
  "Part.facing": "The direction the part faces.",
  "Part.kind": "What the part is, such as a fan or a graphics card.",
  "Part.mapped": "True once the person confirmed this part in setup.",
  "PatchOutput.brightness": "The brightness to set, from 0 to 100. A field left out is unchanged.",
  "PatchOutput.power": "Whether the render loop runs or pauses. A field left out is unchanged.",
  "PatchOutput.warmth": "The warmth to set, in kelvin from 1800 to 6500. A value outside that range answers 422. A field left out is unchanged.",
  "PatchZone.led_count": "How many LEDs the zone holds.",
  "Pattern.built_on": "The device shape the pattern was built on.",
  "Pattern.frames": "The frames of the pattern, in order.",
  "Pattern.id": "The id of the pattern, made from `pattern:` and the name. A save never renames a pattern, and a body whose id does not match the path is refused.",
  "Pattern.motion": "How the frames move: `none` holds the first frame, `spin` turns the pattern along its LEDs at `turns_per_s` laps a second.",
  "Pattern.playback": "How the frames play. Leave it out to loop.",
  "PatternsResponse.patterns": "The saved patterns.",
  "PeripheralAccess.reads": "True opens the device and reads its settings. A read may send the device a request that changes no setting.",
  "PeripheralAccess.writes": "True sends config writes. It needs reads, because a write patches what the device last reported; `writes` without `reads` answers 422.",
  "PeripheralDetail.access": "What the daemon may do with this device.",
  "PeripheralDetail.info": "The peripheral itself.",
  "PeripheralDetail.problem": "Why the values are missing, when they are: reads are off, the device did not answer or the vendor app holds it. It is null after a good read.",
  "PeripheralDetail.read_at_ms": "When the values were last read from the device, in Unix milliseconds. The daemon answers from kept values, so the editor shows how old they are.",
  "PeripheralDetail.schema": "The peripheral's sections and properties.",
  "PeripheralDetail.values": "The values read from the device, by property key. A key the device did not report this time is absent; all of them are absent while reads are off.",
  "PeripheralInfo.adapter": "The name of the adapter that found the device, such as `openrgb`.",
  "PeripheralInfo.connected": "True when the adapter has the device open.",
  "PeripheralInfo.id": "The stable fingerprint of the device.",
  "PeripheralInfo.kind": "The kind of peripheral: a mouse, a game controller or a keyboard.",
  "PeripheralInfo.location": "Where the device is attached.",
  "PeripheralInfo.name": "The product name.",
  "PeripheralInfo.serial": "The serial number, empty when the device reports none.",
  "PeripheralInfo.vendor": "The name of the manufacturer.",
  "PeripheralSchema.device": "The peripheral the schema describes.",
  "PeripheralSchema.kind": "The kind of peripheral the schema describes.",
  "PeripheralSchema.read_only": "The property keys the adapter cannot write yet.",
  "PeripheralSchema.sections": "The sections, in the order the editor shows them.",
  "PeripheralSchema.unreported": "The ids of the settings the device holds that this adapter does not report as values: the fields the device has that this build cannot name, and the settings the device's firmware does not carry at all.",
  "PeripheralSection.id": "The stable id of the section, such as `dpi` or `lighting`.",
  "PeripheralSection.properties": "The properties of this section, in the order the editor shows them.",
  "PeripheralSection.title": "The title the editor shows for the section.",
  "PeripheralsResponse.peripherals": "The peripherals the adapters list.",
  "PlacedZone.device_id": "The device the zone is on.",
  "PlacedZone.height": "The height of the placement in canvas units.",
  "PlacedZone.label": "The person's own name for this zone, shown before the hardware's name. Leave it out to show the hardware name alone.",
  "PlacedZone.leds": "The run of the zone's LEDs this placement owns, when one zone drives several parts. Leave it out to own the whole zone.",
  "PlacedZone.locked": "True keeps the placement from being dragged or resized in the editor.",
  "PlacedZone.part": "The physical thing this placement stands for, as the person mapped it. Layouts saved before parts existed take it from the device kind.",
  "PlacedZone.reversed": "True walks the LEDs the other way.",
  "PlacedZone.rotation_deg": "The rotation of the placement in degrees, clockwise around its centre.",
  "PlacedZone.shape": "How the zone's LEDs sit inside the rectangle. Layouts saved before shapes existed load as `auto`, which keeps their old placement.",
  "PlacedZone.visible": "True shows the placement.",
  "PlacedZone.width": "The width of the placement in canvas units.",
  "PlacedZone.x": "The horizontal position of the placement.",
  "PlacedZone.y": "The vertical position of the placement.",
  "PlacedZone.zone_index": "The zone's index on the device.",
  "Playback.loop": "True goes back to the first frame after the last one; false leaves the last frame up.",
  "Plays.effect": "The id of the effect the group plays.",
  "Plays.props": "The property values the effect runs with, by property key.",
  "Plays.type": "What the group plays: `off` or `effect`.",
  "Preset.name": "The name of the preset.",
  "Preset.props": "The property values the preset saves, by property key.",
  "PresetCreate.name": "The name of the preset.",
  "PresetCreate.props": "The property values the preset saves, by property key.",
  "PresetReplace.expected": "The property values the client last saw for this preset.",
  "PresetReplace.props": "The property values the preset saves, by property key.",
  "PresetReplace.rename_to": "The new name for the preset; null keeps the name it has.",
  "PresetsResponse.user_presets": "The user's saved presets.",
  "PreviewQuery.stack": "Set to `1` or `true` to stream the stack preview.",
  "PreviewQuery.token": "The API token, in the URL because a browser cannot set a handshake header.",
  "PropKind.default": "The value the property starts at.",
  "PropKind.max": "The largest value allowed.",
  "PropKind.min": "The smallest value allowed.",
  "PropKind.options": "The choices the property offers.",
  "PropKind.step": "The amount one step of a control changes.",
  "PropKind.type": "The kind of property: `color`, `number`, `bool`, `choice`, `hue` or `angle`.",
  "PropValue.type": "The kind of value: `color`, `number`, `bool`, `choice`, `hue` or `angle`.",
  "PropValue.value": "The value itself: a colour as a hex string, a number, true or false, or one of the choices.",
  "PropertyDef.key": "The property's key, used as the global in the effect script.",
  "PropertyDef.kind": "What kind of control the property uses.",
  "PropertyDef.label": "The name shown for the property.",
  "PropertySchema.description": "One line describing the effect.",
  "PropertySchema.id": "`html:` plus the file name without `.html` for a user effect, or the built-in's own id.",
  "PropertySchema.name": "The name of the effect.",
  "PropertySchema.needs": "Which inputs the effect needs.",
  "PropertySchema.presets": "The built-in presets of the effect.",
  "PropertySchema.properties": "The properties the effect exposes.",
  "PropertySchema.thumbnail": "How the library tile is rendered. Leave it out to take the renderer's own.",
  "PutFanHeaderAvailability.available": "False hides the header, true shows it anyway, null leaves the board's own table in charge.",
  "ReadProgress.done": "The reports read so far while the stage is reading; 0 in the other stages.",
  "ReadProgress.stage": "The stage of the read.",
  "ReadProgress.total": "The reports the read will make; 0 when the adapter does not know a count.",
  "ReadbackDevice.colors": "The colours read back from the LEDs.",
  "ReadbackDevice.device_id": "The id of the device holding this zone or light.",
  "ReadbackDevice.name": "The name of the device.",
  "ReadbackResponse.devices": "The devices with the colours read back from their LEDs.",
  "ResolvedAudioSource.endpoint_id": "The endpoint the capture opened or will open; null when there is no endpoint at all.",
  "ResolvedAudioSource.fallback": "Why the capture is not on the picked endpoint.",
  "ResolvedAudioSource.flow": "Whether the endpoint is an input or an output.",
  "ResolvedAudioSource.name": "The name of the endpoint.",
  "ResolvedAudioSource.requested": "The audio source that was picked.",
  "Scene.groups": "The groups the scene holds.",
  "Scene.id": "The id of the scene.",
  "Scene.name": "The name of the scene.",
  "Scene.rev": "The revision of the scene; it moves with every saved change.",
  "SceneActivePut.id": "The id of the scene to use.",
  "SafeStartPut.mode": "The mode to restart in: normal or safe.",
  "SafeStart.active": "True while this run is in safe start.",
  "SafeStart.reason": "Why safe start is on; empty when the service gave no reason.",
  "SceneCreate.copy_of": "The id of a scene to copy the groups from.",
  "SceneCreate.dry_run": "True runs every check and saves nothing; the answer says saved false.",
  "SceneCreate.groups": "The groups the new scene holds, in order. The daemon mints each group's id and runs the scene rules before it saves.",
  "SceneCreate.name": "The name of the new scene.",
  "ScenePut.dry_run": "True runs every check and saves nothing; the answer says saved false.",
  "ScenePut.expected_rev": "The revision the change was made from.",
  "ScenePut.scene": "The scene to save.",
  "SceneRef.id": "The id of the scene.",
  "SceneRef.name": "The name of the scene.",
  "SceneWrite.expected_rev": "The revision the caller last read.",
  "SceneWrite.scene": "The scene to save.",
  "ScenesFile.active": "The id of the active scene.",
  "ScenesFile.scenes": "The saved scenes.",
  "ScenesFile.schema_version": "The format version of the saved file.",
  "ScenesResponse.active": "The id of the active scene.",
  "ScenesResponse.group_states": "The active scene's groups, in its order.",
  "ScenesResponse.scenes": "The saved scenes.",
  "ScenesWrite.expected_order": "The order the caller last saw; a different current order answers 409 carrying it.",
  "ScenesWrite.order": "The new order: every scene id exactly once. The same order as now answers 200 with no write.",
  "ScenesWrite.scenes": "The scenes to replace whole. Leave them out for an order-only write.",
  "SetDisplayValue.value": "The number to show on the display.",
  "ActivateEffect.groups": "The groups of the active scene to play the effect on, by name or id. Leave it out to play it on every group, from the effect's starting values.",
  "ActivateEffect.preset": "The name of a built-in preset or a user preset. A preset's values sit on the defaults, so two presets never leak into each other.",
  "ActivateEffect.props": "The values that sit on top of the saved values, the defaults or the preset.",
  "SetFavorite.favorite": "True marks the effect as a favourite.",
  "Settings.black_on_lock": "True turns the lights off while the screen is locked. It is off by default.",
  "Settings.boot_color": "The colour shown before the daemon connects, as a hex string such as #1b2c3d; null means black.",
  "Settings.brightness": "The brightness of the output, from 0 to 100.",
  "Settings.fps": "The frame rate target, from 10 to 60.",
  "Settings.on_quit": "The lighting to leave on at exit: `black` or `leave_last_frame`.",
  "Settings.power": "Whether the render loop runs or pauses.",
  "Settings.warmth": "The colour temperature in kelvin, from 1800 to 6500; 6500 leaves the picture unchanged.",
  "SetupCommit.parts": "The parts in chain order; their LED counts add up to the final length.",
  "SetupCommit.session": "The id the setup answered with.",
  "SetupCommitted.final_leds": "The header's final LED count.",
  "SetupCommitted.lights": "The lights the setup made, with what the editor shows for each.",
  "SetupEmpty.session": "The id the setup answered with.",
  "SetupLight.runs": "The coloured runs to show.",
  "SetupLight.session": "The id the setup answered with.",
  "SetupPart.copies": "How many times the header shows this run: eight fans that all show the same eight LEDs are one part of eight LEDs with this set to eight. Leave it out, or set it to 1, to show it once. A count of 2 or more is allowed only when this is the header's only part.",
  "SetupPart.kind": "What the part is: a fan, a strip or something else.",
  "SetupPart.leds": "How many LEDs the part holds.",
  "SetupPart.name": "The name of the part.",
  "SetupRun.color": "The colour of the run, as a hex string such as #ff0000.",
  "SetupRun.count": "How many LEDs the run covers.",
  "SetupRun.start": "The index where the run starts.",
  "SetupStarted.raised_to": "The length the header was raised to: the zone's maximum.",
  "SetupStarted.restore_to": "The length the header goes back to if the setup ends without a commit.",
  "SetupStarted.session": "The id the setup answered with.",
  "Stack.id": "The id of the stack, made from `stack:` and the name. The name is the id without the prefix, so there is one string.",
  "Stack.layers": "The layers of the stack, bottom first: the first layer is painted first.",
  "StackDetail.layers": "The state of each layer of the stack.",
  "StackDetail.stack": "The stack itself.",
  "StackLayer.area": "Where the layer is painted.",
  "StackLayer.blend": "How the layer sits over what is under it.",
  "StackLayer.effect": "The id of the effect the layer draws.",
  "StackLayer.id": "The id of the layer, one or two digits.",
  "StackLayer.mask": "An animated mask that shapes this layer only. Leave it out to draw the layer whole.",
  "StackLayer.name": "The name of the layer, as the person named it in the stack.",
  "StackLayer.opacity": "How strongly the layer shows, from 0 to 100. 0 draws nothing.",
  "StackLayer.props": "The values the effect runs with; missing keys take the schema's defaults.",
  "StackLayer.visible": "True shows the layer.",
  "StackLayerStatus.id": "The id of the layer, such as `l1` or `l2`.",
  "StackLayerStatus.last_ms": "The measured time of the last update that overran, in milliseconds. It is set for a held or faulted layer.",
  "StackLayerStatus.status": "What the layer is doing now.",
  "StackPreviewLayer.error": "Why the layer cannot run: its effect is gone, or it names another stack.",
  "StackPreviewLayer.id": "The id of the stack layer.",
  "StackPreviewLayer.status": "What the layer is doing now.",
  "StackPreviewQuery.group": "The id of a group in the active scene.",
  "StackPreviewRequest.epoch": "The page's epoch: the editor page's session counter.",
  "StackPreviewRequest.seq": "The page's sequence number for this preview: a lower one is dropped.",
  "StackPreviewRequest.stack": "The stack to preview.",
  "StackPreviewResponse.applied": "False when the PUT was superseded by a newer one, or a newer epoch owns the preview; the answer is a 200 and the page stops resending that change.",
  "StackPreviewResponse.epoch": "The epoch that owns the preview after this PUT: the editor page's session counter.",
  "StackPreviewResponse.layers": "What each layer of the previewed stack is doing.",
  "StackPreviewResponse.running": "True while the stack preview is running.",
  "StackPreviewResponse.seq": "The highest sequence accepted for that epoch.",
  "StackSummary.html_layers": "How many HTML runtimes it starts: one for each layer whose effect is HTML and one for each HTML mask.",
  "StackSummary.id": "The id of the stack, made from `stack:` and the stack's name.",
  "StackSummary.layers": "How many layers the stack has.",
  "StackSummary.missing": "The ids of the layers whose effect could not be found, so the library can say so.",
  "StackSummary.name": "The stack's name, without the `stack:` prefix.",
  "StacksResponse.stacks": "The saved stacks.",
  "Status.active_effects": "The ids of the effects the active scene's groups play, in group order, without repeats.",
  "Status.active_scene": "The active scene, when one is set.",
  "Status.brightness": "The brightness of the output, from 0 to 100.",
  "Status.conflicts": "The conflicts that need attention: SignalRGB is running and will fight canvas-rgb for the devices. The list is empty when SignalRGB is not running.",
  "Status.devices_connected": "How many devices are connected.",
  "Status.devices_total": "How many devices the daemon knows about, connected or not. The editor draws one card per device while the list is on its way, so it needs the total and the connected count.",
  "Status.faults": "Hardware notes, dismissed ones included. Empty from an older daemon.",
  "Status.fps_measured": "The frame rate measured over the last second.",
  "Status.fps_target": "The frame rate the daemon aims for.",
  "Status.frame_time_ms": "The time the last frame took, in milliseconds.",
  "Status.frames_rendered": "How many frames the daemon has rendered since it started. The count stops growing while the loop is parked or paused.",
  "Status.onboarding_complete": "False until the person finishes the first-run onboarding. While it is false the tray opens the window by itself onto the onboarding.",
  "Status.openrgb": "The state of the OpenRGB connection.",
  "Status.parked": "True when the render loop has nothing to draw for: no device the daemon drives wants frames and no preview client is watching.",
  "Status.power": "Whether the render loop runs or pauses.",
  "Status.preview_subscribers": "How many preview clients are attached.",
  "Status.service": "The state of the link to the fan and update service.",
  "Status.service_down_ms": "How long the link to the fan and update service has been down, in milliseconds, when it is down.",
  "Status.show_zone_panel": "True shows the editor's dismissible panel for sizing the ARGB headers.",
  "Status.version": "The version of the daemon.",
  "TempReading.celsius": "The temperature in degrees Celsius.",
  "TempReading.label": "The name shown for the reading.",
  "TempReading.source": "The temperature source this reading comes from.",
  "TempSource.name": "The name of the sensor.",
  "TempSource.type": "Which sensor the reading comes from: `cpu`, `gpu` or `board`.",
  "TestQuery.lease": "The id returned by the audio test PUT.",
  "ThumbnailRecipe.crossfade_frames": "Frames at the end of the loop that are crossfaded into the first frames, for an effect with no period. 0 means no crossfade.",
  "ThumbnailRecipe.dark_on_purpose": "Set when the effect is dark on purpose: the reason, which exempts the tile from the share of lit pixels bar.",
  "ThumbnailRecipe.loop_jump_on_purpose": "Set when the loop is meant to jump where it restarts: the reason, which exempts the tile from the seam bar.",
  "ThumbnailRecipe.loop_s": "How long the tile's loop lasts, in seconds, matched to the effect's period where it has one. The renderer uses 2 seconds.",
  "ThumbnailRecipe.props": "The property values the tile shows, laid over the schema's defaults.",
  "ThumbnailRecipe.warm_up_s": "How long the effect runs before the first recorded frame, in seconds, so trails and build-ups have content. The renderer uses 1 second.",
  "Transition.easing": "How a fade changes speed, such as `linear` or `ease_in_out`.",
  "Transition.kind": "How the frame passes to the next one: `cut` or `fade`.",
  "Transition.ms": "How long the passage takes, in milliseconds.",
  "WhiteBalance.blue": "The blue gain, as a multiplier.",
  "WhiteBalance.green": "The green gain, as a multiplier.",
  "WhiteBalance.red": "The red gain, as a multiplier.",
  "WhiteBalances.devices": "The colour gains of each device, by device id.",
  "ZoneInfo.index": "The zone's index on the device, starting at 0.",
  "ZoneInfo.kind": "What shape the zone's LEDs are in: `single`, `linear` or `matrix`.",
  "ZoneInfo.led_count": "How many LEDs the zone holds.",
  "ZoneInfo.max_leds": "The largest LED count this zone allows.",
  "ZoneInfo.min_leds": "The smallest LED count this zone allows.",
  "ZoneInfo.name": "The name of the zone.",
  "ZoneInfo.resizable": "True when the LED count can be changed, such as on MSI JARGB headers.",
  "ZoneKind.height": "The height of the matrix, in rows.",
  "ZoneKind.map": "The LED positions of the matrix, with null where there is no LED.",
  "ZoneKind.type": "What shape the zone's LEDs are in: `single`, `linear` or `matrix`.",
  "ZoneKind.width": "The width of the matrix, in columns.",
  "ZoneShape.columns": "The column count of a grid.",
  "ZoneShape.type": "The shape of the zone: `auto`, `strip`, `ring`, `outline`, `point` or `grid`.",
  "ZonesFile.schema_version": "The format version of the saved file.",
  "ZonesFile.sizes": "The LED count of each zone, by device id and then by zone index.",
  "array of DisplayStatus.config": "How the display is set.",
  "array of DisplayStatus.info": "What the display is.",
  "array of DisplayStatus.shown": "The value shown on the display, when one has been written.",
};

/**
 * A reader-facing meaning for each value of a string enum. The type's doc comment names the enum,
 * not the values, so each value carries its own plain sentence here.
 */
const ENUM_VALUE_MEANINGS = {
  "ChipProblem.pawn_io_unavailable": "PawnIO, the driver used to reach the fan chip, is unavailable.",
  "ChipProblem.chip_not_found": "No supported fan chip was found on the board.",
  "FaultKind.fan_chip": "The note is about the fan chip.",
  "FaultKind.open_rgb_down": "The note is about lights reached through OpenRGB.",
  "LastExit.clean": "The previous run ended on purpose.",
  "LastExit.unclean": "The previous run ended without a clean stop.",
  "LastExit.unknown": "There is no record of the previous run yet.",
  "LastExit.planned": "The previous run was stopped by a reboot, a logoff, an update or a service stop.",
  "LastExit.startup_failure": "The previous run failed while starting, before it was ready.",
  "SafeStartMode.normal": "The saved scene, effects, fan profile and readings run as usual.",
  "SafeStartMode.safe": "Safe start: the saved scene, effects and readings are off, the lights breathe and the fans stay on BIOS control.",
  "SafeStartReason.unclean_once": "The previous run did not end cleanly.",
  "SafeStartReason.crash_loop": "Several runs in a row did not end cleanly.",
  "SafeStartReason.user_chose": "The user asked for safe start; it stays on until they start normally.",
  "AudioFlow.render": "An output, captured as the sound it plays.",
  "AudioFlow.capture": "An input such as a microphone or line-in, captured directly.",
  "AudioTestStatus.off": "No lease is held and no effect needs audio.",
  "AudioTestStatus.waiting": "The capture is opening.",
  "AudioTestStatus.not_listening": "An input is picked but listening is off, so no input is opened.",
  "AudioTestStatus.no_device": "No active audio endpoint is available.",
  "AudioTestStatus.missing": "The pinned endpoint is not connected.",
  "AudioTestStatus.held_exclusive": "Another app holds the endpoint exclusively.",
  "AudioTestStatus.no_packets": "The capture runs but no packets arrive.",
  "AudioTestStatus.silent": "Packets arrive with a peak under the silence floor.",
  "AudioTestStatus.live": "The capture is receiving sound.",
  "AudioTestStatus.default_changed": "The Windows default output changed and the capture moved to it.",
  "AudioTestStatus.reopening": "The endpoint was invalidated and the capture is opening it again.",
  "AudioTestStatus.unsupported_format": "The capture cannot decode the endpoint's mix format.",
  "DisplayUnit.celsius": "The temperature in degrees Celsius.",
  "DisplayUnit.fahrenheit": "The temperature in degrees Fahrenheit.",
  "Arrangement.single": "One colour at the slot's centre.",
  "Arrangement.strip": "A line of LEDs, left to right at mid-height.",
  "Arrangement.ring": "A circle of LEDs from the top, clockwise.",
  "Arrangement.grid": "The zone's own matrix fills the slot, or a strip when the zone is not a matrix.",
  "ChainPartKind.fan": "A fan the person identified.",
  "ChainPartKind.strip": "An LED strip the person identified.",
  "ChainPartKind.other": "Anything else the person identified.",
  "Power.running": "The render loop is running.",
  "Power.paused": "Black was written once and the render loop stopped.",
  "ReadStage.idle": "No read is running.",
  "ReadStage.opening": "The device is being opened.",
  "ReadStage.waking": "The device is asleep and the adapter is waiting for it to wake.",
  "ReadStage.reading": "Settings are being read, one report at a time.",
  "OnQuit.black": "Lights go black at exit.",
  "OnQuit.leave_last_frame": "The last picture stays on the lights at exit.",
  "LinkState.connected": "The link is up.",
  "LinkState.connecting": "The link is being made.",
  "LinkState.down": "The link is down.",
  "AudioEndpointState.active": "The endpoint can be opened now.",
  "AudioEndpointState.not_connected": "The saved endpoint is unplugged, disabled or gone.",
  "ChainSetupState.not_set_up": "The header has no setup record.",
  "ChainSetupState.empty": "The header was marked as having nothing plugged into it.",
  "ChainSetupState.set_up": "The header's parts are set up as lights.",
  "DeviceKind.motherboard": "A motherboard's RGB header.",
  "DeviceKind.dram": "The memory modules.",
  "DeviceKind.keyboard": "A keyboard, as a device.",
  "DeviceKind.mouse": "A mouse, as a device.",
  "DeviceKind.microphone": "A microphone, as a device.",
  "DeviceKind.headset": "A headset, as a device.",
  "DeviceKind.led_strip": "An LED strip.",
  "DeviceKind.gpu": "A graphics card.",
  "DeviceKind.other": "A device of another kind.",
  "DisplayMode.temperature": "The display shows a temperature.",
  "DisplayMode.fan_rpm": "The display shows a fan speed in revolutions per minute.",
  "EffectKind.canvas": "An HTML effect that draws a picture.",
  "EffectKind.pattern": "A pattern of colours for LEDs.",
  "HotKind.cpu": "The CPU temperature.",
  "HotKind.gpu": "The GPU temperature.",
  "HotKind.board": "A board sensor temperature.",
  "PeripheralKind.mouse": "A mouse, as a peripheral.",
  "PeripheralKind.game_controller": "A game controller.",
  "PeripheralKind.keyboard": "A keyboard with settings and no LEDs of its own.",
  "GroupMode.flow_across": "One picture flows across every light in member order, each light's share set by its LED count.",
  "GroupMode.each_its_own": "Every light shows the whole picture.",
  "SequenceShape.ring": "One run through every member that closes into a ring, so gradients wrap and the pattern's `spin` motion turns it the whole way round.",
  "SequenceShape.line": "One run with two ends, so gradients do not wrap and the pattern's `spin` motion turns back at the end.",
  "StackLayerState.ok": "The layer is running.",
  "StackLayerState.missing_effect": "The layer's effect id resolves to no effect.",
  "StackLayerState.nested_stack": "The layer names another stack, which a stack cannot contain.",
  "StackLayerState.no_area": "The layer's area covers no pixel because a device or zone is gone or unplaced.",
  "StackLayerState.held": "The layer's update overran its budget, so it is called less often for a while.",
  "StackLayerState.faulted": "The layer overran its budget three times running and is frozen on its last picture.",
  "BlendMode.normal": "The layer covers what is below it.",
  "BlendMode.add": "Colours are added to the picture below.",
  "BlendMode.multiply": "Colours are multiplied with the picture below.",
  "BlendMode.screen": "Colours are screened over the picture below.",
  "SamplingMode.nearest": "Each LED takes the nearest render pixel.",
  "SamplingMode.bilinear": "Each LED blends the four nearest render pixels.",
  "SamplingMode.area_average": "Each LED takes the average of the render pixels under it.",
  "CaseForm.none": "No case is drawn; zones sit freely on the desk.",
  "CaseForm.mid_tower": "A mid-tower case.",
  "CaseForm.full_tower": "A full-tower case.",
  "CaseForm.small_form_factor": "A small form factor case.",
  "CaseForm.open_frame": "An open-frame bench case.",
  "ChainState.empty": "Nothing is plugged in, so the header shows black.",
  "ChainState.set_up": "The header's parts are set up as lights.",
  "TransitionKind.cut": "The next frame replaces this one when its hold ends.",
  "TransitionKind.fade": "The colours blend from this frame into the next over the transition's duration.",
  "Easing.linear": "The fade changes at a steady rate.",
  "Easing.ease_in_out": "The fade starts slowly, speeds up and slows again.",
  "Easing.ease_in": "The fade starts slowly and speeds up.",
  "Easing.ease_out": "The fade starts quickly and slows down.",
  "FitKind.exact": "The count and arrangement match the light, so LEDs copy colour for colour.",
  "FitKind.stretched": "The arrangement matches but the count differs, so the pattern stretches along the light.",
  "FitKind.other_shape": "The arrangement differs, so the pattern is laid along the LED order.",
  "FitKind.one_led": "A light with one LED takes the first colour in the pattern that is on.",
  "MaskSource.luma": "Brightness sets coverage: white shows the layer and black hides it.",
  "MaskSource.alpha": "The mask picture's alpha sets coverage.",
  "PartKind.fan": "A fan, as a part.",
  "PartKind.radiator": "A radiator; its fans are their own parts.",
  "PartKind.pump": "A pump cap.",
  "PartKind.ram": "The memory modules.",
  "PartKind.gpu": "A graphics card.",
  "PartKind.cpu": "A processor, as a part.",
  "PartKind.strip": "An LED strip.",
  "PartKind.keyboard": "A keyboard, as a part.",
  "PartKind.mouse": "A mouse, as a part.",
  "PartKind.microphone": "A microphone, as a part.",
  "PartKind.headset": "A headset, as a part.",
  "PartKind.case": "The case itself, drawn as an unlit box.",
  "PartKind.generic": "Anything else, drawn as a plain named box.",
  "Facing.viewer": "Facing the side glass, so the ring is visible.",
  "Facing.away": "Facing away from the side glass.",
  "Facing.up": "Facing up, seen edge-on as a bar.",
  "Facing.down": "Facing down, seen edge-on as a bar.",
  "Facing.left": "Facing to the left.",
  "Facing.right": "Facing to the right.",
  "AudioSource.follow_default": "Follow the system's default audio device.",
  "AudioSource.endpoint": "Use the named endpoint.",
  "FanMode.bios": "The fan runs at the speed the board's BIOS sets.",
  "FanMode.fixed": "The fan runs at a fixed duty.",
  "FanMode.curve": "The fan's duty follows a curve of temperature and duty points.",
  "AudioFallback.picked_missing": "The endpoint that was picked is gone, so the default output is used instead.",
  "AudioFallback.open_failed": "The endpoint that was picked could not be opened.",
  "DisplaySource.cpu_temp": "The CPU temperature.",
  "DisplaySource.gpu_temp": "The GPU temperature.",
  "DisplaySource.board_temp": "The temperature of a sensor on the board.",
  "DisplaySource.fan_rpm": "A fan's speed in revolutions per minute.",
  "DisplaySource.fixed": "A number saved with the display's config.",
  "DisplaySource.api": "A number written through this API. The display stays blank until a value arrives.",
  "PropValue.color": "A colour as a hex string such as #ff0000.",
  "PropValue.number": "The number itself.",
  "PropValue.bool": "True or false.",
  "PropValue.choice": "One of the property's choices.",
  "PropValue.hue": "A hue in degrees, from 0 to 360.",
  "PropValue.angle": "A direction in degrees, from 0 to 360.",
  "PropKind.color": "A colour, as a hex string.",
  "PropKind.number": "A number within a range.",
  "PropKind.bool": "True or false.",
  "PropKind.choice": "One of a fixed list of choices.",
  "PropKind.hue": "A hue in degrees.",
  "PropKind.angle": "A direction in degrees.",
  "TempSource.cpu": "The CPU temperature.",
  "TempSource.gpu": "The GPU temperature.",
  "TempSource.board": "A sensor on the board.",
  "Conflict.signal_rgb": "SignalRGB is running and will fight canvas-rgb for the devices.",
  "Plays.off": "The group plays nothing.",
  "Plays.effect": "The group plays an effect.",
  "LayerArea.canvas": "The whole canvas.",
  "LayerArea.device": "A whole device.",
  "LayerArea.zone": "One zone of a device.",
  "LayerArea.rect": "A rectangle of the canvas.",
  "RenderScale.25": "A quarter of the logical canvas: 80 by 50 pixels.",
  "RenderScale.50": "Half of the logical canvas: 160 by 100 pixels.",
  "RenderScale.100": "The whole logical canvas: 320 by 200 pixels.",
  "ZoneKind.single": "One LED on its own.",
  "ZoneKind.linear": "A line of LEDs.",
  "ZoneKind.matrix": "A grid of LEDs.",
  "ZoneShape.auto": "The shape follows the device's zone kind: one point, a strip along the long axis or the zone's own grid.",
  "ZoneShape.strip": "The LEDs are evenly spaced along the rectangle's long axis.",
  "ZoneShape.ring": "The LEDs are evenly spaced around the ellipse inside the rectangle, clockwise from the top.",
  "ZoneShape.outline": "The LEDs are evenly spaced around the rectangle's edge, clockwise from the top-left corner.",
  "ZoneShape.point": "Every LED samples the whole rectangle, so the light is one blob.",
  "ZoneShape.grid": "The LEDs fill a grid of columns and rows, from the top-left.",
};

/** True when a description only repeats its method, path or a type name. */
export function restatesDescription(text, routePath) {
  const t = text.trim();
  if (/^(GET|POST|PUT|PATCH|DELETE)\b/i.test(t)) return true;
  if (t.includes(routePath)) return true;
  if (/^(with|the body is|the answer is|answers?)\b/i.test(t)) return true;
  if (/^(?:a|an|the)\s+[A-Z]\w*(?:\s|$)/.test(t)) return true;
  return false;
}

/** Looks up a hand-written description, refusing one that restates its route. */
function described(routeName, method, routePath) {
  const key = `${routeName}:${method}`;
  const text = DESCRIPTIONS[key];
  if (!text) throw new Error(`no hand-written description for ${key} (${routePath})`);
  if (restatesDescription(text, routePath)) {
    throw new Error(`the description for ${key} restates its method, path or type: ${text}`);
  }
  return text;
}

/** True when a title only repeats its method or its full path. */
export function restatesTitle(title, routePath) {
  const t = title.trim();
  if (/^(GET|POST|PUT|PATCH|DELETE)\b/i.test(t)) return true;
  if (t.includes(routePath)) return true;
  return false;
}

/** Looks up a hand-written title, refusing one that is badly cased or restates its route. */
function titled(routePath) {
  const title = TITLES[routePath];
  if (!title) throw new Error(`no hand-written title for ${routePath}`);
  if (!/^[A-Z][a-z]+(?: [a-z]+){0,3}$/.test(title)) {
    throw new Error(`badly cased title for ${routePath}: ${title}`);
  }
  if (restatesTitle(title, routePath)) {
    throw new Error(`the title for ${routePath} restates its method or path: ${title}`);
  }
  return title;
}

/** Areas named in issue #540. Each router module maps to one. */
const AREAS = {
  about: "Status and output",
  status: "Status and output",
  output: "Status and output",
  devices: "Devices and lights",
  lights: "Devices and lights",
  effects: "Effects",
  editor: "Effects",
  stacks: "Stacks and patterns",
  patterns: "Stacks and patterns",
  stack_preview: "Stacks and patterns",
  scenes: "Scenes and chains",
  chains: "Scenes and chains",
  fans: "Fans",
  displays: "Displays and peripherals",
  peripherals: "Displays and peripherals",
  audio: "Audio",
  preview: "Preview",
  settings: "Settings and update",
  update: "Settings and update",
  diag: "Diagnostics",
};

/** Finds the index of the matching close bracket, skipping double-quoted strings. */
export function matchBracket(src, start, open, close) {
  let depth = 0;
  let inString = false;
  for (let i = start; i < src.length; i++) {
    const c = src[i];
    if (inString) {
      if (c === "\\") { i++; continue; }
      if (c === '"') inString = false;
      continue;
    }
    if (c === '"') { inString = true; continue; }
    if (c === open) depth++;
    else if (c === close) {
      depth--;
      if (depth === 0) return i;
    }
  }
  throw new Error(`unbalanced ${open}`);
}

/** The first comma at bracket depth zero, or -1. */
function topLevelComma(text) {
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "(" || c === "<") depth++;
    else if (c === ")" || c === ">") depth--;
    else if (c === "," && depth === 0) return i;
  }
  return -1;
}

/** A Rust type name without module prefixes, with generics kept. */
function typeName(raw) {
  let s = raw.trim().replace(/^&/, "");
  if (s === "serde_json::Value" || s.endsWith("::Value")) return "serde_json::Value";
  const parts = s.split("::");
  let last = parts[parts.length - 1].trim();
  const open = (last.match(/</g) || []).length;
  const close = (last.match(/>/g) || []).length;
  if (open > close) last += ">".repeat(open - close);
  return last;
}

/** Turns Rust doc comments into one plain sentence. Inline code keeps its backticks: the reader
 * gets the value, not the shape of the source. */
function oneLine(docLines) {
  let text = docLines.join(" ").trim();
  text = text.replace(/\[([^\]]+)\]\([^)]+\)/g, "$1"); // inline links
  text = text.replace(/\[`?([^`\]]+)`?\]/g, (_, label) => label.split("::").pop()); // links like [`Type`]
  text = text.replace(/\s+/g, " ").trim();
  const sentences = text.split(/\.\s+/).map((s) => s.trim()).filter(Boolean);
  // Handler docs often open with "`POST /api/v1/...`." Skip that line for the description.
  if (sentences.length > 1 && /^(GET|POST|PUT|PATCH|DELETE)\s+\/api\//.test(sentences[0])) {
    sentences.shift();
  }
  // "WebSocket." on its own says little; the next sentence does.
  if (sentences.length > 1 && sentences[0].split(/\s+/).length <= 2) sentences.shift();
  let out = (sentences[0] || "").replace(/\.$/, "");
  if (!out) return "";
  if (!out.endsWith(".")) out += ".";
  if (out.length > 220) out = out.slice(0, 217).trimEnd() + "...";
  return out;
}

/** Reads the `routes` module: constant name -> path and its doc comment. */
function parseRoutes(src) {
  const lines = src.split(/\r?\n/);
  const routes = new Map();
  let inBlock = false;
  let docs = [];
  for (const line of lines) {
    if (!inBlock) {
      if (/^\s*pub mod routes\s*\{/.test(line)) { inBlock = true; docs = []; }
      continue;
    }
    if (/^\}/.test(line)) break;
    const doc = /^\s*\/\/\/\s?(.*)$/.exec(line);
    if (doc) { docs.push(doc[1].trim()); continue; }
    const m = /^\s*pub const (\w+): &str = "([^"]+)";/.exec(line);
    if (m) {
      routes.set(m[1], { name: m[1], path: m[2], doc: oneLine(docs) });
      docs = [];
      continue;
    }
    if (line.trim() !== "") docs = [];
  }
  return routes;
}

/** Reads `pub type Alias = Type;` from the spine, so aliases resolve to the exported type. */
function parseAliases(src) {
  const aliases = new Map();
  const re = /pub\s+type\s+(\w+)\s*=\s*([^;]+);/g;
  let m;
  while ((m = re.exec(src))) aliases.set(m[1], typeName(m[2]));
  return aliases;
}

/** Finds `fn <name>(...) -> Type` in the daemon's own source, for handlers that answer a bare Response. */
function methodReturn(name) {
  const files = fs.readdirSync(DAEMON_SRC).filter((f) => f.endsWith(".rs"));
  let fallback = "";
  for (const file of files) {
    const src = fs.readFileSync(path.join(DAEMON_SRC, file), "utf8");
    const re = new RegExp(`fn\\s+${name}\\s*\\(([^)]*)\\)\\s*->\\s*([^\\{;]+)`, "gm");
    let m;
    while ((m = re.exec(src))) {
      const raw = m[2].trim();
      const option = /^Option\s*<\s*([\s\S]+)\s*>$/.exec(raw);
      const type = typeName(option ? option[1] : raw);
      // A call on `app` resolves to a method on App, whose receiver is `&self`.
      if (/\bself\b/.test(m[1])) return type;
      if (!fallback) fallback = type;
    }
  }
  return fallback;
}

/**
 * The flag names a handler reads out of its raw query. The handler parses them with `parse_flag`,
 * sometimes through a helper of its own, so the helper's body is read as well.
 */
function flagNames(src, body) {
  const names = new Set();
  const collect = (text) => {
    for (const m of text.matchAll(/parse_flag\s*\(\s*[^,]+,\s*"([^"]+)"/g)) names.add(m[1]);
  };
  collect(body);
  const seen = new Set();
  for (const m of body.matchAll(/\b([a-z_][a-z0-9_]*)\s*\(/g)) {
    const name = m[1];
    if (seen.has(name)) continue;
    seen.add(name);
    const fn = new RegExp(`(?:async\\s+)?fn\\s+${name}\\s*\\(`).exec(src);
    if (!fn) continue;
    const open = src.indexOf("(", fn.index);
    const close = matchBracket(src, open, "(", ")");
    const start = src.indexOf("{", close);
    if (start === -1) continue;
    collect(src.slice(start + 1, matchBracket(src, start, "{", "}")));
  }
  return [...names];
}

/** Reads every handler in crates/canvas-daemon/src/api/*.rs. */
function parseHandlers(structFields, aliases) {
  const handlers = new Map();
  for (const file of fs.readdirSync(API_DIR).filter((f) => f.endsWith(".rs") && f !== "mod.rs")) {
    const module = path.basename(file, ".rs");
    const src = fs.readFileSync(path.join(API_DIR, file), "utf8");
    const fnRe = /pub\s+(?:async\s+)?fn\s+(\w+)\s*\(/g;
    let m;
    while ((m = fnRe.exec(src))) {
      const name = m[1];
      const open = m.index + m[0].length - 1;
      const close = matchBracket(src, open, "(", ")");
      const args = src.slice(open + 1, close);
      const after = src.slice(close + 1);
      const arrow = after.indexOf("->");
      const bodyOpen = arrow === -1 ? -1 : after.indexOf("{", arrow);
      const ret = arrow === -1 ? "" : after.slice(arrow + 2, bodyOpen === -1 ? undefined : bodyOpen).trim();
      const body = bodyOpen === -1 ? "" : src.slice(close + 1 + bodyOpen + 1, matchBracket(src, close + 1 + bodyOpen, "{", "}"));

      // Doc comment directly above the function.
      const before = src.slice(0, m.index).split(/\r?\n/);
      let lineIdx = before.length - 1;
      if (before[lineIdx].trim() === "") lineIdx--; // the fragment before the fn on its own line
      const docs = [];
      for (let i = lineIdx; i >= 0; i--) {
        const d = /^\s*\/\/\/\s?(.*)$/.exec(before[i]);
        if (d) docs.unshift(d[1].trim());
        else break;
      }

      // Request body: a Json<T> extractor, or a raw body parsed in the handler.
      let request = "";
      const jm = /Json\s*<\s*([^>]+)\s*>/.exec(args);
      if (jm) request = typeName(jm[1]);
      else {
        const bm = /serde_json::from_slice\s*::<\s*([^>]+)\s*>/.exec(body)
          || /(?:let\s+(?:mut\s+)?\w+\s*:\s*)([A-Za-z0-9_:]+)\s*=\s*serde_json::from_slice/.exec(body);
        if (bm) request = typeName(bm[1]);
        else if (/axum::body::Bytes/.test(args)) request = "Bytes";
        else request = "none";
      }

      // Response: unwrap ApiResult<Json<T>>, Json<T>, then ApiResult<T>.
      let response = "";
      let rm = /ApiResult\s*<\s*Json\s*<\s*([\s\S]*?)\s*>\s*>/.exec(ret);
      if (!rm) rm = /\bJson\s*<\s*([\s\S]*?)\s*>/.exec(ret);
      if (!rm) rm = /(?:ApiResult|Result)\s*<\s*([\s\S]*?)\s*,/.exec(ret); // Result<Ok, Err>: the body is Ok
      if (!rm) rm = /ApiResult\s*<\s*([\s\S]*?)\s*>/.exec(ret);
      response = rm ? typeName(rm[1]) : typeName(ret || "Response");

      // A handler that returns axum's Response hides its body type in the return signature.
      // Unwrap the `Json(value).into_response()` in the body, then find the return type of the
      // `app.method()` call that produced `value`: status() on App answers Option<Status>.
      if (response === "Response") {
        const jm = /(?:axum::)?Json\s*\(\s*([A-Za-z_]\w*)\s*\)\s*\.into_response\s*\(\s*\)/.exec(body);
        if (jm) {
          const bm = /app\.(\w+)\s*\(/.exec(body);
          if (bm) {
            const found = methodReturn(bm[1]);
            if (found) response = found;
          }
        }
      }

      // Query: a Query<T> extractor plus the fields of that struct if it is in the spine or api dir.
      // A handler that reads the raw query parses its flags out of it, so the flag names are read
      // from the handler's own source too: none can be missed.
      let queryType = "";
      let queryParams = [];
      const qm = /Query\s*<\s*([A-Za-z0-9_:]+)\s*>/.exec(args);
      if (qm) {
        queryType = typeName(qm[1]);
        queryParams = (structFields.get(queryType) || []).map((f) => {
          const info = QUERY_PARAM_INFO[`${queryType}.${f.name}`] || QUERY_PARAM_INFO[f.name];
          if (!info) throw new Error(`no documented value for query parameter ${queryType}.${f.name}`);
          return { name: f.name, type: info.type, meaning: info.meaning, value: info.value };
        });
      } else if (/RawQuery\s*\(/.test(args)) {
        for (const name of flagNames(src, body)) {
          const info = QUERY_PARAM_INFO[name];
          if (!info) throw new Error(`no documented value for the query flag ${name}`);
          queryParams.push({ name, type: info.type, meaning: info.meaning, value: info.value });
        }
      }

      handlers.set(`${module}::${name}`, {
        doc: oneLine(docs),
        request: aliases.get(request) || request,
        response: aliases.get(response) || response,
        queryType,
        queryParams,
        websocket: /WebSocketUpgrade/.test(args),
      });
    }
  }
  return handlers;
}

/**
 * The Rust structs behind the published types: every field's type, whether the field carries a
 * serde default and whether the struct itself carries one. A reader may leave out a field with
 * `#[serde(default)]`, an `Option` field, or any field of a struct that carries
 * `#[serde(default)]`. The generated TypeScript marks only some of them with `?`, so the Rust
 * source is the truth about what a request may leave out.
 */
export function rustStructs() {
  const structs = new Map();
  const files = [
    ...fs.readdirSync(PROTO_SRC).filter((f) => f.endsWith(".rs")).map((f) => path.join(PROTO_SRC, f)),
    ...fs.readdirSync(API_DIR).filter((f) => f.endsWith(".rs")).map((f) => path.join(API_DIR, f)),
  ];
  for (const file of files) {
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    let attrs = [];
    let current = null;
    for (const line of lines) {
      const attr = /^\s*#\[([^\]]*)\]\s*$/.exec(line);
      if (attr) {
        attrs.push(attr[1]);
        continue;
      }
      const decl = /^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+(\w+)\s*\{\s*$/.exec(line);
      if (decl && !current) {
        current = {
          name: decl[1],
          default: attrs.some((a) => /^serde\(.*\bdefault\b/.test(a)),
          fields: new Map(),
        };
        attrs = [];
        structs.set(current.name, current);
        continue;
      }
      if (!current) {
        if (!/^\s*\/\//.test(line) && line.trim() !== "") attrs = [];
        continue;
      }
      const field = /^\s*pub\s+(\w+)\s*:\s*(.+?)\s*,?\s*$/.exec(line);
      if (field) {
        const type = field[2].replace(/,\s*$/, "").trim();
        const serde = attrs.filter((a) => /^serde\(/.test(a)).join(" ");
        const ts = attrs.filter((a) => /^ts\(/.test(a)).join(" ");
        const rename = /(?<![_a-z])rename\s*=\s*"([^"]+)"/.exec(serde);
        current.fields.set(field[1], {
          type,
          option: /^Option\s*</.test(type),
          default: /^serde\(.*\bdefault\b/.test(serde),
          flatten: /\bflatten\b/.test(serde),
          skip: /\bskip\b/.test(ts),
          // The name the API carries: the serde rename if there is one.
          published: rename ? rename[1] : field[1],
        });
        attrs = [];
        continue;
      }
      if (/^\s*\}/.test(line)) {
        current = null;
        attrs = [];
        continue;
      }
      // A doc comment sits between the attributes and the field it describes.
      if (!/^\s*\/\//.test(line) && line.trim() !== "") attrs = [];
    }
  }
  return structs;
}

/** The Rust struct of one type, read once. */
let rustCache = null;
function rustStruct(name) {
  if (!rustCache) rustCache = rustStructs();
  return rustCache.get(name) || null;
}

/** Reads `pub struct Name { pub field: Type }` from the spine and the api handler files. */
function parseStructFields() {
  const fields = new Map();
  const sources = [API_RS];
  for (const file of fs.readdirSync(API_DIR).filter((f) => f.endsWith(".rs"))) {
    sources.push(path.join(API_DIR, file));
  }
  for (const file of sources) {
    const src = fs.readFileSync(file, "utf8");
    const re = /pub\s+struct\s+(\w+)\s*\{([\s\S]*?)\n\}/g;
    let m;
    while ((m = re.exec(src))) {
      const parsed = [];
      let docs = [];
      for (const line of m[2].split(/\r?\n/)) {
        const doc = /^\s*\/\/\/\s?(.*)$/.exec(line);
        if (doc) { docs.push(doc[1].trim()); continue; }
        const fm = /^\s*pub\s+(\w+)\s*:\s*(.+?)\s*,?\s*$/.exec(line);
        if (fm) {
          const raw = fm[2].trim().replace(/,$/, "").trim();
          const option = /^Option\s*<\s*(.+)\s*>$/.exec(raw);
          parsed.push({
            name: fm[1],
            type: option ? option[1].trim() : raw,
            optional: Boolean(option),
            doc: oneLine(docs),
          });
          docs = [];
          continue;
        }
        if (line.trim() !== "") docs = [];
      }
      if (!fields.has(m[1])) fields.set(m[1], parsed);
    }
  }
  return fields;
}

/** Reads the router: each .route(...) call, with its path expression and method/handler pairs. */
function parseRouter(src) {
  const fnStart = src.indexOf("pub fn router(");
  const bodyOpen = src.indexOf("{", fnStart);
  const bodyClose = matchBracket(src, bodyOpen, "{", "}");
  const body = src.slice(bodyOpen + 1, bodyClose);
  const calls = [];
  let idx = 0;
  while ((idx = body.indexOf(".route(", idx)) !== -1) {
    const open = idx + ".route".length;
    const close = matchBracket(body, open, "(", ")");
    calls.push(body.slice(open + 1, close));
    idx = close + 1;
  }
  return calls;
}

/** Splits TS text on a separator at bracket depth zero, ignoring quoted strings. */
function splitTopLevel(text, separator) {
  const parts = [];
  let depth = 0;
  let quote = "";
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === "\\") { i++; continue; }
      if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'" || c === "`") { quote = c; continue; }
    if (c === "=" && text[i + 1] === ">") { i++; continue; } // arrow, not a greater-than
    if ("([{<".includes(c)) depth++;
    else if (")]}>".includes(c)) depth--;
    else if (depth === 0 && c === separator) {
      parts.push(text.slice(start, i).trim());
      start = i + 1;
    }
  }
  parts.push(text.slice(start).trim());
  return parts;
}

/** Finds the `;` that ends a TS type alias, at bracket depth zero. */
function aliasEnd(src, from) {
  let depth = 0;
  let quote = "";
  for (let i = from; i < src.length; i++) {
    const c = src[i];
    if (quote) {
      if (c === "\\") { i++; continue; }
      if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'" || c === "`") { quote = c; continue; }
    if (c === "=" && src[i + 1] === ">") { i++; continue; }
    if ("([{<".includes(c)) depth++;
    else if (")]}>".includes(c)) depth--;
    else if (c === ";" && depth === 0) return i;
  }
  return src.length;
}

/**
 * Maps a TS or Rust field type to the JSON word a reader sees, and returns the named types it
 * refers to. A named type keeps its name; the reference publishes a definition for it too.
 */
function readerFieldType(raw) {
  const s = String(raw).trim();
  // An index map: { [key in DeviceId]: V }, { [key in number]: V } or { [key: string]: V }.
  let m = /^\{\s*\[\s*\w+\s+(?:in|:)\s*([^\]]+?)\s*\]\s*:\s*([\s\S]+)\s*\}$/.exec(s);
  if (m) {
    const inner = readerFieldType(m[2]);
    const keyWord = /^(?:number|integer)$/.test(m[1].trim()) ? "number" : "text";
    return { type: mapType(keyWord, inner.type), refs: inner.refs };
  }
  // An array: Array<X>, X[] or Vec<X>.
  m = /^Array\s*<\s*([\s\S]+)\s*>$/.exec(s) || /^Vec\s*<\s*([\s\S]+)\s*>$/.exec(s) || /^([\s\S]+)\[\]$/.exec(s);
  if (m) {
    const inner = readerFieldType(m[1]);
    return { type: `array of ${inner.type}`, refs: inner.refs };
  }
  // A nullable value: X | null or Option<X>.
  m = /^([\s\S]+?)\s*\|\s*null$/.exec(s) || /^Option\s*<\s*([\s\S]+)\s*>$/.exec(s);
  if (m) {
    const inner = readerFieldType(m[1]);
    return { type: `${inner.type} or null`, refs: inner.refs };
  }
  // A union: say what each alternative is in the reader's words.
  if (splitTopLevel(s, "|").length > 1) {
    const parts = splitTopLevel(s, "|").map((p) => readerFieldType(p.trim()));
    return { type: parts.map((p) => p.type).join(" or "), refs: parts.flatMap((p) => p.refs) };
  }
  const primitives = new Map([
    ["string", "string"], ["String", "string"], ["str", "string"], ["&str", "string"],
    ["number", "number"], ["f32", "number"], ["f64", "number"], ["f16", "number"],
    ["bigint", "integer"],
    ["u8", "integer"], ["u16", "integer"], ["u32", "integer"], ["u64", "integer"], ["usize", "integer"],
    ["i8", "integer"], ["i16", "integer"], ["i32", "integer"], ["i64", "integer"], ["isize", "integer"],
    ["boolean", "boolean"], ["bool", "boolean"], ["null", "null"],
    ["unknown", "JSON object"], ["any", "JSON object"], ["Value", "JSON object"],
    ["serde_json::Value", "JSON object"], ["serde_json::Map<String, Value>", "JSON object"],
  ]);
  if (primitives.has(s)) return { type: primitives.get(s), refs: [] };
  if (/^"[^"]*"$/.test(s)) return { type: s, refs: [] };
  const name = typeName(s);
  return { type: name, refs: [name] };
}

/** A map in plain words: an object with keys of one kind and values of another. */
function mapType(keyWord, inner) {
  if (/^an object keyed by /.test(inner)) return `an object keyed by ${keyWord}, holding ${inner}`;
  return `an object keyed by ${keyWord}, with ${inner} values`;
}

/** The named types inside a reader-facing type label, including the inner type of a container. */
function referencedNames(label) {
  const s = String(label).trim();
  const inner = /^array of (.+)$/.exec(s) || /^(.+) or null$/.exec(s) || /^an object keyed by (?:text|number), (?:holding|with) (.+?)(?: values)?$/.exec(s);
  if (inner) return referencedNames(inner[1]);
  const out = new Set();
  for (const part of s.includes(" or ") ? s.split(" or ") : [s]) {
    const name = typeName(part.trim());
    if (/^[A-Z]\w*$/.test(name)) out.add(name);
  }
  return out;
}

/** The meaning published for one field of one type. The hand-written table comes first: it says
 * what the field means in the reader's words; the field's own doc comment is the fallback. */
function meaningFor(typeName, field) {
  const hand = FIELD_MEANINGS[`${typeName}.${field.name}`];
  if (hand) return capitalise(hand);
  if (field.doc) return field.doc;
  if (/DeviceId/.test(field.type)) return "A device id.";
  if (/Rgb8/.test(field.type)) return "A colour as a hex string such as #ff0000.";
  return "";
}

/** A meaning as a reader reads it: one sentence, capitalised. */
function capitalise(text) {
  const trimmed = String(text).trim();
  return trimmed ? trimmed.charAt(0).toUpperCase() + trimmed.slice(1) : trimmed;
}

/**
 * The words that name the inside of the program rather than the reader's world, with what each
 * becomes. A reader never meets a spine, a pipe or a router.
 */
const INTERNAL_WORDS = [
  [/\bspine\b/gi, ""],
  [/\bpipe\b/gi, ""],
  [/\brouter\b/gi, ""],
  [/\bmain task\b/gi, ""],
  [/\bWebView\b/gi, ""],
  [/\bshim\b/gi, ""],
  [/\blit-share\b/gi, ""],
  [/\brecipes?\b/gi, "settings"],
  [/\bthe owner's\b/gi, "the person's"],
  [/\bowner's\b/gi, "person's"],
  [/\bowner\b/gi, "person"],
  [/\bmy\b/gi, "the person's"],
];

/**
 * Turns a doc comment into reader prose. The fact in the sentence stays: a code span keeps its
 * backticks, a parenthetical and a range stay, a quoted value stays, and a Rust literal becomes
 * the JSON a reader sends. Build history goes: issue references, roadmap codes, version notes and
 * the words that name the inside of the program.
 */
function cleanDoc(text) {
  const joined = text
    .split("\n")
    .map((line) => line.replace(/^\s*\*?\s?/, "").trim())
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();

  // A Rust doc link is a value the reader reads, so it becomes a code span.
  const linked = joined
    .replace(/\[\s*`([^`]+)`\s*\]/g, "`$1`")
    .replace(/\[\s*([A-Za-z_][\w:]*(?:::[A-Za-z_]\w*)*)\s*\]/g, "`$1`");

  // The code spans come out first, so nothing below can eat a value inside one and the sentence
  // splitter never breaks a sentence in the middle of a value.
  const spans = [];
  let prose = linked.replace(/`([^`]+)`/g, (_, span) => {
    spans.push(span);
    return `\u0000${spans.length - 1}\u0000`;
  });

  // Build history: issue references, roadmap codes and version notes.
  prose = prose
    .replace(/\bissues?\s*#\s*\d+\b/gi, "")
    .replace(/#\d+'s/gi, "the")
    .replace(/#\d+\b/g, "")
    .replace(/\bG\d+\b:?\s*/g, "")
    .replace(/\bD95(?:\.\d+)*\b/g, "")
    .replace(/\bS\d+\b/g, "")
    .replace(/\(\s*\d+\.\d+\.\d+\s*\)/g, "")
    .replace(/\(\s*:/g, "(")
    .replace(/:\s*\)/g, ")");

  // The words that name the inside of the program.
  for (const [pattern, to] of INTERNAL_WORDS) prose = prose.replace(pattern, to);

  // Rust literals become the JSON a reader sends.
  prose = prose
    .replace(/\bNone\b/g, "null")
    .replace(/\bSome\s*\(([^()]*)\)/g, "$1")
    .replace(/\b[A-Za-z_]\w*::[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*/g, (path) => path.split("::").pop());
  prose = rustGeneric(prose, "Vec", (inner) => `an array of ${inner}`);
  prose = rustGeneric(prose, "Option", (inner) => inner);
  prose = rustGeneric(prose, "Result", (inner) => inner.split(",")[0].trim());

  // Ranges read as words.
  prose = prose.replace(/(\d+)\s*\.\.=?\s*(\d+)/g, "$1 to $2");

  // Version notes are build history.
  prose = prose.replace(/\b\d+\.\d+\.\d+\b/g, "");

  // Filler a reader does not need.
  prose = prose
    .replace(/\be\.g\.?,?\s*/gi, "for example ")
    .replace(/\bi\.e\.?,?\s*/gi, "that is ")
    .replace(/\bLuke's answer[^.;]*|as before|as it was|one's own\b/gi, "")
    .replace(/\s*\+\s*/g, " and ");

  prose = prose
    .replace(/\(\s*\)/g, "")
    .replace(/[,;:]\s*[,;:]+/g, ",")
    .replace(/\s*[,;]\s*(?=[.;]|$)/g, "")
    .replace(/\s+([.,;:])/g, "$1")
    .replace(/:\./g, ".")
    .replace(/\s+/g, " ")
    .trim();

  // Keep the first two sentences that carry a fact. Later ones are build history, and a sentence
  // built on a contrast adds no fact about the field.
  const sentences = prose.match(/[^.]+\.(?:\s|$)/g) || [prose];
  const kept = sentences
    .map((sentence) => sentence.trim())
    .filter((sentence) => !/\bnever\b/i.test(sentence))
    .slice(0, 2);
  prose = (kept.length ? kept : [sentences[0].replace(/,?\s*never[^,]*$/i, "")]).join(" ").trim();

  // The code spans come back, each one as the value a reader reads.
  prose = prose.replace(/\u0000(\d+)\u0000/g, (_, index) => codeSpan(spans[Number(index)]));

  const clean = prose.replace(/^[\s,;:.]+|[\s,;:.]+$/g, "");
  return clean ? clean.charAt(0).toUpperCase() + clean.slice(1) : clean;
}

/** The index of the `>` that closes the `<` at `open`, counting the ones that nest. */
function matchAngle(text, open) {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    if (text[i] === "<") depth++;
    else if (text[i] === ">") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** Rewrites `Name<...>` with the words a reader reads, matching the brackets that nest. */
function rustGeneric(text, name, to) {
  const re = new RegExp(`\\b${name}\\s*<`, "g");
  let out = "";
  let index = 0;
  let m;
  while ((m = re.exec(text))) {
    const open = text.indexOf("<", m.index);
    const close = matchAngle(text, open);
    if (close === -1) break;
    out += text.slice(index, m.index) + to(text.slice(open + 1, close).trim());
    index = close + 1;
    re.lastIndex = close + 1;
  }
  return out + text.slice(index);
}

/** A code span, with a Rust path cut down to its last part and its literals in JSON's words. */
function codeSpan(span) {
  let text = span.trim();
  text = text.replace(/\bNone\b/g, "null");
  text = text.replace(/\bSome\s*\(([^()]*)\)/g, "$1");
  text = rustGeneric(text, "Vec", (inner) => `an array of ${inner}`);
  text = rustGeneric(text, "Option", (inner) => inner);
  text = text.replace(/\b[A-Za-z_]\w*::[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*/g, (path) => path.split("::").pop());
  text = text.replace(/(\d+)\s*\.\.=?\s*(\d+)/g, "$1 to $2");
  return `\`${text}\``;
}

/** Doc comments in a ts-rs file, keyed by the field or quoted key they sit above. */
function docComments(src) {
  const docs = new Map();
  const re = /\/\*\*([\s\S]*?)\*\//g;
  let m;
  while ((m = re.exec(src))) {
    const following = /^\s*(?:"([^"]+)"|'([^']+)'|([A-Za-z_]\w*))/.exec(src.slice(m.index + m[0].length));
    if (!following) continue;
    const key = following[1] || following[2] || following[3];
    const prose = cleanDoc(m[1]);
    if (prose.length > 3) docs.set(key, prose);
  }
  return docs;
}

/**
 * Reads one named type from its ts-rs file: a struct's fields, a string enum's values or a type
 * alias's plain value. A tagged union is flattened into one field list, with a field that only
 * some variants carry marked optional. No new dependencies.
 */
function tsDefinition(name) {
  const file = path.join(PROTO_DIR, `${name}.ts`);
  if (!fs.existsSync(file)) return null;
  const raw = fs.readFileSync(file, "utf8");
  const docs = docComments(raw);
  // The Rust struct says what a request may leave out: the generated TypeScript marks only the
  // fields with `#[ts(optional)]` or an `Option` with `?`.
  const rust = rustStruct(name);
  const structDefault = Boolean(rust && rust.default);
  const src = raw
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/[^\n]*/g, "");
  const head = new RegExp(`export\\s+type\\s+${name}\\s*=`).exec(src);
  if (!head) return null;
  const from = head.index + head[0].length;
  const rhs = src.slice(from, aliasEnd(src, from)).trim();

  const fields = [];
  const byName = new Map();
  const literals = [];
  const tags = [];
  const refs = new Set();
  let plain = "";
  let objectVariants = 0;
  for (const variant of splitTopLevel(rhs, "|")) {
    const v = variant.trim();
    if (v.startsWith("{") && v.endsWith("}")) {
      objectVariants++;
      for (const part of splitTopLevel(v.slice(1, -1), ",")) {
        if (!part) continue;
        const bits = splitTopLevel(part, ":");
        if (bits.length < 2) continue;
        let fieldName = bits[0].trim();
        const optional = fieldName.endsWith("?");
        if (optional) fieldName = fieldName.slice(0, -1).trim();
        fieldName = fieldName.replace(/^["']|["']$/g, "");
        if (!fieldName) continue;
        const raw = bits.slice(1).join(":").trim();
        // A tagged union names its variant with a literal: that literal is one of the values a
        // reader may send.
        const tag = /^"([^"]*)"$/.exec(raw);
        if (tag && !tags.includes(tag[1])) tags.push(tag[1]);
        const info = readerFieldType(raw);
        info.refs.forEach((r) => refs.add(r));
        let field = byName.get(fieldName);
        if (!field) {
          const rf = rust ? rust.fields.get(fieldName) : null;
          field = {
            name: fieldName,
            type: info.type,
            optional,
            // A request may leave out a field with a serde default, an `Option` field, or any
            // field of a struct that carries `#[serde(default)]`.
            requestOptional: Boolean(rf && (rf.default || rf.option || structDefault)),
            seen: 0,
            doc: docs.get(fieldName) || "",
          };
          byName.set(fieldName, field);
          fields.push(field);
        }
        field.seen++;
        if (field.type !== info.type && !field.type.includes(info.type)) {
          field.type = `${field.type} or ${info.type}`;
        }
        if (!optional) field.optional = false;
      }
      continue;
    }
    if (/^"[^"]*"$/.test(v)) { literals.push(v); continue; }
    if (/^\d+$/.test(v)) { literals.push(`"${v}"`); plain = plain ? `${plain} | ${v}` : v; continue; }
    plain = plain ? `${plain} | ${v}` : v;
    readerFieldType(v).refs.forEach((r) => refs.add(r));
  }
  const meaning = (value) => ENUM_VALUE_MEANINGS[`${name}.${value}`] || "";
  const values = literals.map((literal) => {
    const value = literal.replace(/^"|"$/g, "");
    return { value, meaning: meaning(value) };
  });
  // A tagged union's values are the names of its variants.
  const tagValues = tags.map((value) => ({ value, meaning: meaning(value) }));
  if (!fields.length) {
    let note;
    if (literals.length && !plain) note = `One of: ${literals.join(", ")}.`;
    else if (plain) note = `A plain value: ${readerFieldType(plain).type}.`;
    else note = "A free-form JSON object.";
    return { fields: [], note, values, refs: [...refs] };
  }
  return {
    fields: fields.map((f) => ({
      name: f.name,
      type: f.type,
      optional: f.optional || f.seen < objectVariants,
      requestOptional: f.requestOptional,
      doc: f.doc,
    })),
    note: tagValues.length ? `One of: ${tagValues.map((v) => `"${v.value}"`).join(", ")}.` : "",
    values: tagValues,
    refs: [...refs],
  };
}

/** The word a reader sees for a Rust container or primitive used as a whole request or answer. */
function readerType(name) {
  const vector = /^Vec\s*<\s*([\w:]+)\s*>$/.exec(name);
  if (vector) return `array of ${typeName(vector[1])}`;
  const scalar = {
    String: "string", str: "string", bool: "boolean",
    u8: "integer", u16: "integer", u32: "integer", u64: "integer", usize: "integer",
    i8: "integer", i16: "integer", i32: "integer", i64: "integer", isize: "integer",
    f32: "number", f64: "number",
  };
  return scalar[name] || name;
}

/** Notes for types the scanned source cannot carry on its own. */
const TYPE_NOTES = {
  FailSensorRequest: "The temperature source to mark as failed and whether to mark it.",
  Rgb8: "A colour as a hex string such as #ff0000.",
  DeviceId: "The id of a device, such as `openrgb:2a3b4c5d` or `display:5f6a:7b8c`.",
  RenderScale: "The render scale as a percentage of the logical canvas: 25, 50 or 100.",
};

/** Shapes for the reader-facing labels that have no ts-rs file of their own. */
function specialShape(name) {
  switch (name) {
    case "Props": return { fields: [], note: "A map of property id to PropValue." };
    case "Binary thumbnail": return { fields: [], note: "Binary bytes: three 16-bit little-endian counts (width, height, frame count) followed by raw RGB frames." };
    case "JSON object": return { fields: [], note: "A JSON object with the fields shown beside the request or answer." };
    case "SceneAnswer": return { fields: [], note: "A saved scene object, or a dry-run answer with saved set to false." };
    default: return null;
  }
}

/** Every named type used by the reference, followed transitively to its own definition. */
function collectShapes(types, structFields, queryTypes) {
  const queue = [];
  const seenNames = new Set();
  const add = (name) => {
    const s = String(name).trim();
    if (!s || s === "none" || s === "WebSocket (binary frames)") return;
    // A reader word such as "array of string" has no definition; a named label does.
    if (/^(?:[A-Z]|array of [A-Z])/.test(s) && !seenNames.has(s)) { seenNames.add(s); queue.push(s); }
    for (const ref of referencedNames(s)) {
      if (!seenNames.has(ref)) { seenNames.add(ref); queue.push(ref); }
    }
  };
  for (const name of [...types, ...queryTypes]) add(name);

  const shapes = {};
  // The fields a request may leave out, by type name: a field with a serde default, an `Option`
  // field, or any field of a struct that carries `#[serde(default)]`.
  const requestOptional = new Map();
  const visited = new Set();
  while (queue.length) {
    const name = queue.shift();
    if (visited.has(name)) continue;
    visited.add(name);

    const special = specialShape(name);
    if (special) { shapes[name] = special; continue; }

    const arrayOf = /^array of ([A-Z]\w*)$/.exec(name);
    if (arrayOf) {
      add(arrayOf[1]);
      const inner = shapes[arrayOf[1]] || tsDefinition(arrayOf[1]);
      shapes[name] = {
        fields: inner?.fields || [],
        note: `A list of ${arrayOf[1]} objects, with the fields below.`,
      };
      continue;
    }

    let shape = tsDefinition(name);
    if (shape) {
      shape.refs.forEach(add);
    } else if (structFields.has(name)) {
      // Daemon-only query structs and bodies have no ts-rs file; their Rust fields are accurate.
      const rust = rustStruct(name);
      const structDefault = Boolean(rust && rust.default);
      const fields = structFields.get(name).map((f) => {
        const info = readerFieldType(f.type);
        info.refs.forEach(add);
        const rf = rust ? rust.fields.get(f.name) : null;
        const out = {
          name: f.name,
          type: info.type,
          optional: f.optional,
          requestOptional: Boolean(rf && (rf.default || rf.option || structDefault)),
        };
        if (f.doc && cleanDoc(f.doc).length > 3) out.doc = cleanDoc(f.doc);
        return out;
      });
      shape = { fields, note: TYPE_NOTES[name] || "" };
    } else {
      shape = { fields: [], note: "A free-form JSON object." };
    }
    if (TYPE_NOTES[name]) shape.note = TYPE_NOTES[name];
    if (shape.fields?.length) {
      for (const field of shape.fields) {
        const meaning = meaningFor(name, field);
        if (meaning) field.meaning = meaning;
      }
    }
    const { refs, doc, ...published } = shape;
    for (const field of published.fields || []) delete field.doc;
    if (!published.note) delete published.note;
    if (!published.values?.length) delete published.values;
    shapes[name] = published;
  }
  // An array shape borrows the inner type's finished fields, meanings and enum values.
  for (const [name, shape] of Object.entries(shapes)) {
    const arrayOf = /^array of ([A-Z]\w*)$/.exec(name);
    if (arrayOf && shapes[arrayOf[1]]) {
      shape.fields = shapes[arrayOf[1]].fields;
      if (shapes[arrayOf[1]].values) shape.values = shapes[arrayOf[1]].values;
    }
    // The optional fields are read after the array shape took the inner type's fields.
    const optional = (shape.fields || []).filter((f) => f.requestOptional).map((f) => f.name);
    if (optional.length) requestOptional.set(name, new Set(optional));
  }
  // The flag is a build detail: the published shape says optional or it does not.
  for (const shape of Object.values(shapes)) {
    for (const field of shape.fields || []) delete field.requestOptional;
  }
  return { shapes, requestOptional };
}

/**
 * A line under the curl command that tells the reader what the example needs. The published
 * examples are a sequence a reader can follow, so each one says where its values come from.
 */
const EXAMPLE_NOTES = {
  "CHAIN_SETUP:POST": "The answer carries the session id. The setup requests that follow send it in their body.",
  "CHAIN_SETUP:DELETE": "Send the session id the setup POST answered with.",
  "CHAIN_SETUP_COMMIT:POST": "Send the session id the setup POST answered with.",
  "CHAIN_SETUP_EMPTY:POST": "Send the session id the setup POST answered with.",
  "CHAIN_SETUP_LIGHT:POST": "Send the session id the setup POST answered with.",
  "PREVIEW:GET": "Needs a curl built with WebSocket support (7.86 or later); the JavaScript sample is the main example.",
  "PREVIEW_GROUP:GET": "Needs a curl built with WebSocket support (7.86 or later); the JavaScript sample is the main example.",
  "PREVIEW_LIGHTS:GET": "Needs a curl built with WebSocket support (7.86 or later); the JavaScript sample is the main example.",
};

/** One copy-pasteable curl example for a route and method. */
function curlExample(entry, method) {
  const path = entry.path.replace(/\{(\w+)\}/g, "<$1>");
  const params = entry.queryParams[method] || [];
  const websocket = entry.responseType[method] === "WebSocket (binary frames)";
  const query = params.length ? `?${params.map((p) => `${p.name}=${p.value}`).join("&")}` : "";
  const scheme = websocket ? "ws" : "http";
  // A WebSocket handshake is a GET by default; naming the method invites a copy that fails.
  let line = websocket ? "curl" : `curl -X ${method}`;
  if (entry.route !== "STATUS" && !websocket) line += ' -H "Authorization: Bearer $TOKEN"';
  const requestType = entry.requestType[method];
  if (requestType !== "none") {
    const key = `${entry.route}:${method}`;
    const body = EXAMPLE_BODIES[key];
    if (!body) throw new Error(`no hand-written example body for ${key} (${entry.path} ${method})`);
    // The JSON body extractor answers 415 without this header, even for an empty object.
    line += ' -H "Content-Type: application/json"';
    line += ` -d '${JSON.stringify(body)}'`;
  }
  line += ` "${scheme}://127.0.0.1:9430${path}${query}"`;
  const note = EXAMPLE_NOTES[`${entry.route}:${method}`];
  return note ? `${LISTENER_NOTE}\n${line}\n# ${note}` : `${LISTENER_NOTE}\n${line}`;
}

/** Builds the reference. Pure: same sources in, same object out. */
function generateInternal() {
  const apiSrc = fs.readFileSync(API_RS, "utf8");
  const modSrc = fs.readFileSync(MOD_RS, "utf8");
  const routes = parseRoutes(apiSrc);
  const structFields = parseStructFields();
  const handlers = parseHandlers(structFields, parseAliases(apiSrc));
  const calls = parseRouter(modSrc);

  // Group by path: one entry per route path, with one record per method.
  const byPath = new Map();
  let inlineCount = 0;
  for (const call of calls) {
    const comma = topLevelComma(call);
    if (comma === -1) throw new Error(`a .route() call has no arguments: ${call}`);
    const pathExpr = call.slice(0, comma).trim();
    const rest = call.slice(comma + 1);
    const named = /^routes::(\w+)$/.exec(pathExpr);
    let routeName, routePath;
    if (named) {
      routeName = named[1];
      if (routeName === "PREFIX") continue;
      const found = routes.get(routeName);
      if (!found) throw new Error(`router uses unknown route constant ${routeName}`);
      routePath = found.path;
    } else if (/^"[^"]+"$/.test(pathExpr)) {
      routeName = INLINE_ROUTE_KEY;
      routePath = pathExpr.slice(1, -1); // inline literal, e.g. the fail-sensor route
      inlineCount++;
    } else {
      throw new Error(`unrecognised route argument: ${pathExpr}`);
    }

    const methods = [];
    const mre = /\b(get|post|put|patch|delete)\s*\(\s*([A-Za-z0-9_:]+)\s*\)/g;
    let mm;
    while ((mm = mre.exec(rest))) {
      methods.push({ method: mm[1].toUpperCase(), handler: mm[2] });
    }
    if (methods.length === 0) throw new Error(`no method found for ${routePath}`);

    if (!byPath.has(routePath)) {
      byPath.set(routePath, {
        route: routeName,
        path: routePath,
        title: titled(routePath),
        area: "",
        methods: [],
        pathParams: [...routePath.matchAll(/\{(\w+)\}/g)].map((p) => pathParam(routeName, p[1])),
        descriptions: {},
        examples: {},
        requestType: {},
        responseType: {},
        requestShape: {},
        responseShape: {},
        queryParams: {},
        queryType: {},
      });
    }
    const entry = byPath.get(routePath);
    for (const { method, handler } of methods) {
      const detail = handlers.get(handler);
      if (!detail) throw new Error(`no handler source found for ${handler} (${routePath})`);
      entry.methods.push(method);
      entry.descriptions[method] = described(routeName, method, routePath);
      const key = `${routeName}:${method}`;
      for (const [direction, raw] of [
        ["request", detail.request],
        ["response", detail.websocket ? "WebSocket (binary frames)" : detail.response],
      ]) {
        let type = readerType(raw);
        if (raw === "serde_json::Value") type = "JSON object";
        if (direction === "request" && key in REQUEST_TYPE_OVERRIDES) type = REQUEST_TYPE_OVERRIDES[key];
        if (direction === "response" && key in RESPONSE_OVERRIDES) type = RESPONSE_OVERRIDES[key];
        if (type === "JSON object") {
          const shapeKey = `${key}:${direction}`;
          const shape = HAND_SHAPES[shapeKey];
          if (!shape) throw new Error(`no hand-written JSON object shape for ${shapeKey} (${routePath} ${method})`);
          entry[`${direction}Shape`][method] = shape;
        }
        entry[`${direction}Type`][method] = type;
      }
      entry.queryParams[method] = detail.queryParams;
      if (detail.queryType) entry.queryType[method] = detail.queryType;
      const module = handler.split("::")[0];
      entry.area = ROUTE_AREA_OVERRIDES[routeName] || AREAS[module] || "Other";
    }
  }

  const entries = [...byPath.values()];
  const titles = entries.map((e) => e.title);
  if (new Set(titles).size !== titles.length) {
    throw new Error(`route titles are not unique: ${titles.join(", ")}`);
  }
  for (const entry of entries) {
    entry.methods.sort((a, b) => METHOD_ORDER.indexOf(a) - METHOD_ORDER.indexOf(b));
    for (const field of ["descriptions", "requestType", "responseType", "requestShape", "responseShape", "queryParams", "queryType"]) {
      const src = entry[field];
      const out = {};
      for (const key of entry.methods) if (key in src) out[key] = src[key];
      entry[field] = out;
    }
  }
  entries.sort((a, b) => a.path.localeCompare(b.path));

  // Cross-check: every route constant in the spine (bar PREFIX) and every inline route is present.
  const spinePaths = [...routes.values()].filter((r) => r.name !== "PREFIX").map((r) => r.path);
  const present = new Set(entries.map((e) => e.path));
  const missing = spinePaths.filter((p) => !present.has(p));
  if (missing.length) throw new Error(`spine routes missing from the reference: ${missing.join(", ")}`);

  // The status handler answers a bare axum Response; its body resolves to Status. Refuse to
  // publish a reference that slipped back to saying the body is the Response wrapper.
  const status = entries.find((e) => e.route === "STATUS");
  if (!status || status.responseType.GET !== "Status") {
    throw new Error(`GET /api/v1/status must resolve to Status, got ${status?.responseType?.GET}`);
  }

  const types = new Set();
  const queryTypes = new Set();
  for (const entry of entries) {
    for (const method of entry.methods) {
      types.add(entry.requestType[method]);
      types.add(entry.responseType[method]);
      if (entry.queryType[method]) queryTypes.add(entry.queryType[method]);
    }
  }
  const { shapes, requestOptional } = collectShapes(types, structFields, queryTypes);

  // Every route publishes the shape of its request and of its answer. A named type's shape is
  // that type's own table, so the two cannot drift; a plain JSON object carries the hand-written
  // shape the generator already knows.
  for (const entry of entries) {
    for (const method of entry.methods) {
      for (const direction of ["request", "response"]) {
        const type = entry[`${direction}Type`][method];
        if (!type || type === "none" || type === "WebSocket (binary frames)") continue;
        if (entry[`${direction}Shape`][method]) continue;
        const shape = shapes[type];
        if (!shape) continue;
        // A request may leave out a field with a serde default, an `Option` field, or any field
        // of a struct that carries `#[serde(default)]`. An answer carries the fields it sends.
        const optionalFields = requestOptional.get(type) || requestOptional.get(/^array of ([A-Z]\w*)$/.exec(type)?.[1] || "") || new Set();
        const fields = (shape.fields || []).map((field) => {
          const out = { name: field.name, type: field.type, meaning: field.meaning };
          const optional = field.optional || (direction === "request" && optionalFields.has(field.name));
          if (optional) out.optional = true;
          return out;
        });
        const published = { label: type, fields };
        if (shape.note) published.note = shape.note;
        entry[`${direction}Shape`][method] = published;
      }
    }
  }

  // Every published field and every enum value must say what it is. A gap fails generation.
  const gaps = [];
  for (const [name, shape] of Object.entries(shapes)) {
    for (const field of shape.fields || []) {
      if (!field.meaning) gaps.push(`${name}.${field.name}`);
    }
    for (const value of shape.values || []) {
      if (!value.meaning) gaps.push(`${name}.${value.value}`);
    }
  }
  if (gaps.length) throw new Error(`fields and values with no reader meaning: ${gaps.join(", ")}`);

  for (const entry of entries) {
    for (const method of entry.methods) {
      entry.examples[method] = curlExample(entry, method);
    }
    // Route constant names are a build detail. The published entry has the path and the area.
    delete entry.route;
  }

  return {
    reference: { entries, types: shapes },
    spineRouteCount: spinePaths.length,
    inlineCount,
  };
}

/** The keys of the hand-written meaning tables, for the tests that forbid bare-name keys. */
export function meaningKeys() {
  return { fields: Object.keys(FIELD_MEANINGS), values: Object.keys(ENUM_VALUE_MEANINGS) };
}

/** The published reference. Counts and route constants stay inside the build step. */
export function generate() {
  return generateInternal().reference;
}

/** Formats the document exactly as the committed file. */
export function serialize(reference) {
  return JSON.stringify(reference, null, 2) + "\n";
}

function check() {
  // core.autocrlf turns this file into CRLF on checkout on Windows; compare text, not bytes.
  const current = fs.readFileSync(OUT, "utf8").replace(/\r\n/g, "\n");
  const built = generateInternal();
  const expected = serialize(built.reference);
  const count = built.reference.entries.length;
  if (current === expected) {
    console.log(`api-reference: up to date (${count} routes, ${built.spineRouteCount} spine + ${built.inlineCount} inline)`);
    return 0;
  }
  console.error("api-reference: stale or missing a route. Regenerate with: node site/scripts/gen-api-reference.mjs");
  const a = current.split(/\r?\n/);
  const b = expected.split(/\r?\n/);
  let shown = 0;
  for (let i = 0; i < Math.max(a.length, b.length) && shown < 20; i++) {
    if (a[i] !== b[i]) {
      console.error(`  line ${i + 1}: file=${JSON.stringify(a[i] ?? "")} generated=${JSON.stringify(b[i] ?? "")}`);
      shown++;
    }
  }
  return 1;
}

const invokedDirectly = process.argv[1] && import.meta.url === new URL(`file://${path.resolve(process.argv[1])}`).href;
if (invokedDirectly) {
  if (process.argv.includes("--check")) {
    process.exitCode = check();
  } else {
    const built = generateInternal();
    fs.mkdirSync(path.dirname(OUT), { recursive: true });
    fs.writeFileSync(OUT, serialize(built.reference));
    console.log(`api-reference: wrote ${built.reference.entries.length} routes (${built.spineRouteCount} spine + ${built.inlineCount} inline) to site/content/docs/api-reference.json`);
  }
}
