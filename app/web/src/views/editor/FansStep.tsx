"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, ArrowsClockwise, Check, Minus, Plus } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Toggle } from "@/components/Toggle";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { Fold, HoldKey, StepFrame } from "@/views/editor/OnboardStep";
import { useIdentifyWalker } from "@/views/editor/IdentifyWalker";
import type { Sel } from "@/views/editor/parts";
import { FAN_LEDS, segmentAt, segmentsOf, setupZones, splitAt, placeForPart, type SetupZone } from "@/views/editor/setup";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshDevices } from "@/lib/store";
import { placementName, zoneKey } from "@/lib/zones";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";

/**
 * One screen of the Fans step, for one header with several LEDs: the daemon lights one LED, and
 * the owner says which fan it is by tapping it on the drawing.
 *
 * Nothing here keeps its own state except where the light is standing. The splits, the order and
 * the confirmations all live in the layout, so what is done survives a restart.
 *
 *  - The tap swaps where two parts sit on the drawing: the lit LED's part moves to the fan that
 *    was tapped, and the one that was there takes its old place. The pulse follows, so the
 *    drawing always shows what the app believes.
 *  - "This fan ends here" splits the chain after the lit LED, confirms this fan and moves on.
 *  - The light is the whole fan: the engine shows nothing else while the screen is open, so the
 *    one lit run is easy to find. A header not yet split lights from the fan's first LED up to
 *    the LED the owner has stepped to.
 *  - "Can't see it" first blinks the same run. If it is still lost, the owner skips this fan
 *    (its dot stays hollow) or moves on to the next light.
 */
export interface FansProps {
  layout: Layout;
  devices: DeviceInfo[] | null;
  zone: SetupZone;
  /** One change to the layout, one undo step. */
  onChange: (fn: (l: Layout) => Layout) => void;
  onBack: (() => void) | null;
  onNext: () => void;
  /** The drawing's taps come here while this screen is open. */
  setPick: (fn: ((sel: Sel[]) => void) | null) => void;
  /** The part the light is on, so the drawing can pulse it. */
  onFocusPart: (placement: number | null) => void;
  /** Told which LED is lit, so the desk can mark it. */
  onLit: (lit: { deviceId: string; zoneIndex: number; led: number; count?: number } | null) => void;
  /** Starts the chase on a zone, so the owner can see which way its LEDs run. */
  onChase: (placementIndex: number) => void;
  /** Marks a part as skipped ("Can't see it"): confirmed, with a hollow dot. */
  onSkip: (placement: PlacedZone) => void;
  /** The keys (`zoneKey`) of the parts the owner could not see. */
  skipped: ReadonlySet<string>;
}

export function FansStep(props: FansProps) {
  const device = props.devices?.find((d) => d.id === props.zone.deviceId) ?? null;
  const zoneInfo = device?.zones.find((z) => z.index === props.zone.zoneIndex) ?? null;
  if (!device || !zoneInfo) return null;
  return <FansInner {...props} device={device} zoneInfo={zoneInfo} />;
}

/** What the screen calls the parts on this header. */
function nounOf(layout: Layout, zone: SetupZone): string {
  const kinds = new Set(zone.placements.map((i) => layout.zones[i]?.part?.kind ?? "generic"));
  if (kinds.has("fan")) return "fan";
  if (kinds.size === 1 && kinds.has("strip")) return "strip";
  return "light";
}

/** Everything about where a part sits on the drawing. */
const SPOT = ["x", "y", "width", "height", "rotation_deg", "shape", "label"] as const;

