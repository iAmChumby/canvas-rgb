"use client";

import type { CSSProperties } from "react";
import { ColorField } from "@/components/ColorField";
import type { ColorSwatch } from "@/components/ColorPicker";
import { HueRing } from "@/components/HueRing";
import { NumberField } from "@/components/NumberField";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { Slider } from "@/components/Slider";
import { Toggle } from "@/components/Toggle";
import { parseHex } from "@/lib/color";
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
 *
 * `readOnly` names the keys the device cannot be asked to change: they are shown as the values
 * they hold, not as controls. `disabled` holds every control back (the daemon may not write).
 */
export function PropertyControls({
  schema,
  values,
  onChange,
  readOnly,
  disabled,
  optionLabel,
}: {
  schema: PropertySchema;
  values: Values;
  onChange: (key: string, value: PropValue) => void;
  readOnly?: string[];
  disabled?: boolean;
  /** The words for a Choice option's id, when the ids are not words (peripherals). */
  optionLabel?: (id: string) => string;
}) {
  return (
    <div className="prop-list">
      {schema.properties.map((def) => {
        const value = values[def.key] ?? defaultFor(def.kind);
        const fixed = readOnly?.includes(def.key) ?? false;
        return (
          <div className="prop-item" data-kind={def.kind.type} key={def.key}>
            <span className="field-label">
              <span>{def.label}</span>
            </span>
            <div className="prop-control">
              {fixed ? (
                <ReadOnlyValue value={value} optionLabel={optionLabel} />
              ) : (
                control(def.key, def.label, def.kind, value, schema, values, onChange, disabled, optionLabel)
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** A setting the adapter cannot write: the value it holds, as a value. */
function ReadOnlyValue({ value, optionLabel }: { value: PropValue; optionLabel?: (id: string) => string }) {
  if (value.type === "color") {
    return (
      <span className="read-only">
        <span className="read-only-dot" style={{ "--c": value.value } as CSSProperties} aria-hidden="true" />
        <span className="fig">{value.value}</span>
      </span>
    );
  }
  if (value.type === "bool") {
    return <span className="read-only fig">{value.value ? "on" : "off"}</span>;
  }
  if (value.type === "number") {
    return <span className="read-only fig">{value.value}</span>;
  }
  if (value.type === "choice" && optionLabel) {
    return <span className="read-only">{optionLabel(value.value)}</span>;
  }
  return <span className="read-only fig">{value.value}</span>;
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
  schema: PropertySchema,
  values: Values,
  onChange: (key: string, value: PropValue) => void,
  disabled?: boolean,
  optionLabel?: (id: string) => string,
) {
  const word = (o: string) => (optionLabel ? optionLabel(o) : o);
  switch (kind.type) {
    case "color": {
      const hex = value.type === "color" ? value.value : kind.default;
      // The other colours of the same effect are one click away inside the picker.
      const quick: ColorSwatch[] = [];
      for (const other of schema.properties) {
        if (other.key === key || other.kind.type !== "color") continue;
        const held = values[other.key];
        const otherHex = held && held.type === "color" ? held.value : other.kind.default;
        const parsed = parseHex(otherHex);
        if (!parsed || parsed === hex.toLowerCase()) continue;
        quick.push({ hex: parsed, label: other.label });
      }
      return (
        <ColorField
          label={label}
          value={hex}
          swatches={quick}
          disabled={disabled}
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
            disabled={disabled}
            onChange={(v) => onChange(key, { type: "number", value: v })}
          />
          <NumberField
            label={`${label} value`}
            min={kind.min}
            max={kind.max}
            step={kind.step}
            decimals={decimals}
            value={n}
            disabled={disabled}
            onCommit={(v) => onChange(key, { type: "number", value: v })}
          />
        </>
      );
    }
    case "bool": {
      const on = value.type === "bool" ? value.value : kind.default;
      return (
        <Toggle
          label={label}
          checked={on}
          disabled={disabled}
          onChange={(v) => onChange(key, { type: "bool", value: v })}
        />
      );
    }
    case "choice": {
      const current = value.type === "choice" ? value.value : kind.default;
      if (kind.options.length <= 4) {
        return (
          <Segmented<string>
            label={label}
            value={current}
            options={kind.options.map((o) => ({ value: o, label: word(o) }))}
            disabled={disabled}
            onChange={(v) => onChange(key, { type: "choice", value: v })}
          />
        );
      }
      return (
        <Select
          label={label}
          value={current}
          options={kind.options.map((o) => ({ value: o, label: word(o) }))}
          disabled={disabled}
          onChange={(v) => onChange(key, { type: "choice", value: v })}
        />
      );
    }
    case "hue": {
      const deg = value.type === "hue" ? value.value : kind.default;
      return (
        <div className="hue">
          <HueRing label={label} value={deg} disabled={disabled} onChange={(v) => onChange(key, { type: "hue", value: v })} />
          <NumberField
            label={`${label} degrees`}
            min={0}
            max={360}
            value={Math.round(deg)}
            unit="°"
            disabled={disabled}
            onCommit={(v) => onChange(key, { type: "hue", value: v })}
          />
        </div>
      );
    }
  }
}
