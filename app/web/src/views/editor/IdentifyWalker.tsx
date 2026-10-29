"use client";

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, ArrowRight, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import { Panel, StateDot } from "@/components/Panel";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshDevices } from "@/lib/store";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { ZoneInfo } from "@/proto/ZoneInfo";

/** How often the walker asks the engine to keep the light on (ms): inside its few-second watchdog. */
const RESEND_MS = 2000;
/** One half of a blink (ms), and how many blinks "Can't see it" makes. */
const BLINK_MS = 300;
const BLINKS = 4;

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/**
 * The walker's behaviour: one LED of a zone is lit at a time, ← and → step it, and "This is the
 * last LED" sets the zone's count to the LED just lit.
 *
 * The same behaviour is used in two places: on the Devices page, where it sizes a header, and
 * in the guided setup, where it walks a zone while the owner splits it into parts. Leaving the
 * walker, or the page, sends `stop: true`.
 */
export function useIdentifyWalker(
  device: DeviceInfo,
  zone: ZoneInfo,
  options?: {
    /** The LED the walker is on, when something else drives it. */
    led?: number;
    setLed?: (led: number) => void;
    /** Told when the zone's length was set. */
    onLengthSet?: (count: number) => void;
    /**
     * The run of LEDs to light (a whole fan), when it is more than the one LED the walker is on.
     * The engine shows only this run until the walker leaves.
     */
    run?: { start: number; count: number } | null;
  },
) {
  const [own, setOwn] = useState(0);
  const at = options?.led ?? own;
  const set = options?.setLed ?? setOwn;
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const stopped = useRef(false);
  /** The request that lights what the walker is on now. */
  const runStart = options?.run?.start ?? at;
  const runCount = Math.max(1, options?.run?.count ?? 1);
  const wantRef = useRef({ led: runStart, count: runCount });
  useEffect(() => {
    wantRef.current = { led: runStart, count: runCount };
  }, [runStart, runCount]);
  /** True while a relight is blinking: the steady resend waits. */
  const blinking = useRef(false);
  const maxIndex = Math.max(0, zone.max_leds - 1);

  const stop = useCallback(() => {
    if (stopped.current) return;
    stopped.current = true;
    void api.identify(device.id, { zone: zone.index, led: 0, stop: true }).catch(() => undefined);
  }, [device.id, zone.index]);

  /** One identify request for the run the walker is on. */
  const light = useCallback(() => {
    const { led, count } = wantRef.current;
    return api.identify(device.id, { zone: zone.index, led, ...(count > 1 ? { count } : {}), stop: false });
  }, [device.id, zone.index]);

  // Light the current LED (or run) whenever it changes.
  useEffect(() => {
    stopped.current = false;
    let cancelled = false;
    light()
      .then(() => {
        if (!cancelled) setError(null);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(messageOf(e));
      });
    return () => {
      cancelled = true;
    };
  }, [light, runStart, runCount]);

  // The engine ends an identify a few seconds after the last request, so a closed window can
  // never leave the PC dark: while the walker is open it asks again every two seconds.
  useEffect(() => {
    const t = setInterval(() => {
      if (stopped.current || blinking.current) return;
      void light().catch(() => undefined);
    }, RESEND_MS);
    return () => clearInterval(t);
  }, [light]);

  /**
   * "Can't see it": the same run goes off and on in a 300 ms rhythm a few times, which the eye
   * finds where a steady light is lost. Resolves when it has finished.
   */
  const relight = useCallback(async () => {
    if (blinking.current) return;
    blinking.current = true;
    try {
      for (let i = 0; i < BLINKS; i++) {
        if (stopped.current) return;
        await api.identify(device.id, { zone: zone.index, led: 0, stop: true }).catch(() => undefined);
        await sleep(BLINK_MS);
        if (stopped.current) return;
        await light().catch(() => undefined);
        await sleep(BLINK_MS);
      }
    } finally {
      blinking.current = false;
    }
  }, [device.id, zone.index, light]);

  // Stop on unmount (leaving the page or closing the walker) and when the zone changes.
  useEffect(() => {
    const id = device.id;
    const index = zone.index;
    return () => {
      if (stopped.current) return;
      stopped.current = true;
      void api.identify(id, { zone: index, led: 0, stop: true }).catch(() => undefined);
    };
  }, [device.id, zone.index]);

  // ← and → step the walker when focus is not in a text field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) return;
      if (e.key === "ArrowRight") set(Math.min(maxIndex, at + 1));
      if (e.key === "ArrowLeft") set(Math.max(0, at - 1));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [maxIndex, at, set]);

  const setLength = useCallback(async () => {
    setError(null);
    setNote(null);
    try {
      await api.patchZone(device.id, zone.index, { led_count: at + 1 });
      await refreshDevices();
      stop();
      setNote(`${zone.name} is now ${at + 1} LEDs.`);
      options?.onLengthSet?.(at + 1);
    } catch (e) {
      setError(messageOf(e));
    }
  }, [at, device.id, zone.index, zone.name, options, stop]);

  return { led: at, setLed: set, maxIndex, error, note, setLength, stop, setNote, setError, relight };
}

