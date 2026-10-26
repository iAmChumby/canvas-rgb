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
  effectActiveProps: "/api/v1/effects/active/props",
  effectFavorite: (id: string) => `/api/v1/effects/${id}/favorite`,
  layouts: "/api/v1/layouts",
  layout: (id: string) => `/api/v1/layouts/${encodeURIComponent(id)}`,
  layoutActive: "/api/v1/layouts/active",
  fans: "/api/v1/fans",
  fanHeader: (h: string) => `/api/v1/fans/headers/${h}`,
  fanProfile: "/api/v1/fans/profile",
  fanProfiles: (name: string) => `/api/v1/fans/profiles/${encodeURIComponent(name)}`,
  settings: "/api/v1/settings",
  dismissZonePanel: "/api/v1/first-run/dismiss-zone-panel",
  configExport: "/api/v1/config/export",
  configImport: "/api/v1/config/import",
  diagReadback: "/api/v1/diag/readback",
  // WebSocket; binary frames: u16 LE width, u16 LE height, width*height*3 RGB bytes.
  // Browsers cannot set headers on a WebSocket: pass the token as `?token=`.
  preview: "/api/v1/preview",
} as const;
