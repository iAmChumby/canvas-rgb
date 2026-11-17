"use client";

// One resizable ARGB header on the Devices page: what state it is in, the one key that starts or
// redoes its setup, and every device's own controls once it is set up.
//
// A header's state is the daemon's word (`ChainSetupState`): never set up, empty, or set up with
// its devices in cable order. The key opens the same flow the first run opens, in the same overlay
// the takeover uses, so the rows below stay where they are. It skips the introduction (the owner
// asked for this header) and writes no "Later": the header's own record is only the takeover's.
// After a set-up a header has several lights in one zone, so each device is drawn with its own
// row: its name, what it is (Fan, Strip or Other, sent as `kind`) and whether it runs the other
// way. A header whose devices mirror each other is one light shown several times; its row says so
// and has no second device to edit. Those edits go straight to `PATCH /api/v1/lights/{id}` and read
// both lists back; the flow is not run for them.

import { useCallback, useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
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
import { setTakeover } from "@/views/editor/takeover";

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
  const shown = light.copies ?? 1;

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
          {shown > 1 ? ` · shown ${shown} times` : ""}
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
 * One resizable ARGB header's row. The state words carry the meaning (never set up, empty, or its
 * devices); the dot agrees. The key is the only thing that opens a setup session. `named` is false
 * where the header's own strip sits right above the row and already carries its name.
 */
export function ChainRow({ chain, named = true, lead }: { chain: ChainView; named?: boolean; lead?: ReactNode }) {
  const [open, setOpen] = useState(false);
  /** One callback for the overlay's life: Escape's listener depends on it and must not re-subscribe. */
  const close = useCallback(() => setOpen(false), []);
  const state = chain.state;
  const count = chain.parts.length;
  /** How many times a header shows its one run: 8 for Luke's splitter, 1 for a series. */
  const repeats = chain.parts.length === 1 ? (chain.parts[0]?.light.copies ?? 1) : 1;
  const leds = `${chain.leds} ${chain.leds === 1 ? "LED" : "LEDs"}`;
  const words =
    state === "set_up"
      ? repeats > 1
        ? `${leds} · shown ${repeats} times`
        : `${count} ${count === 1 ? "device" : "devices"} · ${leds}`
      : state === "empty"
        ? "Empty"
        : "Not set up";
  const tone = state === "set_up" ? "ok" : state === "empty" ? "mute" : "warn";
  const key = state === "set_up" ? "Set up again" : "Set up";

  return (
    <div
      className={lead ? `${s.row} ${s.rowFlat}` : s.row}
      role="group"
      aria-label={`${chain.device_name} ${chain.zone_name}`}
      data-testid="chain-row"
      data-state={state}
    >
      <div className={lead ? `${s.head} ${s.gridHead}` : s.head}>
        {/* Devices passes the header's own identity, LED bar and count so this is one row, not a
            zone row followed by a second "ARGB header" row (#471). The state and the key stay here:
            the key is a row action, so it carries the secondary weight the other pages use. */}
        {lead}
        {!lead && <span className={s.kicker}>ARGB header</span>}
        {/* Under a zone's own strip the name is already on the page: it is said once. */}
        {!lead && named && <span className={s.zone}>{chain.zone_name}</span>}
        <StateDot tone={tone}>{words}</StateDot>
        <span className={s.key}>
          <Button
            size="sm"
            variant="default"
            aria-expanded={open}
            aria-label={`${key} ${chain.zone_name}`}
            onClick={() => setOpen((v) => !v)}
          >
            {key}
          </Button>
        </span>
      </div>

      {state === "empty" && <p className={s.empty}>Nothing is plugged into this ARGB header.</p>}

      {state === "set_up" && count > 0 && (
        <ul className={s.parts} aria-label={`Devices on ${chain.zone_name}`}>
          {chain.parts.map((view, i) => (
            <PartRow key={view.light.id} view={view} index={i} />
          ))}
        </ul>
      )}
      {state === "set_up" && repeats > 1 && <p className={s.empty}>You cannot set one on its own.</p>}

      {open && <Overlay chain={chain} onClose={close} />}
    </div>
  );
}

/**
 * The setup over the page, as the takeover has it: a plain dim with the flow standing in it, so the
 * rows of the Devices list do not move. The window is the flow's while it is open.
 *
 * Escape is Later here, as it is on the takeover: the flow closes and its session ends as it
 * unmounts, and the header's own record is not touched. A flow opened from a row writes no "Later"
 * flag, and the row's own key opens the header again whenever the owner asks.
 */
function Overlay({ chain, onClose }: { chain: ChainView; onClose: () => void }) {
  useEffect(() => {
    setTakeover(true);
    return () => setTakeover(false);
  }, []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  // Drawn on the body: a row inside a transformed or clipped ancestor would trap a fixed overlay.
  return createPortal(
    <div className={s.takeover}>
      <ChainSetup chain={chain} onClose={onClose} intro="none" />
    </div>,
    document.body,
  );
}
