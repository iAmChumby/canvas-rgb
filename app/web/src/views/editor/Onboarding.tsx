"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowCounterClockwise, ArrowRight, X } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { BrandMark, Wordmark } from "@/components/BrandMark";
import { Segmented } from "@/components/Segmented";
import { FansStep } from "@/views/editor/FansStep";
import { FinishStep } from "@/views/editor/FinishStep";
import { FixedStep } from "@/views/editor/FixedStep";
import { HardwareLine } from "@/views/editor/HardwareLine";
import { HelloStep } from "@/views/editor/HelloStep";
import { StepFrame } from "@/views/editor/OnboardStep";
import { PlaceStep } from "@/views/editor/PlaceStep";
import { ProgressDots } from "@/views/editor/ProgressDots";
import { buildSteps, type Scope, type Step } from "@/views/editor/flow";
import { SWEEP_EFFECT, useOnboardEffects } from "@/views/editor/onboardEffects";
import { CASE_ANCHOR, CASE_GUIDES, PART_LABEL, PART_SIZE, type Sel } from "@/views/editor/parts";
import { setupZones } from "@/views/editor/setup";
import { rememberLights } from "@/views/editor/newLights";
import { setTakeover } from "@/views/editor/takeover";
import { api } from "@/lib/endpoints";
import { refreshLayouts, refreshStatus, useSnap } from "@/lib/store";
import { formatLength, placementName, zoneKey, type LengthUnit } from "@/lib/zones";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { CaseGuide } from "@/views/PreviewStage";
import type { Layout } from "@/proto/Layout";
import type { PartKind } from "@/proto/PartKind";
import type { PlacedZone } from "@/proto/PlacedZone";

type Pick = (sel: Sel[]) => void;

/** "37.0 cm" reads as "37 cm"; inches keep their decimals. */
const trimCm = (text: string) => text.replace(/\.0 cm$/, " cm");

/** A case length as a person says it: whole centimetres, or inches to a tenth. */
const caseText = (units: number, unit: LengthUnit) =>
  unit === "in" ? `${(units / 6.35).toFixed(1)} in` : `${Math.round(units / 2.5)} cm`;

/**
 * The onboarding: mapping the PC as six screens (Hello, Case, Fans, Fixed devices, Place,
 * Finish) that come to the owner. It takes over the window: the rail, the strip and the banners
 * are hidden (see `takeover.ts`). The drawing of the PC stays on screen at every step, because the
 * owner matches the real lights to it, and one panel beside it (a bottom sheet on a phone) says
 * what to do next.
 */
