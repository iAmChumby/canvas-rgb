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
import type { EffectsResponse } from "@/proto/EffectsResponse";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { ActivateEffect } from "@/proto/ActivateEffect";
import type { SetFavorite } from "@/proto/SetFavorite";
import type { Props } from "@/proto/props";
import type { LayoutsResponse } from "@/proto/LayoutsResponse";
import type { Layout } from "@/proto/Layout";
import type { SetActiveLayout } from "@/proto/SetActiveLayout";
import type { FansStatus } from "@/proto/FansStatus";
import type { FanMode } from "@/proto/FanMode";
import type { FanProfile } from "@/proto/FanProfile";
import type { FanProfileName } from "@/proto/FanProfileName";
import type { Settings } from "@/proto/Settings";
import type { ExportBundle } from "@/proto/ExportBundle";

export const api = {
  status: () => request<Status>("GET", routes.status),

  output: () => request<OutputState>("GET", routes.output),
  patchOutput: (body: PatchOutput) => request<OutputState>("PATCH", routes.output, body),

  render: () => request<RenderState>("GET", routes.render),
  patchRender: (body: PatchRender) => request<RenderState>("PATCH", routes.render, body),

  devices: () => request<DevicesResponse>("GET", routes.devices),
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

  configExport: () => request<ExportBundle>("GET", routes.configExport),
  configImport: (bundle: ExportBundle) => request<unknown>("POST", routes.configImport, bundle),
};
