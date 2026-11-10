"use client";

// The stack's presets (#257 phase 4): the chips under the picked layer, moved out of
// StackBuilder.tsx unchanged. A preset writes the merged values into the draft in one step;
// saving the current values is a write to the stack's own preset list.

import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { InspectorSection } from "@/components/InspectorSection";
import { NameField } from "@/components/NameField";
import { Panel } from "@/components/Panel";
import type { Preset } from "@/proto/Preset";

export function PresetStrip({
  builtIn,
  user,
  appliedPreset,
  presetProblem,
  presetName,
  onApply,
  onStartSave,
  onName,
  onSave,
  onEscape,
}: {
  builtIn: Preset[];
  user: Preset[];
  appliedPreset: string | null;
  presetProblem: string | null;
  presetName: string | null;
  onApply: (preset: Preset) => void;
  onStartSave: () => void;
  onName: (value: string) => void;
  onSave: () => void;
  onEscape: () => void;
}) {
  return (
    <Panel className="rd-presets">
      <InspectorSection title="Presets">
        <div className="preset-chips" role="group" aria-label="Presets">
        {builtIn.map((preset) => (
          <Chip
            key={preset.name}
            pressed={appliedPreset === preset.name}
            label={`Apply preset ${preset.name}`}
            onClick={() => onApply(preset)}
          >
            {preset.name}
          </Chip>
        ))}
        {user.map((preset) => (
          <Chip
            key={preset.name}
            pressed={appliedPreset === preset.name}
            label={`Apply preset ${preset.name}`}
            onClick={() => onApply(preset)}
          >
            {preset.name}
          </Chip>
        ))}
        <Chip
          pressed={false}
          label="Save the current values as a preset"
          onClick={onStartSave}
        >
          + Save current
        </Chip>
      </div>
      {presetProblem && (
        <p className="error-line" role="alert">
          {presetProblem}
        </p>
      )}
      {presetName !== null && (
        <div className="preset-save">
          <NameField
            value={presetName}
            label="Preset name"
            placeholder="Preset name"
            maxLength={40}
            onChange={onName}
            onEnter={onSave}
            onEscape={onEscape}
          />
          <Button size="sm" variant="primary" onClick={onSave}>
            Save
          </Button>
        </div>
      )}
      <div className="preset-line" data-mode="idle">
        <span className="preset-status">{appliedPreset ?? "Custom values"}</span>
      </div>
      </InspectorSection>
    </Panel>
  );
}
