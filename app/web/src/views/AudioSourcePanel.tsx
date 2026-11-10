"use client";

import { useEffect, useRef, useState, type RefObject } from "react";
import { SpeakerHigh } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Field, Panel } from "@/components/Panel";
import { Select, type SelectGroup, type SelectOption } from "@/components/Select";
import { InlineSkeleton } from "@/components/Skeleton";
import { Toggle } from "@/components/Toggle";
import { Popover } from "@/components/Popover";
import { useMediaQuery } from "@/components/MediaQuery";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import type { AudioSources } from "@/proto/AudioSources";
import type { AudioSource } from "@/proto/AudioSource";
import type { AudioTestState } from "@/proto/AudioTestState";

const FOLLOW_VALUE = "follow-default";
const LED_CELLS = 24;
const BAND_CELLS = 32;
/** S3: the disclosure the reviewer asked for — an input feeds the audio
 *  effects whenever one runs, not only while this panel is open. */
const EFFECT_NOTE = "Audio effects read this input whenever an audio effect is active (not only while this panel is open).";
/** The same disclosure for the compact panel's state slot (a short window has no room for
 *  another line); the full sentence is its title and is in the readings popover. */
const EFFECT_NOTE_SHORT = "Audio effects use this input";

/** The words for one honest state: never a blank bar (#166). */
export function audioStatusWords(state: AudioTestState | null): string {
  if (!state) return "Live while this panel is open";
  const name = state.source.name ?? "the default output";
  switch (state.status) {
    case "off":
      return "Live while this panel is open";
    case "waiting":
      return "Waiting for the device";
    case "not_listening":
      return "Press Listen to test this input";
    case "no_device":
      return "No output device";
    case "missing":
      return `Saved device not connected: using ${name}`;
    case "held_exclusive":
      return "Held by another app in exclusive mode";
    case "no_packets":
      return "No audio packets from the device";
    case "silent":
      return "Silent: play something";
    case "live":
      return "Live";
    case "default_changed":
      return `Windows default changed: now ${name}`;
    case "reopening":
      return `Reopening ${name}`;
    case "unsupported_format":
      return `Format not supported (tag ${state.format_tag ?? "?"})`;
    default:
      return "Live while this panel is open";
  }
}

/**
 * The state line. While the device has not answered it is a skeleton line (after the load delay),
 * with the words kept for a screen reader; every other state is its words.
 */
function StateWords({ words, waiting }: { words: string; waiting: boolean }) {
  if (!waiting) return <>{words}</>;
  return (
    <>
      <InlineSkeleton width="22ch" />
      <span className="sr-only">{words}</span>
    </>
  );
}

/** The 24-cell flat LED meter: lit cells take --ok/--work/--warn by level. */
export function LedMeter({ peak }: { peak: number | null }) {
  const lit = peak == null || !Number.isFinite(peak) ? 0 : Math.round(Math.min(1, Math.max(0, peak)) * LED_CELLS);
  return (
    <div className="audio-meter" role="meter" aria-label="Audio level" aria-valuemin={0} aria-valuemax={LED_CELLS} aria-valuenow={lit}>
      {Array.from({ length: LED_CELLS }, (_, i) => {
        const on = i < lit;
        const level = i < Math.round(LED_CELLS * 0.6) ? "ok" : i < Math.round(LED_CELLS * 0.85) ? "work" : "warn";
        return <span key={i} className="audio-cell" data-on={on ? "true" : undefined} data-level={on ? level : undefined} />;
      })}
    </div>
  );
}

/** The dBFS figure: its digits jump straight to the reading, never count up. */
export function DbFigure({ peakDb }: { peakDb: number | null }) {
  return (
    <span className="fig audio-db" aria-label="Peak level in dBFS">
      {peakDb == null || !Number.isFinite(peakDb) ? "—" : peakDb.toFixed(1)}
    </span>
  );
}

/** The 32 band cells the effects read. */
export function BandCells({ bands }: { bands: number[] }) {
  const cells = Array.from({ length: BAND_CELLS }, (_, i) => bands[i] ?? 0);
  return (
    <div className="audio-bands" aria-hidden="true">
      {cells.map((v, i) => (
        <span key={i} className="audio-band" style={{ height: `${Math.round(Math.min(1, Math.max(0, v)) * 100)}%` }} />
      ))}
    </div>
  );
}

/** The readings: meter, dB figure and bands. Fixed height in every state. */
export function AudioReadings({ state }: { state: AudioTestState | null }) {
  return (
    <div className="audio-readings">
      <div className="audio-reading-row">
        <LedMeter peak={state?.peak ?? null} />
        <DbFigure peakDb={state?.peak_db ?? null} />
      </div>
      <BandCells bands={state?.bands ?? []} />
    </div>
  );
}

