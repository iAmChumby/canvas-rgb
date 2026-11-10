// The demo backend's effect stacks (PR A's routes, PR C's builder): a seeded "Sunset glow" so the
// library tile, the builder and the preview show the real thing in a plain browser. The shapes
// are the spine's (`Stack`, `StackDetail`, `StackSummary`), built from the demo effects.

import type { DemoEffect, Props } from "@/lib/fixtures";
import type { BlendMode } from "@/proto/BlendMode";
import type { LayerArea } from "@/proto/LayerArea";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";
import type { StackDetail } from "@/proto/StackDetail";
import type { StackLayerState } from "@/proto/StackLayerState";
import type { StackSummary } from "@/proto/StackSummary";
import type { PropValue } from "@/proto/PropValue";

/** `?state=stack…` (or a `?stack=` deep link) turns the seeded stacks on. */
export function stackScenarios(scenario: Set<string>): boolean {
  for (const token of scenario) if (token.startsWith("stack")) return true;
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).has("stack");
}

function color(value: string): PropValue {
  return { type: "color", value };
}
function number(value: number): PropValue {
  return { type: "number", value };
}

function layer(
  id: string,
  name: string,
  effect: string,
  over: Props,
  extra: Partial<StackLayer> = {},
): StackLayer {
  return {
    id,
    name,
    effect,
    props: over,
    area: { kind: "canvas" },
    blend: "normal",
    opacity: 100,
    visible: true,
    ...extra,
  };
}

/** One layer of a starter stack: its effect, its name, and the values it overrides. */
export interface StarterLayer {
  name: string;
  effect: string;
  /** The property values this layer starts with, over the effect's own defaults. */
  props?: Props;
  blend?: BlendMode;
  opacity?: number;
  area?: LayerArea;
  /** A mask of another effect, its own values over that effect's defaults. */
  mask?: { effect: string; props?: Props };
}

/** A whole stack the "New effect" stage offers in one click (#257 phase 4, spec §5). */
export interface StarterStack {
  id: string;
  name: string;
  /** One line under the name: what the stack is made of. */
  blurb: string;
  layers: StarterLayer[];
}

/**
 * The three starters the empty builder offers (#257 phase 4): a whole stack in one click, added as
 * one undo step. They are ordinary effects with ordinary values; nothing here needs the demo
 * backend, and the daemon supplies every default they do not override.
 */
export const STARTER_STACKS: StarterStack[] = [
  {
    id: "starter:sunset-glow",
    name: "Sunset glow",
    blurb: "A gradient, fire on the keys, rain over both",
    layers: [
      {
        name: "Sunset",
        effect: "gradient",
        props: { color1: color("#ff2d55"), color2: color("#ffb703"), angle: { type: "angle", value: 112 } },
      },
      {
        name: "Heat",
        effect: "fire",
        props: { intensity: number(72), cooling: number(34) },
        area: { kind: "rect", x: 16, y: 150, width: 110, height: 46 },
        blend: "add",
        opacity: 70,
      },
      {
        name: "Sparkles",
        effect: "rain",
        props: { drop_color: color("#eaf2ff"), background: color("#050814") },
        blend: "screen",
        opacity: 80,
      },
    ],
  },
  {
    id: "starter:warm-flame",
    name: "Warm flame",
    blurb: "Warm white shaped by a fire mask",
    layers: [
      {
        name: "Warm white",
        effect: "white",
        props: { temperature: number(2700) },
        mask: {
          effect: "fire",
          props: { intensity: number(100), cooling: number(16), palette: { type: "choice", value: "Mono" } },
        },
      },
    ],
  },
  {
    id: "starter:sparkle-sky",
    name: "Sparkle sky",
    blurb: "Deep blue, aurora, and falling rain",
    layers: [
      {
        name: "Base",
        effect: "gradient",
        props: { color1: color("#0a0a4a"), color2: color("#1e6fd9") },
      },
      { name: "Waves", effect: "aurora", blend: "screen", opacity: 80 },
      { name: "Rain", effect: "rain", blend: "screen", opacity: 70 },
    ],
  },
];

