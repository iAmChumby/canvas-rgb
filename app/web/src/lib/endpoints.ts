// Typed calls for every route the editor uses. Paths come from the spine's routes.ts and the
// shapes from the generated proto types; nothing here re-declares an API type.

import { apiConfig, request, requestBinary, type RequestOptions } from "@/lib/api";
import { isDemo } from "@/lib/host";
import { routes } from "@/proto/routes";
import type { Status } from "@/proto/Status";
import type { OutputState } from "@/proto/OutputState";
import type { PatchOutput } from "@/proto/PatchOutput";
import type { DevicesResponse } from "@/proto/DevicesResponse";
import type { PatchZone } from "@/proto/PatchZone";
import type { IdentifyRequest } from "@/proto/IdentifyRequest";
import type { EffectsResponse } from "@/proto/EffectsResponse";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { ActivateEffect } from "@/proto/ActivateEffect";
import type { PresetCreate } from "@/proto/PresetCreate";
import type { PresetReplace } from "@/proto/PresetReplace";
import type { PresetsResponse } from "@/proto/PresetsResponse";
import type { ImportEffect } from "@/proto/ImportEffect";
import type { ImportEffectResponse } from "@/proto/ImportEffectResponse";
import type { EffectSource } from "@/proto/EffectSource";
import type { EffectPreviewRequest } from "@/proto/EffectPreviewRequest";
import type { EffectPreviewResponse } from "@/proto/EffectPreviewResponse";
import type { DraftRequest } from "@/proto/DraftRequest";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { SetFavorite } from "@/proto/SetFavorite";
import type { Props } from "@/proto/props";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { PeripheralAccess } from "@/proto/PeripheralAccess";
import type { ReadProgress } from "@/proto/ReadProgress";
import type { PeripheralRow } from "@/lib/peripherals";
import type { ScenesResponse } from "@/proto/ScenesResponse";
import type { Scene } from "@/proto/Scene";
import type { SceneCreate } from "@/proto/SceneCreate";
import type { ScenePut } from "@/proto/ScenePut";
import type { SceneActivePut } from "@/proto/SceneActivePut";
import type { GroupBrightnessPut } from "@/proto/GroupBrightnessPut";
import type { GroupMove } from "@/proto/GroupMove";
import type { GroupMoved } from "@/proto/GroupMoved";
import type { ScenesWrite } from "@/proto/ScenesWrite";
import type { LightsResponse } from "@/proto/LightsResponse";
import type { LightView } from "@/proto/LightView";
import type { LightPatch } from "@/proto/LightPatch";
import type { FlashRequest } from "@/proto/FlashRequest";
import type { ChainsResponse } from "@/proto/ChainsResponse";
import type { SetupStarted } from "@/proto/SetupStarted";
import type { SetupLight } from "@/proto/SetupLight";
import type { SetupEmpty } from "@/proto/SetupEmpty";
import type { SetupCommit } from "@/proto/SetupCommit";
import type { SetupCommitted } from "@/proto/SetupCommitted";
import type { FansStatus } from "@/proto/FansStatus";
import type { HotLimits } from "@/proto/HotLimits";
import type { FanMode } from "@/proto/FanMode";
import type { PutFanHeaderAvailability } from "@/proto/PutFanHeaderAvailability";
import type { FanProfile } from "@/proto/FanProfile";
import type { FanProfileName } from "@/proto/FanProfileName";
import type { WhiteBalance } from "@/proto/WhiteBalance";
import type { WhiteBalances } from "@/proto/WhiteBalances";
import type { Settings } from "@/proto/Settings";
import type { DisplayConfig } from "@/proto/DisplayConfig";
import type { DisplayStatus } from "@/proto/DisplayStatus";
import type { SetDisplayValue } from "@/proto/SetDisplayValue";
import type { ExportBundle } from "@/proto/ExportBundle";
import type { Stack } from "@/proto/Stack";
import type { StackDetail } from "@/proto/StackDetail";
import type { StacksResponse } from "@/proto/StacksResponse";
import type { DeleteStackResponse } from "@/proto/DeleteStackResponse";
import type { StackPreviewRequest } from "@/proto/StackPreviewRequest";
import type { StackPreviewResponse } from "@/proto/StackPreviewResponse";
import type { Pattern } from "@/proto/Pattern";
import type { PatternsResponse } from "@/proto/PatternsResponse";
import type { DeletePatternResponse } from "@/proto/DeletePatternResponse";
import type { AudioSource } from "@/proto/AudioSource";
import type { AudioSources } from "@/proto/AudioSources";
import type { ResolvedAudioSource } from "@/proto/ResolvedAudioSource";
import type { AudioTestLease } from "@/proto/AudioTestLease";
import type { AudioTestRequest } from "@/proto/AudioTestRequest";
import type { AudioTestState } from "@/proto/AudioTestState";

