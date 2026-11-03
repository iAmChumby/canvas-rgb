// The demo backend's effect stacks (PR A's routes, PR C's builder): a seeded "Sunset glow" so the
// library tile, the builder and the preview show the real thing in a plain browser. The shapes
// are the spine's (`Stack`, `StackDetail`, `StackSummary`), built from the demo effects.

import type { DemoEffect, Props } from "@/lib/fixtures";
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
  const stacks: Record<string, Stack> = { [sunsetGlow.id]: sunsetGlow, [eight.id]: eight };
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
      .filter((layer) => !effects[layer.effect] && !layer.effect.startsWith("stack:"))
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
