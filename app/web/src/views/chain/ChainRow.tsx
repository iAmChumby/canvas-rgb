"use client";

// One resizable header on the Devices page: what state it is in, the one key that starts or redoes
// its setup, and every part's own controls once it is set up.
//
// A header's state is the daemon's word (`ChainSetupState`): never set up, empty, or set up with
// its parts in chain order. The key opens the same flow the first run opens, inline under the row.
// After a set-up a header has several lights in one zone, so each part is drawn with its own row:
// its name, what it is (Fan, Strip or Other, sent as `kind`) and whether it runs the other way.
// Those three edits go straight to `PATCH /api/v1/lights/{id}` and read both lists back; the flow
// is not run for them.

import { useState, type CSSProperties } from "react";
import { PencilSimple } from "@phosphor-icons/react";

import { Button } from "@/components/Button";
import { KindIcon } from "@/components/KindIcon";
import { NameField } from "@/components/NameField";
import { StateDot } from "@/components/Panel";
import { Segmented, type SegOption } from "@/components/Segmented";
import { Toggle } from "@/components/Toggle";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshChains, refreshLights } from "@/lib/store";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainView } from "@/proto/ChainView";
import type { LightPatch } from "@/proto/LightPatch";
import type { LightView } from "@/proto/LightView";
import { ChainSetup } from "@/views/chain/ChainSetup";
import { PART_COLOURS } from "@/views/chain/partColours";
import s from "@/views/chain/wiring.module.css";

/** What a part is, in the words on the key and the mark beside them. */
const KINDS: SegOption<ChainPartKind>[] = [
  { value: "fan", label: "Fan", icon: <KindIcon kind={null} fan size={14} aria-hidden="true" /> },
  { value: "strip", label: "Strip", icon: <KindIcon kind="led_strip" size={14} aria-hidden="true" /> },
  { value: "other", label: "Other" },
];

/** A light from before 0.15.0 has no kind: its own shape is the closest true answer. */
const SHAPES: SegOption<"strip" | "ring">[] = [
  { value: "strip", label: "Strip" },
  { value: "ring", label: "Ring" },
];

/** The colour of a part's light, by its place in the chain (the list the flow lights it with). */
function colourAt(index: number): string {
  return PART_COLOURS[index % PART_COLOURS.length] ?? PART_COLOURS[0] ?? "#ffffff";
}

/** One part of a set-up header: its colour, its name, what it is, and which way it runs. */
function PartRow({ view, index }: { view: LightView; index: number }) {
  const light = view.light;
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(view.label);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  /** Every edit is one PATCH of this light; the two lists are read back so both windows agree. */
  const patch = async (body: LightPatch): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      await api.lights.patch(light.id, body);
      await Promise.all([refreshLights(), refreshChains()]);
      return true;
    } catch (e) {
      setError(messageOf(e));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const commitName = async () => {
    const next = name.trim();
    setEditing(false);
    if (next === view.label) return;
    if (!(await patch({ name: next || null }))) setName(view.label);
  };

  const kind = light.kind;

  return (
    <li className={s.part} data-testid="chain-part" data-busy={busy ? "true" : undefined}>
      <span className={s.chip} style={{ "--c": colourAt(index) } as CSSProperties} aria-hidden="true" />
      <span className={s.partWho}>
        {editing ? (
          <NameField
            value={name}
            label={`Name for ${view.label}`}
            maxLength={40}
            disabled={busy}
            onChange={setName}
            onEnter={() => void commitName()}
            onEscape={() => {
              setName(view.label);
              setEditing(false);
            }}
          />
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className={s.partName}
            aria-label={`Rename ${view.label}`}
            disabled={busy}
            onClick={() => {
              setName(view.label);
              setEditing(true);
            }}
          >
            <PencilSimple size={13} aria-hidden="true" />
            <span className="one-line">{view.label}</span>
          </Button>
        )}
        <span className={`mute ${s.partLeds}`}>
          {view.led_count} {view.led_count === 1 ? "LED" : "LEDs"}
        </span>
      </span>
      <span className={s.partKind}>
        {kind ? (
          <Segmented
            options={KINDS}
            value={kind}
            label={`What ${view.label} is`}
            disabled={busy}
            onChange={(next) => void patch({ kind: next })}
          />
        ) : (
          <Segmented
            options={SHAPES}
            value={light.arrangement === "ring" ? "ring" : "strip"}
            label={`What ${view.label} is`}
            disabled={busy}
            onChange={(next) => void patch({ arrangement: next })}
          />
        )}
      </span>
      <span className={s.partTurn}>
        <span className="mute">Runs the other way</span>
        <Toggle
          label={`Runs the other way, ${view.label}`}
          checked={light.reversed}
          disabled={busy}
          onChange={(next) => void patch({ reversed: next })}
        />
      </span>
      {error && <span className={`error-line ${s.partError}`}>{error}</span>}
    </li>
  );
}

/**
 * One resizable header's row. The state words carry the meaning (never set up, empty, or its
 * parts); the dot agrees. The key is the only thing that opens a setup session.
 */
export function ChainRow({ chain }: { chain: ChainView }) {
  const [open, setOpen] = useState(false);
  const state = chain.state;
  const count = chain.parts.length;
  const words =
    state === "set_up"
      ? `${count} ${count === 1 ? "part" : "parts"} · ${chain.leds} ${chain.leds === 1 ? "LED" : "LEDs"}`
      : state === "empty"
        ? "Empty"
        : "Not set up";
  const tone = state === "set_up" ? "ok" : state === "empty" ? "mute" : "warn";
  const key = state === "set_up" ? "Set up again" : "Set up";

  return (
    <div
      className={s.row}
      role="group"
      aria-label={`${chain.device_name} ${chain.zone_name}`}
      data-testid="chain-row"
      data-state={state}
    >
      <div className={s.head}>
        <span className={s.kicker}>Header</span>
        <span className={s.zone}>{chain.zone_name}</span>
        <StateDot tone={tone}>{words}</StateDot>
        <span className={s.key}>
          <Button
            size="sm"
            variant={state === "set_up" ? "default" : "primary"}
            aria-expanded={open}
            aria-label={`${key} ${chain.zone_name}`}
            onClick={() => setOpen((v) => !v)}
          >
            {key}
          </Button>
        </span>
      </div>

      {state === "empty" && <p className={s.empty}>Nothing is plugged into this header.</p>}

      {state === "set_up" && count > 0 && (
        <ul className={s.parts} aria-label={`Parts on ${chain.zone_name}`}>
          {chain.parts.map((view, i) => (
            <PartRow key={view.light.id} view={view} index={i} />
          ))}
        </ul>
      )}

      {open && (
        <div className={s.setup}>
          <ChainSetup chain={chain} onClose={() => setOpen(false)} />
        </div>
      )}
    </div>
  );
}
