// Typed calls for every route the editor uses. Paths come from the spine's routes.ts and the
// shapes from the generated proto types; nothing here re-declares an API type.

import { request, requestBinary, type RequestOptions } from "@/lib/api";
import { routes } from "@/proto/routes";
import type { Status } from "@/proto/Status";
import type { RenderState } from "@/proto/RenderState";
import type { PatchRender } from "@/proto/PatchRender";
import type { OutputState } from "@/proto/OutputState";
import type { PatchOutput } from "@/proto/PatchOutput";
import type { DevicesResponse } from "@/proto/DevicesResponse";
import type { PatchZone } from "@/proto/PatchZone";
import type { IdentifyRequest } from "@/proto/IdentifyRequest";
import type { OnboardingSession } from "@/proto/OnboardingSession";
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
import type { LayoutsResponse } from "@/proto/LayoutsResponse";
import type { Layout } from "@/proto/Layout";
import type { SetActiveLayout } from "@/proto/SetActiveLayout";
import type { ScenesResponse } from "@/proto/ScenesResponse";
import type { Scene } from "@/proto/Scene";
import type { SceneCreate } from "@/proto/SceneCreate";
import type { ScenePut } from "@/proto/ScenePut";
import type { SceneActivePut } from "@/proto/SceneActivePut";
import type { GroupBrightnessPut } from "@/proto/GroupBrightnessPut";
import type { LightsResponse } from "@/proto/LightsResponse";
import type { LightView } from "@/proto/LightView";
import type { LightPatch } from "@/proto/LightPatch";
import type { FlashRequest } from "@/proto/FlashRequest";
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
import type { AudioSource } from "@/proto/AudioSource";
import type { AudioSources } from "@/proto/AudioSources";
import type { ResolvedAudioSource } from "@/proto/ResolvedAudioSource";
import type { AudioTestLease } from "@/proto/AudioTestLease";
import type { AudioTestRequest } from "@/proto/AudioTestRequest";
import type { AudioTestState } from "@/proto/AudioTestState";

/** POST /api/v1/onboarding/session (canvas-proto `ONBOARDING_SESSION`). */
const ONBOARDING_SESSION = "/api/v1/onboarding/session";

export const api = {
  status: () => request<Status>("GET", routes.status),

  output: () => request<OutputState>("GET", routes.output),
  patchOutput: (body: PatchOutput) => request<OutputState>("PATCH", routes.output, body),

  render: () => request<RenderState>("GET", routes.render),
  patchRender: (body: PatchRender) => request<RenderState>("PATCH", routes.render, body),

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
  patchActiveProps: (props: Props) => request<unknown>("PATCH", routes.effectActiveProps, props),
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
   * values it kept; `fresh` makes it read the device (a read stalls the pointer for seconds).
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

  layouts: () => request<LayoutsResponse>("GET", routes.layouts),
  putLayout: (id: string, body: Layout) => request<Layout>("PUT", routes.layout(id), body),
  putActiveLayout: (body: SetActiveLayout) =>
    request<LayoutsResponse>("PUT", routes.layoutActive, body),

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
  },
  lights: {
    list: () => request<LightsResponse>("GET", routes.lights),
    patch: (id: string, body: LightPatch) => request<LightView>("PATCH", routes.light(id), body),
    /** The named lights white and every other LED black; `stop` ends it. */
    flash: (body: FlashRequest) => request<unknown>("POST", routes.lightsFlash, body),
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
  /** While active, every light is black except what identify lights; resend every 2 s (the daemon ends it after 5 s). */
  onboardingSession: (body: OnboardingSession) =>
    request<unknown>("POST", ONBOARDING_SESSION, body),

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