/**
 * The chain routes (`routes::CHAINS`, `routes::CHAIN_SETUP*`). `proto/routes.ts` mirrors the
 * spine by hand and does not carry them yet, so they are spelled here.
 */
const CHAINS = "/api/v1/chains";
/**
 * The pattern routes (`routes::PATTERNS`, `routes::PATTERN`, 0.16.0). `proto/routes.ts` mirrors the
 * spine by hand and does not carry them yet, so they are spelled here, as the chains above are.
 */
const PATTERNS = "/api/v1/patterns";
const patternPath = (id: string): string => `${PATTERNS}/${encodeURIComponent(id)}`;
/** `POST` starts a session on one header, `DELETE` cancels it. `{zone}` is the zone index. */
const chainSetup = (device: string, zone: number): string =>
  `${CHAINS}/${encodeURIComponent(device)}/${zone}/setup`;

/**
 * Cancels a session with a request the browser finishes even when the page is already going:
 * closing the window must put the header's length back. `keepalive` is a `fetch` option and the
 * shared `request` carries only the caller's abort signal, so this one path takes the address and
 * the token from the tray itself. Nothing is read from the answer, and a failure is quiet: the
 * page is closing, and the daemon ends a session on its own timeout anyway.
 */
async function cancelAndLeave(path: string, body: SetupEmpty): Promise<void> {
  if (isDemo()) {
    await request<unknown>("DELETE", path, body).catch(() => undefined);
    return;
  }
  try {
    const config = await apiConfig();
    await fetch(config.base_url + path, {
      method: "DELETE",
      headers: { Authorization: `Bearer ${config.token}`, "Content-Type": "application/json" },
      body: JSON.stringify(body),
      cache: "no-store",
      keepalive: true,
    });
  } catch {
    // The window is going; the daemon ends the session by its own timeout.
  }
}