export function Onboarding({
  layout,
  devices,
  scope,
  again,
  unit,
  onUnit,
  selection,
  canvas,
  onChange,
  onSelect,
  onEnsureCase,
  onAddPart,
  onChase,
  onFlash,
  onLit,
  onLater,
  onFinished,
  saveError,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  /** The part the run is limited to, when one device is redone. */
  scope: Scope | null;
  /** The owner asked to map the PC again: the greeting is for the first time only. */
  again: boolean;
  unit: LengthUnit;
  /** The owner picked the other unit. */
  onUnit: (unit: LengthUnit) => void;
  /** The parts that are picked on the drawing. */
  selection: Sel[];
  /**
   * The canvas, with the highlighted parts the step asks for, where its taps go, the spotlight
   * (every other part is dimmed), what the pointer is over, and the case guides.
   */
  canvas: (extra: {
    pulse: Sel[];
    spotlight: Sel[] | null;
    onSelect: (sel: Sel[]) => void;
    onHoverPart: (sel: Sel | null) => void;
    guides: CaseGuide[];
    unlockBadge: boolean;
  }) => ReactNode;
  onChange: (fn: (l: Layout) => Layout) => void;
  onSelect: (sel: Sel[]) => void;
  /** Makes sure the layout has a case box (mid-ATX when new) and picks it. */
  onEnsureCase: () => void;
  onAddPart: (kind: PartKind) => void;
  onChase: (placementIndex: number) => void;
  onFlash: (placementIndex: number) => void;
  onLit: (lit: { deviceId: string; zoneIndex: number; led: number; count?: number } | null) => void;
  /** "Set up later": the window closes until the next app start. */
  onLater: () => void;
  /** The owner reached the end: the daemon knows, and the flow closes for good. */
  onFinished: () => void;
  saveError: string | null;
}) {
  const [steps, setSteps] = useState<Step[]>(() => buildSteps(layout, devices, scope));
  const first = again && scope === null ? 1 : 0;
  const [at, setAt] = useState(first);
  const [dir, setDir] = useState<"fwd" | "back">("fwd");
  const step = steps[at] ?? steps[steps.length - 1]!;
  const go = (to: number) => {
    setDir(to >= at ? "fwd" : "back");
    setAt(Math.max(0, Math.min(steps.length - 1, to)));
  };
  const next = () => go(at + 1);
  const back = at > first ? () => go(at - 1) : null;

  const status = useSnap((st) => st.status);
  const zones = useMemo(() => setupZones(layout, devices), [layout, devices]);
  const { play, restore, keep } = useOnboardEffects(layout.effect);

  // The onboarding has the whole window while it is on screen.
  useEffect(() => {
    setTakeover(true);
    return () => setTakeover(false);
  }, []);

  /** The parts the owner could not see: their dots are hollow. */
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(() => new Set());
  const onSkip = useCallback(
    (z: PlacedZone) => {
      setSkipped((cur) => new Set(cur).add(zoneKey(z)));
      onChange((l) => ({
        ...l,
        zones: l.zones.map((p) =>
          p.device_id === z.device_id && p.zone_index === z.zone_index && p.leds?.start === z.leds?.start
            ? { ...p, part: { ...(p.part ?? { kind: "generic", facing: "viewer" }), mapped: true } }
            : p,
        ),
      }));
    },
    [onChange],
  );

  // The case is picked while it is sized; nothing is picked on the other screens.
  const stepKey =
    step.kind === "fans"
      ? `fans:${step.deviceId}#${step.zoneIndex}`
      : step.kind === "fixed"
        ? `fixed:${step.zones[0]?.deviceId ?? ""}`
        : step.kind;
  const ensureRef = useRef(onEnsureCase);
  const selectRef = useRef(onSelect);
  const unlockRef = useRef<() => void>(() => undefined);
  useEffect(() => {
    ensureRef.current = onEnsureCase;
    selectRef.current = onSelect;
  });
  useEffect(() => {
    if (step.kind === "case") {
      ensureRef.current();
      // Coming back to the Case step (Back) lets the owner move the case again.
      unlockRef.current();
    } else selectRef.current([]);
    // Only a change of step moves the selection.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stepKey]);

  // Where the drawing's taps go: a screen that asks for them (Fans) takes them, the rest lets
  // the drawing select as usual.
  const [pick, setPickState] = useState<{ fn: Pick } | null>(null);
  const setPick = useCallback((fn: Pick | null) => setPickState(fn ? { fn } : null), []);
  const stageSelect = useCallback(
    (sel: Sel[]) => {
      if (pick) {
        if (sel.length > 0) pick.fn(sel);
        return;
      }
      onSelect(sel);
    },
    [pick, onSelect],
  );

  // The parts the drawing points at: the fan the light is on, the device on a fixed screen, the
  // case while it is sized, and, while the owner places, the part under the pointer or picked.
  const [focus, setFocus] = useState<number | null>(null);
  const [hovered, setHovered] = useState<Sel | null>(null);
  const placing = step.kind === "place";

  const fixedParts = useMemo<Sel[]>(() => {
    if (step.kind !== "fixed") return [];
    const out: Sel[] = [];
    layout.zones.forEach((z, index) => {
      if (step.zones.some((k) => k.deviceId === z.device_id && k.zoneIndex === z.zone_index)) out.push({ kind: "zone", index });
    });
    return out;
  }, [step, layout.zones]);

  const pulse: Sel[] = useMemo(() => {
    if (step.kind === "fans") return focus !== null ? [{ kind: "zone", index: focus }] : [];
    if (step.kind === "fixed") return fixedParts;
    if (step.kind === "case") {
      const i = (layout.landmarks ?? []).findIndex((l) => l.kind === "case");
      return i >= 0 ? [{ kind: "landmark", index: i }] : [];
    }
    return [];
  }, [step, focus, fixedParts, layout.landmarks]);

  // The spotlight: the one part being asked about, every other part a quiet outline. The Place
  // screen spotlights the part under the pointer or picked, and nothing while there is none.
  const spotlight: Sel[] | null = useMemo(() => {
    if (step.kind === "fans") return pulse;
    if (step.kind === "fixed") return fixedParts;
    if (placing) {
      const on = [...(hovered ? [hovered] : []), ...selection];
      return on.length > 0 ? on : null;
    }
    return null;
  }, [step.kind, pulse, fixedParts, placing, hovered, selection]);

  /** The name of the part the Place screen is pointing at. */
  const showing = (() => {
    if (!placing) return null;
    const p = hovered ?? (selection.length === 1 ? selection[0] : null);
    if (!p) return null;
    if (p.kind === "landmark") {
      const l = layout.landmarks?.[p.index];
      return l ? l.label?.trim() || PART_LABEL[l.kind] : null;
    }
    const z = layout.zones[p.index];
    return z ? placementName(devices, z, layout.zones) : null;
  })();

  // ---- the case ----
  const caseBox = (layout.landmarks ?? []).find((l) => l.kind === "case");
  /** Sets the case's box (a guide or the reset), standing on the same floor line, and its lock. */
  const setCaseBox = (box: { width: number; height: number } | null, locked?: boolean) => {
    onChange((l) => ({
      ...l,
      landmarks: (l.landmarks ?? []).map((m) =>
        m.kind !== "case"
          ? m
          : {
              ...m,
              ...(box
                ? { width: box.width, height: box.height, x: CASE_ANCHOR.x - box.width / 2, y: CASE_ANCHOR.y - box.height }
                : {}),
              ...(locked === undefined ? {} : { locked }),
            },
      ),
    }));
  };
  const layoutRef = useRef(layout);
  useEffect(() => {
    layoutRef.current = layout;
  });
  const setCaseLocked = (locked: boolean) => {
    const cur = layoutRef.current.landmarks?.find((m) => m.kind === "case");
    if (!cur || (cur.locked === true) === locked) return;
    setCaseBox(null, locked);
  };
  useEffect(() => {
    unlockRef.current = () => setCaseLocked(false);
  });
  const atDefault =
    !!caseBox &&
    Math.abs(caseBox.width - PART_SIZE.case.width) < 0.01 &&
    Math.abs(caseBox.height - PART_SIZE.case.height) < 0.01 &&
    Math.abs(caseBox.x + caseBox.width / 2 - CASE_ANCHOR.x) < 0.5 &&
    Math.abs(caseBox.y + caseBox.height - CASE_ANCHOR.y) < 0.5;
  const labelX = CASE_ANCHOR.x - Math.max(...CASE_GUIDES.map((g) => g.cmWide)) * 1.25;
  const guides: CaseGuide[] =
    step.kind === "case"
      ? CASE_GUIDES.map((g) => {
          const width = g.cmWide * 2.5;
          const height = g.cmTall * 2.5;
          return {
            id: g.id,
            name: g.name,
            size: `${trimCm(formatLength(g.cmWide * 10, unit))} × ${trimCm(formatLength(g.cmTall * 10, unit))}`,
            x: CASE_ANCHOR.x - width / 2,
            y: CASE_ANCHOR.y - height,
            width,
            height,
            labelX,
            onPick: () => setCaseBox({ width, height }),
          };
        })
      : [];

  // Leaving puts the owner's own effect back; the end keeps the one they picked.
  const [picked, setPicked] = useState<string | null>(null);
  const later = async () => {
    await restore();
    onLater();
  };
  const choose = async (id: string) => {
    keep();
    setPicked(id);
    try {
      await api.activateEffect(id, { props: {}, preset: null });
      await Promise.all([refreshLayouts(), refreshStatus()]);
    } catch {
      // The tile stays marked; the daemon's refusal shows in the strip once the window is back.
    }
  };
  const finish = async () => {
    if (!picked) await restore();
    try {
      await api.completeOnboarding();
      rememberLights(devices);
      await refreshStatus();
    } catch {
      // The window closes anyway; it opens again at the next start if the daemon never heard.
    }
    onFinished();
  };

  /** Leaves the hello: a rescan may have found lights, so the screens are worked out again. */
  const start = () => {
    if (scope === null) setSteps(buildSteps(layout, devices, scope));
    setDir("fwd");
    setAt(1);
  };

  const gated = step.kind === "hello" && (status?.conflicts.some((c) => c.kind === "signal_rgb") ?? false);

  const body = ((): ReactNode => {
    switch (step.kind) {
      case "hello":
        return <HelloStep onStart={start} onLater={() => void later()} play={play} />;
      case "case":
        return (
          <StepFrame
            center
            kicker="Case"
            title="Is this your case?"
            lead="A typical mid tower. Adjust only if yours is clearly bigger or smaller."
            onBack={back}
            primary={{
              label: "Looks right",
              icon: <ArrowRight size={16} weight="bold" />,
              onClick: () => {
                // The case stays where it is for the rest of the onboarding.
                setCaseLocked(true);
                next();
              },
            }}
            secondary={{ label: "Reset", icon: <ArrowCounterClockwise size={14} />, onClick: () => setCaseBox(PART_SIZE.case) }}
          >
            <div className={s.size} aria-live="polite" data-default={atDefault}>
              <span className={s.sizeValue}>{caseBox ? caseText(caseBox.width, unit) : "\u2014"}</span>
              <span className={s.sizeBy} aria-hidden="true">
                ×
              </span>
              <span className={s.sizeValue}>{caseBox ? caseText(caseBox.height, unit) : "\u2014"}</span>
            </div>
            <div className={s.unitRow}>
              <p className={s.hint}>The dashed outlines are other tower sizes, to compare.</p>
              <span className={s.unitSwitch}>
                <Segmented<LengthUnit>
                  label="The unit every size is read in"
                  value={unit}
                  options={[
                    { value: "cm", label: "cm" },
                    { value: "in", label: "in" },
                  ]}
                  onChange={onUnit}
                />
              </span>
            </div>
          </StepFrame>
        );
      case "fans": {
        const zone = zones.find((x) => x.deviceId === step.deviceId && x.zoneIndex === step.zoneIndex);
        if (!zone) return null;
        return (
          <FansStep
            key={stepKey}
            layout={layout}
            devices={devices}
            zone={zone}
            onChange={onChange}
            onBack={back}
            onNext={next}
            setPick={setPick}
            onFocusPart={setFocus}
            onLit={onLit}
            onChase={(i) => {
              if (i >= 0) onChase(i);
            }}
            onSkip={onSkip}
            skipped={skipped}
          />
        );
      }
      case "fixed":
        return (
          <FixedStep
            layout={layout}
            devices={devices}
            zones={step.zones}
            onChange={onChange}
            onBack={back}
            onNext={next}
            onFlash={onFlash}
          />
        );
      case "place":
        return (
          <PlaceStep
            layout={layout}
            devices={devices}
            unit={unit}
            showing={showing}
            onAddPart={onAddPart}
            onBack={back}
            onNext={next}
          />
        );
      case "finish":
        return (
          <FinishStep
            layout={layout}
            devices={devices}
            scoped={scope !== null}
            playing={picked}
            onSweep={() => void play(SWEEP_EFFECT)}
            onPick={(id) => void choose(id)}
            onDone={() => void finish()}
            saveError={saveError}
          />
        );
    }
  })();

  const showProgress = step.kind !== "hello" && step.kind !== "case";

  return (
    <div className={s.root} data-step={step.kind}>
      <header className={s.bar}>
        <span className={s.brand}>
          <BrandMark size={18} />
          <Wordmark />
        </span>
        <span className={s.barEnd}>
          {step.kind !== "hello" && step.kind !== "finish" && (
            <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={() => void later()}>
              Set up later
            </Button>
          )}
        </span>
        {showProgress && (
          <div className={s.barProgress}>
            <ProgressDots layout={layout} devices={devices} skipped={skipped} celebrate={step.kind === "finish"} />
          </div>
        )}
      </header>
      <HardwareLine layout={layout} devices={step.kind === "hello" || step.kind === "case" ? null : devices} gated={gated} saveError={saveError} />

      <div className={s.main}>
        <div className={s.stage}>
          {canvas({
            pulse,
            spotlight,
            onSelect: stageSelect,
            onHoverPart: setHovered,
            guides,
            unlockBadge: caseBox?.locked === true,
          })}
        </div>
        <section className={s.panel} aria-label={`Screen ${at + 1} of ${steps.length}`}>
          <div key={stepKey} className={s.stepWrap} data-dir={dir}>
            {body}
          </div>
        </section>
      </div>
    </div>
  );
}
