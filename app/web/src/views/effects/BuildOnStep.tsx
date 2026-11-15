"use client";

// The device step of a new pattern: which of the owner's own devices it is built on.
//
// A pattern is built on one kind of device, and the kind is the count and the shape the hardware
// reports (a ring of 12, a strip of 60, a grid). So the step lists the owner's own devices, one
// key per kind, never a list of abstract shapes: the three fan-hub lights that are all a ring of
// 40 are one key, and its dots show the shape before anything is painted.
//
// A light that is unplugged is still offered - a pattern can be built for a kind that is off the
// machine today - and says so. A device that reports no LEDs at all is not offered: there is
// nothing to paint.

import { Button } from "@/components/Button";
import { kindWords } from "@/lib/patternFit";
import { PatternTile, emptyPattern } from "@/views/effects/PatternTile";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LightView } from "@/proto/LightView";
import s from "@/views/effects/newEffect.module.css";

/** One kind of device: every light with the same LED count and the same shape. */
export interface DeviceKind {
  key: string;
  /** The first light's name: the owner's name for it, else the device and zone. */
  label: string;
  builtOn: BuiltOn;
  /** Every light of this kind. */
  count: number;
  /** The ones that are plugged in now. */
  connected: number;
}

/** The grid a light's zone reports, for a grid kind. `undefined` for every other shape. */
function gridOf(light: LightView, devices: DeviceInfo[] | undefined): BuiltOn["grid"] {
  if (light.light.arrangement !== "grid") return undefined;
  const device = devices?.find((d) => d.id === light.light.device_id);
  const zone = device?.zones.find((z) => z.index === light.light.zone_index);
  if (zone?.kind.type !== "matrix") return undefined;
  return { width: zone.kind.width, height: zone.kind.height };
}

/** How a light is told apart from another: the count, the shape, and a grid's cells. */
function kindKey(light: LightView, grid: BuiltOn["grid"]): string {
  return `${light.led_count}:${light.light.arrangement}${grid ? `:${grid.width}x${grid.height}` : ""}`;
}

/** The lights of one kind, in the order the daemon listed them. */
export function kindsOf(lights: LightView[], devices?: DeviceInfo[]): DeviceKind[] {
  const out: DeviceKind[] = [];
  const byKey = new Map<string, DeviceKind>();
  for (const light of lights) {
    const grid = gridOf(light, devices);
    const key = kindKey(light, grid);
    const known = byKey.get(key);
    if (known) {
      known.count += 1;
      if (light.connected) known.connected += 1;
      continue;
    }
    const kind: DeviceKind = {
      key,
      label: light.label,
      builtOn: {
        led_count: light.led_count,
        arrangement: light.light.arrangement,
        ...(grid ? { grid } : {}),
      },
      count: 1,
      connected: light.connected ? 1 : 0,
    };
    byKey.set(key, kind);
    out.push(kind);
  }
  return out;
}

/** What the line under a device's name says: how many there are, or why it is dark. */
function lineOf(kind: DeviceKind): string {
  if (kind.builtOn.led_count < 1) return "no LEDs to paint";
  if (kind.connected === 0) return "no device of this kind is connected";
  return kind.count === 1 ? "1 device" : `${kind.count} devices`;
}

export function BuildOnStep({
  lights,
  devices,
  onChoose,
  onCancel,
}: {
  /** Every light the daemon keeps, as `api.lights` answers. */
  lights: LightView[];
  /** The devices they belong to: a grid's cells are only in its zone. */
  devices?: DeviceInfo[];
  onChoose: (builtOn: BuiltOn) => void;
  onCancel: () => void;
}) {
  const kinds = kindsOf(lights, devices);
  return (
    <section className={s.choose} data-testid="build-on" aria-label="Build a pattern on">
      <header className={s.chooseHead}>
        <div className={s.chooseWho}>
          <h2 className={s.chooseTitle}>Build a pattern on</h2>
          <p className={s.chooseHint}>
            One of your devices. Every device of its kind plays the pattern whole.
          </p>
        </div>
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
      </header>

      {kinds.length === 0 ? (
        <p className={s.chooseHint}>
          No lights are known yet: a pattern is built on one, so set your devices up first.
        </p>
      ) : (
        <div className={s.chooseGrid}>
          {kinds.map((kind) => (
            <button
              key={kind.key}
              type="button"
              className={s.row}
              aria-label={`Build on ${kind.label}`}
              title={kind.builtOn.led_count < 1 ? "This device reports no LEDs" : undefined}
              disabled={kind.builtOn.led_count < 1}
              data-kind={kind.key}
              data-unplugged={kind.builtOn.led_count > 0 && kind.connected === 0 ? "true" : undefined}
              onClick={() => onChoose(kind.builtOn)}
            >
              <span className={s.rowWell} aria-hidden="true">
                <PatternTile pattern={emptyPattern(kind.builtOn)} size="sm" />
              </span>
              <span className={s.rowText}>
                <span className={s.rowName}>{kind.label}</span>
                <span className={s.rowLine}>
                  <span>{kindWords(kind.builtOn)}</span>
                  <span aria-hidden="true">·</span>
                  <span
                    className={
                      kind.builtOn.led_count < 1 || kind.connected === 0 ? s.rowProblem : s.rowCount
                    }
                  >
                    {lineOf(kind)}
                  </span>
                </span>
              </span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