/**
 * Walks a lit LED along one zone (POST identify). ← and → step it. "This is the last LED" sets the
 * zone's length to the LED just lit. Leaving the walker, or the page, sends `stop: true`.
 *
 * The Devices page's own view of it. The guided setup uses the same behaviour through
 * `useIdentifyWalker` and draws its own panel around it.
 */
export function IdentifyWalker({
  device,
  zone,
  onClose,
  children,
}: {
  device: DeviceInfo;
  zone: ZoneInfo;
  onClose: () => void;
  /** Extra controls, under the walker's own. */
  children?: ReactNode;
}) {
  const walk = useIdentifyWalker(device, zone);

  const exit = () => {
    walk.stop();
    onClose();
  };

  return (
    <Panel
      title={`Identify ${zone.name}`}
      actions={<IconButton label="Stop walking" icon={<X size={16} />} onClick={exit} />}
    >
      <div className="stack" style={{ gap: 14 }}>
        <p className="dim one-line" style={{ whiteSpace: "normal" }}>
          {device.name}
        </p>
        <div className="walker-led">
          <span className="walker-big"><AnimatedNumber value={walk.led + 1} /></span>
          <span className="mute">of {zone.max_leds} LEDs</span>
        </div>
        <p className="mute" style={{ fontSize: 14, marginTop: -4 }}>
          Set to <span className="fig"><AnimatedNumber value={zone.led_count} /></span>
          {zone.resizable ? "" : " · detected"}
        </p>
        <div className="walker-keys">
          <IconButton
            label="Previous LED"
            icon={<ArrowLeft size={18} />}
            disabled={walk.led <= 0}
            onClick={() => walk.setLed(Math.max(0, walk.led - 1))}
          />
          <IconButton
            label="Next LED"
            icon={<ArrowRight size={18} />}
            disabled={walk.led >= walk.maxIndex}
            onClick={() => walk.setLed(Math.min(walk.maxIndex, walk.led + 1))}
          />
          <NumberField
            label="LED number"
            min={1}
            max={walk.maxIndex + 1}
            value={walk.led + 1}
            onCommit={(v) => walk.setLed(v - 1)}
          />
        </div>
        <Button variant="primary" onClick={() => void walk.setLength()}>
          This is the last LED
        </Button>
        <p className="mute fig" style={{ fontSize: 14 }}>
          {zone.min_leds}–{zone.max_leds}
        </p>
        {walk.note && <StateDot tone="ok">{walk.note}</StateDot>}
        {walk.error && <p className="error-line">{walk.error}</p>}
        {children}
      </div>
    </Panel>
  );
}
