// When do live property values count as a saved preset, and how do they differ when they do not.
//
// The rule is shared with the daemon's test through docs/preset-equality.json: fill every
// missing key with the schema default, round a number to its step, round a hue to a whole
// degree modulo 360, compare colours without regard to case, ignore keys the schema does not
// name, then compare key by key.

import type { Preset } from "@/proto/Preset";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropKind } from "@/proto/PropKind";
import type { PropValue } from "@/proto/PropValue";
import type { Props } from "@/proto/props";

function defaultOf(kind: PropKind): PropValue {
  switch (kind.type) {
    case "color":
      return { type: "color", value: kind.default };
    case "number":
      return { type: "number", value: kind.default };
    case "bool":
      return { type: "bool", value: kind.default };
    case "choice":
      return { type: "choice", value: kind.default };
    case "hue":
      return { type: "hue", value: kind.default };
  }
}

/** Rounds to four places: enough for any step the effects use, and it drops float noise. */
function tidy(n: number): number {
  return Math.round(n * 1e4) / 1e4;
}

/** The value the rule compares: rounded to the control's step, colours in lower case. */
function normalValue(kind: PropKind, value: PropValue): PropValue {
  if (kind.type === "number" && value.type === "number") {
    const step = kind.step > 0 ? kind.step : 1;
    return { type: "number", value: tidy(Math.round(value.value / step) * step) };
  }
  if (kind.type === "hue" && value.type === "hue") {
    const whole = Math.round(value.value);
    return { type: "hue", value: ((whole % 360) + 360) % 360 };
  }
  if (value.type === "color") return { type: "color", value: value.value.toLowerCase() };
  return value;
}

/** Every schema key, in schema order, with the default for any the values leave out. */
export function fillDefaults(defs: PropertyDef[], props: Props): Props {
  const out: Props = {};
  for (const def of defs) out[def.key] = props[def.key] ?? defaultOf(def.kind);
  return out;
}

/** Every schema key, in schema order, filled with defaults and rounded. */
export function normalise(defs: PropertyDef[], props: Props): Props {
  const out: Props = {};
  for (const def of defs) {
    out[def.key] = normalValue(def.kind, props[def.key] ?? defaultOf(def.kind));
  }
  return out;
}

function sameValue(a: PropValue, b: PropValue): boolean {
  return a.type === b.type && a.value === b.value;
}

/** True when the two sets of values are the same preset. */
export function sameProps(defs: PropertyDef[], a: Props, b: Props): boolean {
  const na = normalise(defs, a);
  const nb = normalise(defs, b);
  return defs.every((d) => {
    const x = na[d.key];
    const y = nb[d.key];
    return x !== undefined && y !== undefined && sameValue(x, y);
  });
}

/** The first preset the values equal, or null. `presets` is searched in order. */
export function matchingPreset(defs: PropertyDef[], presets: Preset[], live: Props): Preset | null {
  return presets.find((p) => sameProps(defs, p.props, live)) ?? null;
}

/** A value in the words of the diff line: "40", "on", "#ff5e00", "120 deg". */
export function formatValue(v: PropValue): string {
  switch (v.type) {
    case "bool":
      return v.value ? "on" : "off";
    case "hue":
      return `${v.value}°`;
    case "number":
      return String(tidy(v.value));
    default:
      return String(v.value);
  }
}

export interface Change {
  key: string;
  label: string;
  from: string;
  to: string;
}

/** The properties whose live value differs from the preset's, as "Speed 20 to 35". */
export function diffProps(defs: PropertyDef[], preset: Props, live: Props): Change[] {
  const np = normalise(defs, preset);
  const nl = normalise(defs, live);
  const changes: Change[] = [];
  for (const def of defs) {
    const from = np[def.key];
    const to = nl[def.key];
    if (!from || !to || sameValue(from, to)) continue;
    changes.push({ key: def.key, label: def.label, from: formatValue(from), to: formatValue(to) });
  }
  return changes;
}

/** Folds the letters A to Z and nothing else: the daemon compares names with `eq_ignore_ascii_case`. */
export function asciiFold(name: string): string {
  return name.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** True when the name is taken by one of the lists, ignoring ASCII case. `except` may keep its own. */
export function nameTaken(name: string, lists: Preset[][], except?: string): boolean {
  const n = asciiFold(name.trim());
  const own = except === undefined ? null : asciiFold(except);
  return lists.some((list) => list.some((p) => asciiFold(p.name) === n && asciiFold(p.name) !== own));
}

/** The next free copy name: "Storm 2", then "Storm 3". */
export function nextFreeName(base: string, lists: Preset[][]): string {
  const stem = base.replace(/\s+\d+$/, "");
  for (let i = 2; i < 1000; i += 1) {
    const candidate = `${stem} ${i}`;
    if (!nameTaken(candidate, lists)) return candidate;
  }
  return `${stem} copy`;
}