function FansInner({
  layout,
  devices,
  zone,
  device,
  zoneInfo,
  onChange,
  onBack,
  onNext,
  setPick,
  onFocusPart,
  onLit,
  onChase,
  onSkip,
  skipped,
}: FansProps & { device: DeviceInfo; zoneInfo: ZoneInfo }) {
  // The zone as the layout holds it now (its count and its parts change while it is walked).
  const live = useMemo(
    () => setupZones(layout, devices).find((z) => z.deviceId === zone.deviceId && z.zoneIndex === zone.zoneIndex) ?? zone,
    [layout, devices, zone],
  );
  const segments = segmentsOf(live, layout);
  const noun = nounOf(layout, live);

  // The light starts on the first part that is not confirmed yet.
  const [led, setLed] = useState(() => {
    const first = segments.find((seg) => seg.placement !== null && layout.zones[seg.placement]?.part?.mapped !== true);
    return (first ?? segments[0])?.start ?? 0;
  });
  const under = segmentAt(segments, led);
  const runNo = under ? segments.indexOf(under) : 0;
  const placement = under?.placement ?? null;
  const part = placement !== null ? layout.zones[placement] ?? null : null;

  // What is lit: the whole fan. A run longer than one fan is a chain not yet split, and the
  // light grows along it from the fan's first LED to the LED the owner has stepped to.
  const lit = useMemo(() => {
    if (!under) return { start: led, count: 1 };
    if (under.count <= FAN_LEDS) return { start: under.start, count: under.count };
    return { start: under.start, count: Math.max(1, led - under.start + 1) };
  }, [under, led]);
  const walk = useIdentifyWalker(device, zoneInfo, { led, setLed, run: lit });

  // The desk marks the lit run, and the drawing pulses the part it belongs to.
  useEffect(() => {
    onLit({ deviceId: zone.deviceId, zoneIndex: zone.zoneIndex, led: lit.start, count: lit.count });
  }, [zone.deviceId, zone.zoneIndex, lit.start, lit.count, onLit]);
  useEffect(() => () => onLit(null), [onLit]);
  useEffect(() => {
    onFocusPart(placement);
  }, [placement, onFocusPart]);
  useEffect(() => () => onFocusPart(null), [onFocusPart]);

  /** Swaps where two parts sit on the drawing. */
  const swap = (a: number, b: number) => {
    onChange((l) => {
      const za = l.zones[a];
      const zb = l.zones[b];
      if (!za || !zb) return l;
      const pa: Record<string, unknown> = {};
      const pb: Record<string, unknown> = {};
      for (const k of SPOT) {
        pa[k] = za[k];
        pb[k] = zb[k];
      }
      const next = l.zones.slice();
      next[a] = {
        ...za,
        ...(pb as Partial<PlacedZone>),
        part: za.part && { ...za.part, facing: zb.part?.facing ?? za.part.facing },
      };
      next[b] = {
        ...zb,
        ...(pa as Partial<PlacedZone>),
        part: zb.part && { ...zb.part, facing: za.part?.facing ?? zb.part.facing },
      };
      return { ...l, zones: next };
    });
  };

  // The tap on the drawing: the lit part is the one that was tapped. The drawing reports a tap
  // when the pointer goes down, and the swap waits for it to come up (and not to have moved), so
  // the drawing's own move-drag has ended before the parts change places, and a drag that
  // really moves a part is never taken for a tap. A key or a focus has no pointer: it swaps now.
  const downAt = useRef<{ x: number; y: number; t: number } | null>(null);
  useEffect(() => {
    const on = (e: PointerEvent) => {
      downAt.current = { x: e.clientX, y: e.clientY, t: performance.now() };
    };
    window.addEventListener("pointerdown", on, true);
    return () => window.removeEventListener("pointerdown", on, true);
  }, []);
  const tapRef = useRef<(sel: Sel[]) => void>(() => undefined);
  useEffect(() => {
    tapRef.current = (sel) => {
      const hit = sel[sel.length - 1];
      if (!hit || hit.kind !== "zone" || placement === null || hit.index === placement) return;
      const from = downAt.current;
      if (!from || performance.now() - from.t > 80) {
        swap(placement, hit.index);
        return;
      }
      const lit = placement;
      const onUp = (e: PointerEvent) => {
        if (Math.hypot(e.clientX - from.x, e.clientY - from.y) > 5) return;
        requestAnimationFrame(() => swap(lit, hit.index));
      };
      window.addEventListener("pointerup", onUp, { once: true });
    };
  });
  useEffect(() => {
    setPick((sel) => tapRef.current(sel));
    return () => setPick(null);
  }, [setPick]);

  const confirmAt = (idx: number | null) => {
    if (idx === null) return;
    onChange((l) => ({
      ...l,
      zones: l.zones.map((z, i) =>
        i === idx ? { ...z, part: { ...(z.part ?? { kind: "generic", facing: "viewer" }), mapped: true } } : z,
      ),
    }));
  };

  /** Moves the light to the next part of this header, or on to the next screen after the last. */
  const advance = () => {
    const after = segments[runNo + 1];
    if (after) setLed(after.start);
    else onNext();
  };

  const confirm = () => {
    confirmAt(placement);
    advance();
  };

  const skip = () => {
    if (part) onSkip(part);
    advance();
  };

  /** "Can't see it": the first press blinks the run; then the owner can skip it or try the next. */
  const [tried, setTried] = useState<number | null>(null);
  const [blinking, setBlinking] = useState(false);
  const cantSee = async () => {
    setTried(runNo);
    setBlinking(true);
    try {
      await walk.relight();
    } finally {
      setBlinking(false);
    }
  };
  const lost = tried === runNo;

  /** The split: this part ends at the lit LED, and the next one starts after it. */
  const endsHere = () => {
    if (!under || placement === null) return;
    const cut = splitAt(segments, led + 1);
    if (!cut) return;
    const kind = part?.part?.kind ?? "fan";
    const startOfThis = under.start;
    onChange((l) => {
      const template = l.zones[placement];
      if (!template) return l;
      const mine = (z: PlacedZone) => z.device_id === zone.deviceId && z.zone_index === zone.zoneIndex;
      const built: PlacedZone[] = cut.map((seg) => {
        const src = seg.placement !== null ? l.zones[seg.placement] ?? template : template;
        const fresh = seg.placement === null;
        return {
          ...src,
          leds: { start: seg.start, count: seg.count },
          part: {
            kind: src.part?.kind ?? kind,
            facing: src.part?.facing ?? "viewer",
            // The part that ends here is confirmed; the one that starts after it is not.
            mapped: fresh ? false : seg.start === startOfThis ? true : src.part?.mapped ?? false,
          },
          label: fresh ? undefined : src.label,
          ...(fresh ? placeForPart(l, kind, template) : {}),
        };
      });
      const out: PlacedZone[] = [];
      let done = false;
      l.zones.forEach((z) => {
        if (mine(z)) {
          if (!done) {
            out.push(...built);
            done = true;
          }
          return;
        }
        out.push(z);
      });
      if (!done) out.push(...built);
      return { ...l, zones: out };
    });
    setLed(led + 1);
  };

  const reverse = () => {
    onChange((l) => ({
      ...l,
      zones: l.zones.map((z) =>
        z.device_id === zone.deviceId && z.zone_index === zone.zoneIndex ? { ...z, reversed: !z.reversed } : z,
      ),
    }));
  };

  const reversed = layout.zones[live.placements[0] ?? -1]?.reversed === true;
  const runEnd = under ? under.start + under.count - 1 : led;
  const last = runNo >= segments.length - 1;
  const [sizeError, setSizeError] = useState<string | null>(null);
  const resize = async (count: number) => {
    setSizeError(null);
    try {
      await api.patchZone(device.id, zoneInfo.index, { led_count: count });
      await refreshDevices();
    } catch (e) {
      setSizeError(messageOf(e));
    }
  };

  return (
    <StepFrame
      kicker={
        <>
          {part ? placementName(devices, part, layout.zones) : device.name}
          {segments.length > 1 && (
            <>
              {" · "}
              <span className="fig">{runNo + 1}</span> of <span className="fig">{segments.length}</span>
            </>
          )}
        </>
      }
      title={`Which ${noun} is lit?`}
      lead="Tap it on the drawing."
      onBack={onBack}
      primary={{
        label: last ? "Done" : `Next ${noun}`,
        icon: last ? <Check size={16} weight="bold" /> : <ArrowRight size={16} weight="bold" />,
        onClick: confirm,
      }}
      secondary={{ label: blinking ? "Blinking" : "Can't see it", onClick: () => void cantSee() }}
    >
      {lost && (
        <div className={s.lost} role="status">
          <p className={s.lostText}>
            {blinking ? `Blinking the ${noun} now.` : `Still lost? Skip this ${noun}, or try the next light.`}
          </p>
          {!blinking && (
            <span className={s.lostRow}>
              <Button size="sm" variant="default" onClick={skip}>
                Skip this one
              </Button>
              <Button size="sm" variant="default" icon={<ArrowRight size={14} />} onClick={advance}>
                Next light
              </Button>
            </span>
          )}
        </div>
      )}
      <div className={s.walker}>
        <div className={s.walkerNumber}>
          <span className={s.walkerLabel}>LED</span>
          <span className={s.walkerBig}>
            <AnimatedNumber value={led + 1} />
          </span>
          <span className="mute fig">/ {live.ledCount}</span>
        </div>
        <span className={s.walkerKeys}>
          <HoldKey
            label="Previous LED"
            icon={<ArrowLeft size={18} />}
            disabled={led <= 0}
            onStep={() => setLed((v) => Math.max(0, v - 1))}
          />
          <HoldKey
            label="Next LED"
            icon={<ArrowRight size={18} />}
            disabled={led >= walk.maxIndex}
            onStep={() => setLed((v) => Math.min(walk.maxIndex, v + 1))}
          />
        </span>
      </div>
      <Button variant="default" disabled={!under || led >= runEnd} onClick={endsHere}>
        This {noun} ends here
      </Button>
      <ol className={s.chain} aria-label={`The ${noun}s on this header`}>
        {segments.map((seg, i) => {
          const z = seg.placement !== null ? layout.zones[seg.placement] : undefined;
          const state = !z?.part?.mapped ? "todo" : skipped.has(zoneKey(z)) ? "skipped" : "lit";
          return (
            <li key={`${seg.start}-${seg.count}`}>
              <button type="button" className={s.chainRow} data-now={i === runNo} onClick={() => setLed(seg.start)}>
                <i className={s.dot} data-state={state} aria-hidden="true" />
                <span className={s.chainName}>{z ? placementName(devices, z, layout.zones) : `${noun} ${i + 1}`}</span>
                <span className={s.chainRange}>
                  {seg.start + 1}–{seg.start + seg.count}
                </span>
              </button>
            </li>
          );
        })}
      </ol>
      <Fold label="Fine-tune">
        <div className={s.tune}>
          <span className={s.tuneLabel}>Reverse the direction</span>
          <Toggle checked={reversed} label="Reverse the direction" onChange={reverse} />
        </div>
        <div className={s.tune}>
          <span className={s.tuneLabel}>See which way it runs</span>
          <Button
            size="sm"
            variant="ghost"
            icon={<ArrowsClockwise size={14} />}
            onClick={() => onChase(live.placements[runNo] ?? live.placements[0] ?? -1)}
          >
            Show
          </Button>
        </div>
        <div className={s.tune}>
          <span className={s.tuneLabel}>LEDs on this header</span>
          {live.resizable ? (
            <span className={s.stepper}>
              <IconButton
                label="Fewer LEDs"
                size="sm"
                icon={<Minus size={14} />}
                disabled={live.ledCount <= live.minLeds}
                onClick={() => void resize(live.ledCount - 1)}
              />
              <span className="fig">{live.ledCount}</span>
              <IconButton
                label="More LEDs"
                size="sm"
                icon={<Plus size={14} />}
                disabled={live.ledCount >= live.maxLeds}
                onClick={() => void resize(live.ledCount + 1)}
              />
            </span>
          ) : (
            <span className={s.fixedNote}>
              <span className="fig">{live.ledCount}</span> · fixed by the hardware
            </span>
          )}
        </div>
        {(sizeError || walk.error) && <p className="error-line">{sizeError ?? walk.error}</p>}
      </Fold>
    </StepFrame>
  );
}
