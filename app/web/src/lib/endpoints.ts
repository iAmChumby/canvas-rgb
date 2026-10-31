// Typed calls for every route the editor uses. Paths come from the spine's routes.ts and the
// shapes from the generated proto types; nothing here re-declares an API type.

import { request, requestBinary } from "@/lib/api";
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
import type { SetFavorite } from "@/proto/SetFavorite";
import type { Props } from "@/proto/props";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { PeripheralAccess } from "@/proto/PeripheralAccess";
import type { PeripheralRow } from "@/lib/peripherals";
import type { LayoutsResponse } from "@/proto/LayoutsResponse";
import type { Layout } from "@/proto/Layout";
import type { SetActiveLayout } from "@/proto/SetActiveLayout";
import type { FansStatus } from "@/proto/FansStatus";
import type { FanMode } from "@/proto/FanMode";
import type { FanProfile } from "@/proto/FanProfile";
import type { FanProfileName } from "@/proto/FanProfileName";
import type { Settings } from "@/proto/Settings";
import type { DisplayConfig } from "@/proto/DisplayConfig";
import type { DisplayStatus } from "@/proto/DisplayStatus";
import type { SetDisplayValue } from "@/proto/SetDisplayValue";
import type { ExportBundle } from "@/proto/ExportBundle";

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

  effects: () => request<EffectsResponse>("GET", routes.effects),
  effect: (id: string) => request<EffectDetail>("GET", routes.effect(id)),
  /** Binary: u16 LE width, u16 LE height, u16 LE frame count, then frames x w x h x 3 RGB bytes. */
  effectThumbnail: (id: string) => requestBinary(routes.effectThumbnail(id)),
  activateEffect: (id: string, body: ActivateEffect) =>
    request<unknown>("POST", routes.effectActivate(id), body),
  patchActiveProps: (props: Props) => request<unknown>("PATCH", routes.effectActiveProps, props),
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
  deleteEffect: (id: string) => request<unknown>("DELETE", routes.effect(id)),

  /** Every peripheral the adapters see. The list opens nothing. */
  peripherals: () => request<{ peripherals: PeripheralRow[] }>("GET", routes.peripherals),
  /** What one peripheral is, what it has and what it holds now. */
  peripheral: (id: string) => request<PeripheralDetail>("GET", routes.peripheral(id)),
  /** Writes the changed keys to the device. Only while writes are on. */
  patchPeripheral: (id: string, body: Props) => request<Props>("PATCH", routes.peripheral(id), body),
  /** What the daemon may do with one peripheral. */
  putPeripheralAccess: (id: string, body: PeripheralAccess) =>
    request<PeripheralAccess>("PUT", routes.peripheralAccess(id), body),

  layouts: () => request<LayoutsResponse>("GET", routes.layouts),
  putLayout: (id: string, body: Layout) => request<Layout>("PUT", routes.layout(id), body),
  putActiveLayout: (body: SetActiveLayout) =>
    request<LayoutsResponse>("PUT", routes.layoutActive, body),

  fans: () => request<FansStatus>("GET", routes.fans),
  patchFanHeader: (header: string, mode: FanMode) =>
    request<unknown>("PATCH", routes.fanHeader(header), mode),
  fanProfile: () => request<FanProfileName>("GET", routes.fanProfile),
  switchFanProfile: (name: string) =>
    request<FanProfileName>("PATCH", routes.fanProfile, { name }),
  putFanProfile: (name: string, profile: FanProfile) =>
    request<FanProfile>("PUT", routes.fanProfiles(name), profile),
  deleteFanProfile: (name: string) => request<unknown>("DELETE", routes.fanProfiles(name)),

  settings: () => request<Settings>("GET", routes.settings),
  putSettings: (body: Settings) => request<Settings>("PUT", routes.settings, body),
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
