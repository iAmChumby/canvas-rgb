// Zone geometry on the 320 x 200 logical canvas: where each LED of a placed zone sits.
// Linear zones run along the rect's long axis, left to right (or top to bottom) before rotation,
// and `reversed` flips them. Matrix zones fill the rect as their grid. Rotation is clockwise
// about the rect's centre, as CSS rotate() draws it.

import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { DeviceInfo } from "@/proto/DeviceInfo";

export interface Point {
  x: number;
  y: number;
}

export function findZoneInfo(
  devices: DeviceInfo[] | null,
  deviceId: string,
  zoneIndex: number,
): { device: DeviceInfo; zone: ZoneInfo } | null {
  const device = devices?.find((d) => d.id === deviceId);
  const zone = device?.zones.find((z) => z.index === zoneIndex);
  if (!device || !zone) return null;
  return { device, zone };
}

export function ledPositions(z: PlacedZone, info: ZoneInfo): Point[] {
  const w = z.width;
  const h = z.height;
  const local: Point[] = [];
  const kind = info.kind;
  if (kind.type === "single") {
    local.push({ x: 0, y: 0 });
  } else if (kind.type === "linear") {
    const n = Math.max(0, info.led_count);
    const horizontal = w >= h;
    for (let i = 0; i < n; i++) {
      let t = (i + 0.5) / n;
      if (z.reversed) t = 1 - t;
      local.push(horizontal ? { x: -w / 2 + t * w, y: 0 } : { x: 0, y: -h / 2 + t * h });
    }
  } else {
    const cols = kind.width;
    const rows = kind.height;
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const idx = kind.map[r * cols + c];
        if (idx === null || idx === undefined) continue;
        let cx = (c + 0.5) / cols;
        if (z.reversed) cx = 1 - cx;
        local.push({ x: -w / 2 + cx * w, y: -h / 2 + ((r + 0.5) / rows) * h });
      }
    }
  }
  const cx = z.x + w / 2;
  const cy = z.y + h / 2;
  const rad = (z.rotation_deg * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  return local.map((p) => ({ x: cx + p.x * cos - p.y * sin, y: cy + p.x * sin + p.y * cos }));
}

/** Logical size of a zone when it is first placed. */
export function defaultRect(info: ZoneInfo): { width: number; height: number } {
  const kind = info.kind;
  if (kind.type === "matrix") {
    const aspect = kind.width / Math.max(1, kind.height);
    const width = 80 * Math.min(1.6, Math.max(0.6, aspect));
    return { width, height: width / Math.max(0.1, aspect) };
  }
  if (kind.type === "linear") {
    const width = Math.min(160, Math.max(24, info.led_count * 0.6));
    return { width, height: 8 };
  }
  return { width: 14, height: 14 };
}