/** The stack the mockups show: a Sunset gradient, Heat's fire on the keyboard, sparkles over both. */
export function seededStacks(scenario: Set<string>, effects: Record<string, DemoEffect>): Record<string, Stack> {
  if (!stackScenarios(scenario)) return {};
  const gradient = effects.gradient?.defaults ?? {};
  const fire = effects.fire?.defaults ?? {};
  const rain = effects.rain?.defaults ?? {};
  const sunset = layer("l1", "Sunset", "gradient", {
    ...gradient,
    color1: color("#ff2d55"),
    color2: color("#ffb703"),
    angle: { type: "angle", value: 112 },
  });
  const heat = layer(
    "l2",
    "Heat",
    "fire",
    { ...fire, intensity: number(72), cooling: number(34) },
    // The mockups' keyboard: 110 × 46 at x 16, y 150, against the keyboard's own 16–126.
    { area: { kind: "rect", x: 16, y: 150, width: 110, height: 46 }, blend: "add", opacity: 70 },
  );
  const sparkles = layer(
    "l3",
    "Sparkles",
    scenario.has("stack-missing") ? "html:gone" : "rain",
    { ...rain, drop_color: color("#eaf2ff"), background: color("#050814") },
    { blend: "screen", opacity: 80 },
  );
  const sunsetGlow: Stack = { id: "stack:Sunset glow", layers: [sunset, heat, sparkles] };
  // The mask's example (#256): a warm white shaped by a Fire luma mask, so the flame decides
  // where the white shows. It is in the demo so the builder's mask section can be seen.
  const warmWhite = layer("l1", "Warm white", "white", {
    ...(effects.white?.defaults ?? {}),
    temperature: number(2700),
  }, {
    mask: {
      effect: "fire",
      // Mono so the fire's luma reaches 1, intensity 100 with cooling 16 so the flame body fills
      // roughly the bottom half with tongues (measured on the real compositor, #256).
      props: { ...fire, intensity: number(100), cooling: number(16), palette: { type: "choice", value: "Mono" } },
      source: "luma",
      invert: false,
      amount: 100,
    },
  });
  const warmFlame: Stack = { id: "stack:Warm flame", layers: [warmWhite] };
  const eight: Stack = {
    id: "stack:Eight layers",
    layers: [
      layer("l1", "Base", "gradient", { ...gradient, color1: color("#0a0a4a"), color2: color("#1e6fd9") }),
      layer("l2", "Waves", "aurora", effects.aurora?.defaults ?? {}, { blend: "screen", opacity: 80 }),
      layer("l3", "Heat", "fire", fire, { blend: "add", opacity: 60, area: { kind: "rect", x: 16, y: 150, width: 110, height: 46 } }),
      layer("l4", "Rain", "rain", rain, { blend: "screen", opacity: 70 }),
      layer("l5", "Arms", "spiral", effects.spiral?.defaults ?? {}, { blend: "screen", opacity: 55 }),
      layer("l6", "Pulse", "breathing", effects.breathing?.defaults ?? {}, { blend: "add", opacity: 40 }),
      layer("l7", "Cycle", "color_cycle", effects.color_cycle?.defaults ?? {}, { blend: "screen", opacity: 35 }),
      layer("l8", "Flat", "solid", effects.solid?.defaults ?? {}, { opacity: 20 }),
    ],
  };
  const stacks: Record<string, Stack> = {
    [sunsetGlow.id]: sunsetGlow,
    [eight.id]: eight,
    [warmFlame.id]: warmFlame,
  };
  // `?state=stack-nested`: a hand-edited stack whose middle layer is another stack, so the row's
  // `Nested stack` and the picked layer's explanation can be seen (D95.4.8).
  if (scenario.has("stack-nested")) {
    const nested: Stack = {
      id: "stack:Nested demo",
      layers: [
        layer("l1", "Base", "gradient", gradient),
        layer("l2", "Twin", "stack:Sunset glow", {}),
        layer("l3", "Sparkles", "rain", rain),
      ],
    };
    stacks[nested.id] = nested;
  }
  // `?state=stack-gone-mask`: a layer whose mask names an effect the library does not have, so the
  // builder can show its refusal next to the mask Select (#256).
  if (scenario.has("stack-gone-mask")) {
    const gone: Stack = {
      id: "stack:Gone mask",
      layers: [
        layer("l1", "Warm white", "white", { ...(effects.white?.defaults ?? {}), temperature: number(2700) }, {
          mask: { effect: "html:gone", props: {}, source: "luma", invert: false, amount: 100 },
        }),
      ],
    };
    stacks[gone.id] = gone;
  }
  return stacks;
}

