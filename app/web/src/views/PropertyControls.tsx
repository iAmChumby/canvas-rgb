"use client";

import { ColorField } from "@/components/ColorField";
import { HueRing } from "@/components/HueRing";
import { NumberField } from "@/components/NumberField";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { Slider } from "@/components/Slider";
import { Toggle } from "@/components/Toggle";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { PropKind } from "@/proto/PropKind";

export type Values = { [key: string]: PropValue };

/** Decimal places a step needs: 1 -> 0, 0.5 -> 1, 0.1 -> 1. */
function decimalsOf(step: number): number {
  const s = String(step);
  const dot = s.indexOf(".");
  return dot < 0 ? 0 : Math.min(2, s.length - dot - 1);
}

/**
 * Renders an effect's controls from its PropertySchema alone. There is no per-effect component
 * anywhere in the editor: a colour is a ColorField, a number a Slider plus NumberField, a bool a
 * Toggle, a choice a Segmented (up to four options) or Select, a hue a HueRing plus degrees.
 */
export function PropertyControls({
  schema,
  values,
  onChange,
}: {
  schema: PropertySchema;
  values: Values;
  onChange: (key: string, value: PropValue) => void;
}) {
  return (
    <div className="prop-list">
      {schema.properties.map((def) => {
        const value = values[def.key] ?? defaultFor(def.kind);
        return (
          <div className="prop-item" data-kind={def.kind.type} key={def.key}>
            <span className="field-label">
              <span>{def.label}</span>
            </span>
            <div className="prop-control">{control(def.key, def.label, def.kind, value, onChange)}</div>
          </div>
        );
      })}
    </div>
  );
}

function defaultFor(kind: PropKind): PropValue {
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

function control(
  key: string,
  label: string,
  kind: PropKind,
  value: PropValue,
  onChange: (key: string, value: PropValue) => void,
) {
  switch (kind.type) {
    case "color": {
      const hex = value.type === "color" ? value.value : kind.default;
      return (
        <ColorField
          label={label}
          value={hex}
          onChange={(v) => onChange(key, { type: "color", value: v })}
        />
      );
    }
    case "number": {
      const n = value.type === "number" ? value.value : kind.default;
      const decimals = decimalsOf(kind.step);
      return (
        <>
          <Slider
            label={label}
            min={kind.min}
            max={kind.max}
            step={kind.step}
            value={n}
            onChange={(v) => onChange(key, { type: "number", value: v })}
          />
          <NumberField
            label={`${label} value`}
            min={kind.min}
            max={kind.max}
            step={kind.step}
            decimals={decimals}
            value={n}
            onCommit={(v) => onChange(key, { type: "number", value: v })}
          />
        </>
      );
    }
    case "bool": {
      const on = value.type === "bool" ? value.value : kind.default;
      return <Toggle label={label} checked={on} onChange={(v) => onChange(key, { type: "bool", value: v })} />;
    }
    case "choice": {
      const current = value.type === "choice" ? value.value : kind.default;
      if (kind.options.length <= 4) {
        return (
          <Segmented<string>
            label={label}
            value={current}
            options={kind.options.map((o) => ({ value: o, label: o }))}
            onChange={(v) => onChange(key, { type: "choice", value: v })}
          />
        );
      }
      return (
        <Select
          label={label}
          value={current}
          options={kind.options.map((o) => ({ value: o, label: o }))}
          onChange={(v) => onChange(key, { type: "choice", value: v })}
        />
      );
    }
    case "hue": {
      const deg = value.type === "hue" ? value.value : kind.default;
      return (
        <div className="hue">
          <HueRing label={label} value={deg} onChange={(v) => onChange(key, { type: "hue", value: v })} />
          <NumberField
            label={`${label} degrees`}
            min={0}
            max={360}
            value={Math.round(deg)}
            unit="°"
            onCommit={(v) => onChange(key, { type: "hue", value: v })}
          />
        </div>
      );
    }
  }
}
