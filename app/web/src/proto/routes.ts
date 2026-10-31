// Mirrors crates/canvas-proto/src/api.rs `routes`. Spine file: edit only through the spine owner.
// Every request needs `Authorization: Bearer <token>` except GET STATUS (G9).
export const routes = {
  status: "/api/v1/status",
  output: "/api/v1/output",
  render: "/api/v1/render",
  devices: "/api/v1/devices",
  deviceZone: (id: string, zone: number) => `/api/v1/devices/${encodeURIComponent(id)}/zones/${zone}`,
  deviceIdentify: (id: string) => `/api/v1/devices/${encodeURIComponent(id)}/identify`,
  effects: "/api/v1/effects",
  effect: (id: string) => `/api/v1/effects/${id}`,
  effectActivate: (id: string) => `/api/v1/effects/${id}/activate`,
  // POST a PresetCreate: saves a user preset (409 on a name taken by a built-in or user preset).
  effectPresets: (id: string) => `/api/v1/effects/${id}/presets`,
  // PUT a PresetReplace (overwrite and/or rename; 409 when `expected` is stale); DELETE removes it.
  effectPreset: (id: string, name: string) => `/api/v1/effects/${id}/presets/${encodeURIComponent(name)}`,
  effectActiveProps: "/api/v1/effects/active/props",
  // POST an ImportEffect: a SignalRGB-format HTML effect to add to the effects folder.
  // 409 when the name is taken and no replace is asked for; 422 when the file does not parse.
  effectsImport: "/api/v1/effects/import",
  // DELETE removes a user effect (an `html:` id).
  // GET every peripheral the adapters see; GET one peripheral's detail; PATCH writes the
  // changed keys to the device; PUT the access says what the daemon may do with it.
  peripherals: "/api/v1/peripherals",
  peripheral: (id: string) => `/api/v1/peripherals/${encodeURIComponent(id)}`,
  peripheralAccess: (id: string) => `/api/v1/peripherals/${encodeURIComponent(id)}/access`,
  // GET binary: u16 LE w, u16 LE h, u16 LE frames, then frames*w*h*3 RGB (64x40, 24 frames, 12 fps).
  effectThumbnail: (id: string) => `/api/v1/effects/${encodeURIComponent(id)}/thumbnail`,
  effectFavorite: (id: string) => `/api/v1/effects/${id}/favorite`,
  layouts: "/api/v1/layouts",
  layout: (id: string) => `/api/v1/layouts/${encodeURIComponent(id)}`,
  layoutActive: "/api/v1/layouts/active",
  fans: "/api/v1/fans",
  fanHeader: (h: string) => `/api/v1/fans/headers/${h}`,
  fanProfile: "/api/v1/fans/profile",
  // PUT a FanProfile; DELETE removes it (409 for the active or last profile).
  fanProfiles: (name: string) => `/api/v1/fans/profiles/${encodeURIComponent(name)}`,
  settings: "/api/v1/settings",
  dismissZonePanel: "/api/v1/first-run/dismiss-zone-panel",
  // POST, no body: the owner reached the end of the onboarding (Status.onboarding_complete).
  onboardingComplete: "/api/v1/onboarding/complete",
  // GET every numeric display with its config and the value it shows now; PUT a DisplayConfig
  // for one display; DELETE removes the config, stops writing and closes the device.
  displays: "/api/v1/displays",
  display: (id: string) => `/api/v1/displays/${encodeURIComponent(id)}`,
  displayValue: (id: string) => `/api/v1/displays/${encodeURIComponent(id)}/value`,
  configExport: "/api/v1/config/export",
  configImport: "/api/v1/config/import",
  quit: "/api/v1/quit",
  diagReadback: "/api/v1/diag/readback",
  // WebSocket; binary frames: u16 LE width, u16 LE height, width*height*3 RGB bytes.
  // Browsers cannot set headers on a WebSocket: pass the token as `?token=`.
  preview: "/api/v1/preview",
} as const;