/** The library's row for a stack (D95.4.2). */
export function stackSummary(stack: Stack, effects: Record<string, DemoEffect>): StackSummary {
  return {
    id: stack.id,
    name: stack.id.slice("stack:".length),
    layers: stack.layers.length,
    html_layers: stack.layers.filter((layer) => layer.effect.startsWith("html:")).length,
    missing: stack.layers
      .filter(
        (layer) =>
          (!effects[layer.effect] && !layer.effect.startsWith("stack:")) ||
          (layer.mask !== undefined && !effects[layer.mask.effect]),
      )
      .map((layer) => layer.id),
  };
}

/** The library's `EffectSummary` for a stack: the generated description says its layers. */
export function stackEffectSummary(stack: Stack, effects: Record<string, DemoEffect>) {
  const names = stack.layers.map((layer) => effects[layer.effect]?.schema.name ?? layer.effect);
  const count = names.length;
  return {
    id: stack.id,
    name: stack.id.slice("stack:".length),
    description: count === 0 ? "No layers yet" : `${count} ${count === 1 ? "layer" : "layers"}: ${names.join(", ")}`,
    needs: { screen: false, audio: false },
    favorite: false,
  };
}

/** The stack's schema as the daemon derives it, from the demo effects' own schemas. */
export function stackSchema(stack: Stack, effects: Record<string, DemoEffect>): PropertySchema {
  const properties: PropertyDef[] = [];
  const needs = { screen: false, audio: false };
  const names: string[] = [];
  for (const layer of stack.layers) {
    const schema = effects[layer.effect]?.schema;
    // A mask's effect reads its own inputs too (#256).
    const maskSchema = layer.mask ? effects[layer.mask.effect]?.schema : undefined;
    if (maskSchema) {
      needs.screen ||= maskSchema.needs.screen;
      needs.audio ||= maskSchema.needs.audio;
    }
    if (schema) {
      needs.screen ||= schema.needs.screen;
      needs.audio ||= schema.needs.audio;
      names.push(schema.name);
      for (const def of schema.properties) {
        properties.push({
          key: `${layer.id}.${def.key}`,
          label: `${layer.name} · ${def.label}`,
          kind: def.kind,
        });
      }
    } else {
      names.push(layer.effect);
    }
  }
  const count = names.length;
  return {
    id: stack.id,
    name: stack.id.slice("stack:".length),
    description: count === 0 ? "No layers yet" : `${count} ${count === 1 ? "layer" : "layers"}: ${names.join(", ")}`,
    needs,
    properties,
    presets: [],
  };
}

/** Every layer's values over its defaults, namespaced `{layer.id}.{key}`. */
export function stackProps(stack: Stack, effects: Record<string, DemoEffect>): Props {
  const props: Props = {};
  for (const layer of stack.layers) {
    const schema = effects[layer.effect]?.schema;
    for (const def of schema?.properties ?? []) {
      const value = layer.props[def.key];
      props[`${layer.id}.${def.key}`] = value ?? defaultValue(def.kind);
    }
    for (const [key, value] of Object.entries(layer.props)) {
      if (schema && !schema.properties.some((def) => def.key === key)) continue;
      props[`${layer.id}.${key}`] = value;
    }
  }
  return props;
}

function defaultValue(kind: PropertyDef["kind"]): PropValue {
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
    case "angle":
      return { type: "angle", value: kind.default };
    default:
      return { type: "number", value: 0 };
  }
}

/** The stack as one `DemoEffect`, so the effect routes (schema, presets, thumbnail) serve it. */
export function stackAsEffect(stack: Stack, effects: Record<string, DemoEffect>): DemoEffect {
  const schema = stackSchema(stack, effects);
  return {
    schema,
    defaults: stackProps(stack, effects),
    summary: stackEffectSummary(stack, effects),
  };
}

/** `GET /stacks/{id}`: the stack and what each layer is doing now. */
export function stackDetail(stack: Stack, scenario: Set<string>, effects: Record<string, DemoEffect>): StackDetail {
  const layers = stack.layers.map((layer) => {
    let status: StackLayerState = "ok";
    let last_ms: number | undefined;
    if (layer.effect.startsWith("stack:")) status = "nested_stack";
    else if (!effects[layer.effect]) status = "missing_effect";
    else if (scenario.has("stack-held") && layer.id === "l2") {
      status = "held";
      last_ms = 12.4;
    }
    return { id: layer.id, status, ...(last_ms === undefined ? {} : { last_ms }) };
  });
  return { stack, layers };
}