/** Whether the current pick resolves to an input (S8): from the requested
 *  pick itself, so a saved microphone that is unplugged (and so absent from
 *  the endpoint list) still counts as an input selection. */
export function isInputSelection(sources: AudioSources | null): boolean {
  if (!sources) return false;
  const requested = sources.current.requested;
  if (requested.kind === "endpoint") return requested.flow === "capture";
  return sources.current.flow === "capture";
}

/**
 * The audio-source picker with its live test (#166).
 *
 * Inline vs popover: the readings go inline only where the column has room —
 * 1920x1040, 1440x900 and 960x1032; at 1280x860, 1024x768 and phone width
 * they open in a Popover instead (UI finding 4). In the compact form the
 * panel is about 90 px: the Source select sits in the panel head's action
 * slot, with the state words and the "Show readings" control on one row
 * under it, and the panel's height does not change between states.
 */
export function AudioSourcePanel() {
  const [sources, setSources] = useState<AudioSources | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [listen, setListen] = useState(false);
  const [state, setState] = useState<AudioTestState | null>(null);
  const [popoverOpen, setPopoverOpen] = useState(false);
  const [readingsVisible, setReadingsVisible] = useState(true);
  const [documentVisible, setDocumentVisible] = useState(true);
  const readingsRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLDivElement>(null);
  const popoverFocusRef = useRef<HTMLDivElement>(null);
  const lastPopoverOpen = useRef(false);
  const leaseRef = useRef<string | null>(null);
  const usePopover = useMediaQuery("(max-height: 899px), (max-width: 639px)");

  useEffect(() => {
    let cancelled = false;
    api
      .audioSources()
      .then((s) => {
        if (!cancelled) setSources(s);
      })
      .catch((e: unknown) => {
        if (!cancelled) setErr(messageOf(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // The readings are on screen: inline via IntersectionObserver, or the
  // popover open. Only then is a lease taken.
  useEffect(() => {
    const el = readingsRef.current;
    if (!el) return;
    if (typeof IntersectionObserver === "undefined") {
      return;
    }
    const observer = new IntersectionObserver((entries) => {
      setReadingsVisible(entries[0]?.isIntersecting ?? false);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [usePopover, popoverOpen]);

  useEffect(() => {
    const onVisibility = () => setDocumentVisible(document.visibilityState !== "hidden");
    onVisibility();
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, []);

  const active = (usePopover ? popoverOpen : readingsVisible) && documentVisible;

  // The lease: taken only while the readings are on screen and the document
  // is visible; polled every AUDIO_TEST_POLL_MS (100 ms); a 410 takes a new
  // one; leaving the screen or unmounting ends the lease at once (S2), so
  // turning Listen off closes the microphone immediately.
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;
    const release = () => {
      const id = leaseRef.current;
      leaseRef.current = null;
      if (id) void api.releaseAudioTest(id).catch(() => {});
    };
    const poll = async () => {
      const lease = leaseRef.current;
      if (!lease) return;
      try {
        const next = await api.getAudioTest(lease);
        if (!cancelled) setState(next);
      } catch (e) {
        if (cancelled) return;
        if (e instanceof ApiFailure && e.status === 410) {
          leaseRef.current = null;
          void take();
        }
      }
    };
    const take = async () => {
      // S2: the previous lease (with the old Listen value) ends now, so the
      // microphone closes at once instead of after the daemon's expiry.
      release();
      try {
        const taken = await api.putAudioTest({ listen });
        if (cancelled) {
          void api.releaseAudioTest(taken.lease_id).catch(() => {});
          return;
        }
        leaseRef.current = taken.lease_id;
        await poll();
        if (!cancelled && !timer) {
          timer = setInterval(() => void poll(), 100);
        }
      } catch {
        // The state words keep saying what is happening; a failed lease
        // leaves the last state (or the off words) on screen.
      }
    };
    void take();
    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
      release();
    };
  }, [active, listen]);

  // Focus moves into the popover when it opens and back to the trigger when
  // it closes (never on mount).
  useEffect(() => {
    if (popoverOpen) {
      popoverFocusRef.current?.focus();
    } else if (lastPopoverOpen.current) {
      triggerRef.current?.focus();
    }
    lastPopoverOpen.current = popoverOpen;
  }, [popoverOpen]);

  const value =
    sources?.current.requested.kind === "endpoint" ? sources.current.requested.id : FOLLOW_VALUE;
  const resolvedName = sources?.current.name ?? null;
  const outputs: SelectOption[] = (sources?.endpoints ?? [])
    .filter((e) => e.flow === "render")
    .map((e) => ({
      value: e.id,
      label: e.state === "not_connected" ? `${e.name} (not connected)` : e.name,
    }));
  const inputs: SelectOption[] = (sources?.endpoints ?? [])
    .filter((e) => e.flow === "capture")
    .map((e) => ({
      value: e.id,
      label: e.state === "not_connected" ? `${e.name} (not connected)` : e.name,
    }));
  // UI finding 3: the resolved name appears only once it is known.
  const followLabel = resolvedName ? `Follow Windows default output: ${resolvedName}` : "Follow Windows default output";
  const groups: SelectGroup[] = [
    { label: "", options: [{ value: FOLLOW_VALUE, label: followLabel }] },
    { label: "Outputs (loopback)", options: outputs },
    { label: "Inputs", options: inputs },
  ];
  const flatOptions = groups.flatMap((g) => g.options);
  const inputSelected = isInputSelection(sources);

  const pick = async (next: string) => {
    if (!sources) return;
    const body: AudioSource =
      next === FOLLOW_VALUE
        ? { kind: "follow_default" }
        : (() => {
            const ep = sources.endpoints.find((e) => e.id === next);
            return {
              kind: "endpoint" as const,
              id: next,
              flow: ep?.flow ?? "render",
              name_hint: ep?.name ?? next,
            };
          })();
    setErr(null);
    try {
      const resolved = await api.putAudioSource(body);
      setSources((s) => (s ? { ...s, current: resolved } : s));
      const fresh = await api.audioSources();
      setSources(fresh);
    } catch (e) {
      setErr(messageOf(e));
    }
  };

  const readings = (
    <div ref={readingsRef} data-testid="audio-readings">
      <AudioReadings state={state} />
    </div>
  );

  const select = (
    <span className="audio-picker" ref={triggerRef}>
      <Select
        label="Audio source"
        value={value}
        options={flatOptions}
        groups={groups}
        onChange={(v) => void pick(v)}
        loading={!sources && !err}
        disabled={!sources && !!err}
      />
    </span>
  );

  const words = audioStatusWords(state);
  const waiting = state?.status === "waiting";
  // S3 in the compact form: with an input picked and Listen off, nothing is live
  // yet, so the state slot carries the disclosure (short, one line, the full
  // sentence in `title`); with Listen on it carries the live state. The readings
  // popover always states the full sentence. There is no room for another line
  // at 1024x768, where Settings fits the window exactly.
  const compactNote = inputSelected && !listen;
  const compactWords = compactNote ? EFFECT_NOTE_SHORT : words;
  const compactTitle = compactNote ? EFFECT_NOTE : words;

  return (
    <Panel
      title="Audio source"
      className={usePopover ? "audio-panel audio-compact" : "audio-panel"}
      actions={usePopover ? <span className="audio-picker-head">{select}</span> : <SpeakerHigh size={16} aria-hidden="true" />}
    >
      {!usePopover && (
        <Field label="Source" hint={inputSelected ? EFFECT_NOTE : undefined}>
          <span className="audio-picker-row">
            {select}
            {inputSelected && (
              <Toggle label="Listen to this input" checked={listen} onChange={setListen} />
            )}
          </span>
        </Field>
      )}
      {usePopover ? (
        <>
          <span className="audio-compact-row">
            <span
              className="audio-state audio-state-compact"
              role="status"
              title={compactTitle}
              data-note={compactNote ? "true" : undefined}
            >
              <StateWords words={compactWords} waiting={waiting && !compactNote} />
            </span>
            {inputSelected && (
              <Toggle label="Listen to this input" checked={listen} onChange={setListen} />
            )}
            <Button
              variant="ghost"
              size="sm"
              className="audio-readings-button"
              onClick={() => setPopoverOpen((o) => !o)}
              aria-expanded={popoverOpen}
            >
              {popoverOpen ? "Hide readings" : "Show readings"}
            </Button>
          </span>
          {err && <p className="error-line">{err}</p>}
          <Popover
            anchor={triggerRef as unknown as RefObject<HTMLElement | null>}
            open={popoverOpen}
            onClose={() => setPopoverOpen(false)}
            label="Audio readings"
            className="audio-popover"
          >
            <div ref={popoverFocusRef} tabIndex={-1}>
              {readings}
              {inputSelected && <p className="audio-state">{EFFECT_NOTE}</p>}
            </div>
          </Popover>
        </>
      ) : (
        <>
          <p className="audio-state" role="status">
            <StateWords words={words} waiting={waiting} />
          </p>
          {err && <p className="error-line">{err}</p>}
          {readings}
        </>
      )}
    </Panel>
  );
}