export const api = {
  status: () => request<Status>("GET", routes.status),

  output: () => request<OutputState>("GET", routes.output),
  patchOutput: (body: PatchOutput) => request<OutputState>("PATCH", routes.output, body),


  devices: () => request<DevicesResponse>("GET", routes.devices),
  /** The same list, asked for again on purpose (the onboarding's "Rescan"). */
  rescanDevices: () => request<DevicesResponse>("GET", `${routes.devices}?rescan=1`),
  patchZone: (id: string, zone: number, body: PatchZone) =>
    request<unknown>("PATCH", routes.deviceZone(id, zone), body),
  identify: (id: string, body: IdentifyRequest) =>
    request<unknown>("POST", routes.deviceIdentify(id), body),

  /** Every device's white balance (#50): gains 0..1 per channel; a device not listed is neutral. */
  getWhiteBalance: () => request<WhiteBalances>("GET", routes.whiteBalance),
  /** Sets one device's white balance; 422 when any gain is outside 0..1. Answers WhiteBalances. */
  putWhiteBalance: (id: string, wb: WhiteBalance) =>
    request<WhiteBalances>("PUT", routes.deviceWhiteBalance(id), wb),
  /** Puts one device back to neutral. Answers WhiteBalances. */
  deleteWhiteBalance: (id: string) =>
    request<WhiteBalances>("DELETE", routes.deviceWhiteBalance(id)),

  effects: () => request<EffectsResponse>("GET", routes.effects),
  effect: (id: string) => request<EffectDetail>("GET", routes.effect(id)),
  /** Binary: u16 LE width, u16 LE height, u16 LE frame count, then frames x w x h x 3 RGB bytes. */
  effectThumbnail: (id: string) => requestBinary(routes.effectThumbnail(id)),
  activateEffect: (id: string, body: ActivateEffect) =>
    request<unknown>("POST", routes.effectActivate(id), body),
  /**
   * An effect's starting values: what the next group to play it starts from. A group that already
   * plays the effect keeps its own values; no scene changes.
   */
  patchEffectDefaults: (id: string, props: Props) =>
    request<unknown>("PATCH", routes.effectDefaults(id), props),
  setFavorite: (id: string, body: SetFavorite) =>
    request<unknown>("PUT", routes.effectFavorite(id), body),
  /** Saves the values as a user preset of the effect. */
  createPreset: (id: string, body: PresetCreate) =>
    request<PresetsResponse>("POST", routes.effectPresets(id), body),
  /** Overwrites and/or renames a user preset. `expected` is what the window last saw. */
  replacePreset: (id: string, name: string, body: PresetReplace) =>
    request<PresetsResponse>("PUT", routes.effectPreset(id, name), body),
  deletePreset: (id: string, name: string) =>
    request<PresetsResponse>("DELETE", routes.effectPreset(id, name)),
  /** Adds a user effect from an HTML file the reader chose. */
  importEffect: (body: ImportEffect) => request<ImportEffectResponse>("POST", routes.effectsImport, body),
  /** Removes a user effect. Only the `html:` ids can go. */
  deleteEffect: (id: string, keepPresets = false) =>
    request<unknown>("DELETE", `${routes.effect(id)}${keepPresets ? "?keep_presets=true" : ""}`),

  /** The HTML the editor opens and the file name Save writes. A Rust built-in is a 404. */
  effectSource: (id: string) => request<EffectSource>("GET", routes.effectSource(id)),
  /** Renders a draft headless: its schema, the runtime's errors and its loop.
   *  `opts.signal` ends the render when the editor no longer wants the answer. */
  previewEffect: (body: EffectPreviewRequest, opts?: RequestOptions) =>
    request<EffectPreviewResponse>("POST", routes.effectPreview, body, opts),
  /** The draft on the lights now, for the banner a reload opens with. */
  draft: () => request<DraftStatus>("GET", routes.effectsDraft),
  /** Starts the draft, or renews its lease with the current source and props. */
  putDraft: (body: DraftRequest) => request<DraftStatus>("PUT", routes.effectsDraft, body),
  /** Stops the draft: the effect it took over from comes back. The page's epoch goes with it, so
   *  a page that no longer owns the lights ends nothing and is answered `applied: false`. */
  deleteDraft: (epoch?: string) =>
    request<DraftStatus>(
      "DELETE",
      epoch ? `${routes.effectsDraft}?epoch=${encodeURIComponent(epoch)}` : routes.effectsDraft,
    ),

  /** Every peripheral the adapters see. The list opens nothing. */
  peripherals: () => request<{ peripherals: PeripheralRow[] }>("GET", routes.peripherals),
  /**
   * What one peripheral is, what it has and what it holds now. The daemon answers with the
   * values it kept; `fresh` makes it read the device (a read is a series of short report
   * exchanges, and the device may stutter briefly while it reads).
   */
  peripheral: (id: string, opts?: { fresh?: boolean }) =>
    request<PeripheralDetail>("GET", routes.peripheral(id) + (opts?.fresh ? "?fresh=true" : "")),
  /** Writes the changed keys to the device. Only while writes are on. */
  patchPeripheral: (id: string, body: Props) => request<Props>("PATCH", routes.peripheral(id), body),
  /** What the daemon may do with one peripheral. */
  putPeripheralAccess: (id: string, body: PeripheralAccess) =>
    request<PeripheralAccess>("PUT", routes.peripheralAccess(id), body),
  /** The stage of the read of one peripheral that is open now. Answers at once, even mid-read. */
  peripheralRead: (id: string) => request<ReadProgress>("GET", routes.peripheralRead(id)),

  /** Every saved stack, summarised. */
  stacks: () => request<StacksResponse>("GET", routes.stacks),
  /** One stack and what each of its layers is doing. */
  stack: (id: string) => request<StackDetail>("GET", routes.stack(id)),
  /** The whole document in, the saved stack out (409 on a taken name, 422 on a broken value). */
  putStack: (id: string, body: Stack) => request<Stack>("PUT", routes.stack(id), body),
  /** Removes a stack and what points at it; `force` is needed while it runs. */
  deleteStack: (id: string, force = false) =>
    request<DeleteStackResponse>("DELETE", `${routes.stack(id)}${force ? "?force=true" : ""}`),
  /** Builds or updates the preview engine's stack; a superseded PUT answers `applied: false`. */
  putStackPreview: (body: StackPreviewRequest) =>
    request<StackPreviewResponse>("PUT", routes.stackPreview, body),
  /** Drops the preview and clears the sequence watermark. The page's epoch goes with it, so a
   *  page that no longer owns the preview drops nothing and is answered `applied: false`. */
  deleteStackPreview: (epoch?: string) =>
    request<{ ok: boolean; running: boolean; applied?: boolean }>(
      "DELETE",
      epoch ? `${routes.stackPreview}?epoch=${encodeURIComponent(epoch)}` : routes.stackPreview,
    ),

  /**
   * Every saved pattern, whole (0.16.0). A pattern is a colour per LED of one device: the editor
   * draws its dots from these colours, so there is no summary shape and no thumbnail.
   */
  patterns: () => request<PatternsResponse>("GET", PATTERNS),
  /** One pattern, by its `pattern:` id or its bare name. */
  pattern: (id: string) => request<Pattern>("GET", patternPath(id)),
  /** The whole document in, the saved pattern out (409 on a taken name, 422 on a broken frame). */
  putPattern: (id: string, body: Pattern) => request<Pattern>("PUT", patternPath(id), body),
  /** Removes a pattern and its favourite; `force` is needed while a scene still plays it. */
  deletePattern: (id: string, force = false) =>
    request<DeletePatternResponse>(
      "DELETE",
      `${patternPath(id)}${force ? "?force=true" : ""}`,
    ),
  /**
   * Puts a pattern on the lights as a draft, one stroke per PUT: the draft's own epoch and a rising
   * `seq`, so a stroke that arrives late never replaces a newer one. A pattern draft carries no
   * HTML and no props.
   */
  putPatternDraft: (body: { pattern: Pattern; epoch: string; seq: number }) =>
    request<DraftStatus>("PUT", routes.effectsDraft, {
      html: "",
      props: {},
      ...body,
    } satisfies DraftRequest),

  scenes: {
    /** Every scene, the active one's id and how its groups are doing. */
    list: () => request<ScenesResponse>("GET", routes.scenes),
    /** A new scene: a copy of `copy_of`, or one group of every light set to Off. */
    create: (body: SceneCreate) => request<Scene>("POST", routes.scenes, body),
    /** Replaces a scene whole. 409 (the current scene in the body) when `expected_rev` is stale. */
    put: (id: string, body: ScenePut) => request<Scene>("PUT", routes.scene(id), body),
    /** 409 for the last scene. */
    remove: (id: string) => request<unknown>("DELETE", routes.scene(id)),
    /** Sets a scene, with a fade. */
    setActive: (id: string) =>
      request<ScenesResponse>("PUT", routes.sceneActive, { id } satisfies SceneActivePut),
    /** New values for one group's effect, no rebuild. Saved; answers the scene with its new rev. */
    patchGroupProps: (sceneId: string, groupId: string, props: Props) =>
      request<Scene>("PATCH", routes.groupProps(sceneId, groupId), props),
    /** One group's brightness. Saved; answers the scene with its new rev. */
    putGroupBrightness: (sceneId: string, groupId: string, brightness: number) =>
      request<Scene>("PUT", routes.groupBrightness(sceneId, groupId), {
        brightness,
      } satisfies GroupBrightnessPut),
    /**
     * Moves or copies one group to another scene (#368). One route, one save: both scenes land
     * or neither. 409 with `which` ("from" or "to") and that scene when a rev is stale, 422 when
     * a scene rule is broken (the HTML-group limit is the daemon's own check).
     */
    moveGroup: (sceneId: string, groupId: string, body: GroupMove) =>
      request<GroupMoved>("POST", routes.groupMove(sceneId, groupId), body),
    /**
     * Writes several whole scenes and/or the order of the scene bar in one save (#368 undo,
     * #371 order). 409 with the scene or the order that is stale, 422 when a shape is broken.
     */
    write: (body: ScenesWrite) => request<ScenesResponse>("PUT", routes.scenes, body),
  },
  lights: {
    list: () => request<LightsResponse>("GET", routes.lights),
    patch: (id: string, body: LightPatch) => request<LightView>("PATCH", routes.light(id), body),
    /** The named lights white and every other LED black; `stop` ends it. */
    flash: (body: FlashRequest) => request<unknown>("POST", routes.lightsFlash, body),
  },
  chains: {
    /** Every resizable header and what is set up on it (a daemon without the routes answers 404). */
    list: () => request<ChainsResponse>("GET", CHAINS),
    /** Holds the lights, raises the header to its maximum and opens a session. */
    start: (device: string, zone: number) =>
      request<SetupStarted>("POST", chainSetup(device, zone)),
    /** The coloured runs to show. Also the heartbeat: a session ends 6 s after the last one. */
    light: (device: string, zone: number, body: SetupLight) =>
      request<unknown>("POST", `${chainSetup(device, zone)}/light`, body),
    /**
     * Ends the session and puts the length back. `keepalive` finishes the request even when the
     * page goes away with it, which is what a closed window is.
     */
    cancel: (device: string, zone: number, body: SetupEmpty, opts?: { keepalive?: boolean }) =>
      opts?.keepalive
        ? cancelAndLeave(chainSetup(device, zone), body)
        : request<unknown>("DELETE", chainSetup(device, zone), body),
    /** Nothing is plugged into this header: it is marked empty rather than set up. */
    empty: (device: string, zone: number, body: SetupEmpty) =>
      request<unknown>("POST", `${chainSetup(device, zone)}/empty`, body),
    /** Resizes to the parts' LEDs and saves them, all or nothing; answers the new lights. */
    commit: (device: string, zone: number, body: SetupCommit) =>
      request<SetupCommitted>("POST", `${chainSetup(device, zone)}/commit`, body),
  },

  fans: () => request<FansStatus>("GET", routes.fans),
  /** The user's "don't exceed" temperatures, one per kind of source (`null` = automatic). */
  putFanHot: (body: HotLimits) => request<FansStatus>("PUT", routes.fanHot, body),
  patchFanHeader: (header: string, mode: FanMode) =>
    request<unknown>("PATCH", routes.fanHeader(header), mode),
  /** The owner's word on whether a header is on this board (#146): `false` "Not on my board",
   *  `true` "Show anyway", `null` back to the board table. The answer is the new fan status. */
  putFanHeaderAvailability: (header: string, body: PutFanHeaderAvailability) =>
    request<FansStatus>("PUT", routes.fanHeaderAvailability(header), body),
  fanProfile: () => request<FanProfileName>("GET", routes.fanProfile),
  switchFanProfile: (name: string) =>
    request<FanProfileName>("PATCH", routes.fanProfile, { name }),
  putFanProfile: (name: string, profile: FanProfile) =>
    request<FanProfile>("PUT", routes.fanProfiles(name), profile),
  deleteFanProfile: (name: string) => request<unknown>("DELETE", routes.fanProfiles(name)),

  settings: () => request<Settings>("GET", routes.settings),
  putSettings: (body: Settings) => request<Settings>("PUT", routes.settings, body),

  /** The endpoints to pick from and what the current choice resolved to (#166). */
  audioSources: () => request<AudioSources>("GET", routes.audioSources),
  /** The only writer of the pick; answers what it resolved to. */
  putAudioSource: (body: AudioSource) => request<ResolvedAudioSource>("PUT", routes.audioSource, body),
  /** Takes a live-test lease; the readings poll renews it. */
  putAudioTest: (body: AudioTestRequest) => request<AudioTestLease>("PUT", routes.audioTest, body),
  /** Reads and renews a lease (410 when it is unknown or expired). */
  getAudioTest: (lease: string) =>
    request<AudioTestState>("GET", `${routes.audioTest}?lease=${encodeURIComponent(lease)}`),
  /** Ends a lease now, so turning Listen off closes the microphone at once. */
  releaseAudioTest: (lease: string) =>
    request<unknown>("DELETE", `${routes.audioTest}?lease=${encodeURIComponent(lease)}`),
  dismissZonePanel: () => request<unknown>("POST", routes.dismissZonePanel),
  /** The owner reached the end of the onboarding: the window stops opening by itself. */
  completeOnboarding: () => request<unknown>("POST", routes.onboardingComplete),

  /** Every numeric display with its config and the value it shows now. */
  displays: () => request<DisplayStatus[]>("GET", routes.displays),
  /** Saves a display's config. The daemon starts driving the display from it. */
  putDisplay: (id: string, body: DisplayConfig) => request<DisplayConfig>("PUT", routes.display(id), body),
  /** Removes the config: the daemon stops writing and closes the device. */
  deleteDisplay: (id: string) => request<unknown>("DELETE", routes.display(id)),
  /** The number a display with the `api` source shows. */
  putDisplayValue: (id: string, body: SetDisplayValue) => request<SetDisplayValue>("PUT", routes.displayValue(id), body),

  configExport: () => request<ExportBundle>("GET", routes.configExport),
  configImport: (bundle: ExportBundle) => request<unknown>("POST", routes.configImport, bundle),
};
