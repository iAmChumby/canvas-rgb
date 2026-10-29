"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight, Check, Minus, Plus } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { StepFrame } from "@/views/editor/OnboardStep";
import type { Sel } from "@/views/editor/parts";
import { allRuns, locateRun, placementsOfRun, runKey, type Run, type SetupZone } from "@/views/editor/setup";
import { api } from "@/lib/endpoints";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";

/** The daemon ends an identify a few seconds after the last request: ask again inside that. */
const RESEND_MS = 2000;

/**
 * Match the lights: for each run of lights in turn the PC shows only that run (every other light
 * is black for the whole onboarding), and the owner taps the part on the drawing that is lit.
 * "Nothing's lit" skips the run. The tap gives the run to the part: its lights, its part and its
 * "mapped" all live in the layout, so the progress survives a restart.
 *
 * The owner never sees a header, a zone or a LED range here: only parts and lights.
 */
export function MatchStep({
  layout,
  zones,
  onChange,
  setPick,
  onFocusPart,
  onBack,
  onNext,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  /** The zones to walk (one device only, when it is redone). */
  zones: SetupZone[];
  onChange: (fn: (l: Layout) => Layout) => void;
  setPick: (fn: ((sel: Sel[]) => void) | null) => void;
  onFocusPart: (index: number | null) => void;
  onBack: (() => void) | null;
  onNext: () => void;
}) {
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(() => new Set());
  const [run, setRun] = useState<Run | null>(() => locateRun(layout, zones, new Set(), 0, 0));
  /** After a tap: the part that took the run, and whether the owner is resizing it. */
  const [tapped, setTapped] = useState<{ index: number } | null>(null);
  const [resizing, setResizing] = useState(false);
  const [count, setCount] = useState(run?.count ?? 0);
  const all = useMemo(() => allRuns(zones), [zones]);
  const total = all.length;
  const doneCount = run ? all.findIndex((r) => r.zi === run.zi && r.start >= run.start) : total;

  const layoutRef = useRef(layout);
  useEffect(() => {
    layoutRef.current = layout;
  });

  // Light the run, and keep it lit. Only this run shows; the session keeps the rest black.
  const litId = run?.zone.deviceId;
  const litZone = run?.zone.zoneIndex;
  const litLed = run?.start;
  useEffect(() => {
    if (litId === undefined || litZone === undefined || litLed === undefined) return;
    const send = () =>
      void api
        .identify(litId, { zone: litZone, led: litLed, ...(count > 1 ? { count } : {}), stop: false })
        .catch(() => undefined);
    send();
    const t = setInterval(send, RESEND_MS);
    return () => clearInterval(t);
  }, [litId, litZone, litLed, count]);
  useEffect(() => {
    if (litId === undefined || litZone === undefined) return;
    return () => {
      void api.identify(litId, { zone: litZone, led: 0, stop: true }).catch(() => undefined);
    };
  }, [litId, litZone]);

  const advance = useCallback(
    (skip: ReadonlySet<string>, from: Run, used: number) => {
      setTapped(null);
      setResizing(false);
      onFocusPart(null);
      const nxt = locateRun(layoutRef.current, zones, skip, from.zi, from.start + used);
      setRun(nxt);
      setCount(nxt?.count ?? 0);
    },
    [zones, onFocusPart],
  );

  /** The tap: the part takes the run. Whoever held the run takes the part's old lights, so none is lost. */
  const assign = useCallback(
    (index: number) => {
      if (!run) return;
      const slice = run.whole ? undefined : { start: run.start, count };
      const target = layoutRef.current.zones[index];
      if (!target || target.part?.mapped === true) return;
      onChange((l) => {
        const p = l.zones[index];
        if (!p || p.part?.mapped === true) return l;
        const other = placementsOfRun(l, run.zone, run.start, count).find(
          (i) => i !== index && l.zones[i]?.part?.mapped !== true,
        );
        const old = { device_id: p.device_id, zone_index: p.zone_index, leds: p.leds };
        return {
          ...l,
          zones: l.zones.map((z, i) => {
            if (i === index) {
              const { leds: _drop, ...rest } = z;
              void _drop;
              return {
                ...rest,
                device_id: run.zone.deviceId,
                zone_index: run.zone.zoneIndex,
                ...(slice ? { leds: slice } : {}),
                part: { ...(z.part ?? { kind: "generic" as const, facing: "viewer" as const }), mapped: true },
              };
            }
            if (i === other) {
              const { leds: _d, ...rest } = z;
              void _d;
              return { ...rest, device_id: old.device_id, zone_index: old.zone_index, ...(old.leds ? { leds: old.leds } : {}) };
            }
            return z;
          }),
        };
      });
      setTapped({ index });
      onFocusPart(index);
    },
    [run, count, onChange, onFocusPart],
  );

  // The drawing's taps go here while a run is being asked about.
  const assignRef = useRef(assign);
  useEffect(() => {
    assignRef.current = assign;
  });
  const asking = run !== null && tapped === null;
  useEffect(() => {
    if (!asking) {
      setPick(() => undefined);
      return () => setPick(null);
    }
    setPick((sel) => {
      const z = sel.find((x) => x.kind === "zone");
      if (z) assignRef.current(z.index);
    });
    return () => setPick(null);
  }, [asking, setPick]);

  const undo = () => {
    if (!tapped) return;
    const i = tapped.index;
    onChange((l) => ({
      ...l,
      zones: l.zones.map((z, k) => (k === i && z.part ? { ...z, part: { ...z.part, mapped: false } } : z)),
    }));
    setTapped(null);
    setResizing(false);
    onFocusPart(null);
  };

  const resize = (n: number) => {
    if (!run || !tapped) return;
    const c = Math.max(1, Math.min(run.zone.maxLeds, run.zone.ledCount - run.start, n));
    setCount(c);
    const i = tapped.index;
    onChange((l) => ({
      ...l,
      zones: l.zones.map((z, k) => (k === i ? { ...z, leds: { start: run.start, count: c } } : z)),
    }));
  };

  const kicker = run ? (
    <>
      Match the lights · <AnimatedNumber value={Math.max(0, doneCount) + 1} /> of {total}
    </>
  ) : (
    "Match the lights"
  );

  if (!run) {
    return (
      <StepFrame
        kicker={kicker}
        title="Every light is matched."
        lead="Each part now knows which lights are its own."
        onBack={onBack}
        primary={{ label: "Continue", icon: <ArrowRight size={16} weight="bold" />, onClick: onNext }}
      />
    );
  }

  if (tapped) {
    const can = run.zone.resizable;
    return (
      <StepFrame
        kicker={kicker}
        title="Is it lit all the way around?"
        lead={resizing ? "Change how many lights it has until the whole part is lit." : "Look at the whole part, end to end."}
        onBack={onBack}
        primary={{
          label: resizing ? "That's all of it" : "Yes",
          icon: <Check size={16} weight="bold" />,
          onClick: () => advance(skipped, run, count),
        }}
        secondary={
          resizing
            ? null
            : {
                label: "No",
                onClick: () => (can ? setResizing(true) : undo()),
              }
        }
      >
        {resizing && (
          <div className={s.size} aria-live="polite">
            <IconButton label="Fewer lights" icon={<Minus size={16} />} disabled={count <= 1} onClick={() => resize(count - 1)} />
            <span className={s.sizeValue}>
              <AnimatedNumber value={count} />
            </span>
            <IconButton label="More lights" icon={<Plus size={16} />} disabled={count >= run.zone.maxLeds} onClick={() => resize(count + 1)} />
          </div>
        )}
      </StepFrame>
    );
  }

  return (
    <StepFrame
      kicker={kicker}
      title="Which part is lit?"
      lead="Everything else in your PC is off. Tap the lit part on the drawing."
      onBack={onBack}
      primary={{
        label: "Nothing's lit",
        onClick: () => {
          const next = new Set(skipped).add(runKey(run.zone, run.start));
          setSkipped(next);
          advance(next, run, run.count);
        },
      }}
    >
      <p className={s.hint}>If you can&apos;t see it, it may be behind something. Skip it and the next one lights.</p>
    </StepFrame>
  );
}
