"use client";

import { useEffect, useRef, useState } from "react";
import { CheckCircle, CircleDashed, MagnifyingGlass, Pause, Play, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Crossfade } from "@/components/Crossfade";
import { Popover } from "@/components/Popover";
import { useToast } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { patchSnap, refreshStatus, useSnap } from "@/lib/store";
import type { Status } from "@/proto/Status";

/** What the strip says when nothing is wrong. */
const ALL_GOOD = "All connected";

type Tone = "ok" | "warn" | "down" | "work";

interface Problem {
  tone: Tone;
  text: string;
}

/**
 * What is not working, worst first. An empty list means everything is fine.
 *
 * A link that is still connecting is not a fault, but it is not "all connected" either, so it
 * is said in words too - in the link's own colour. The words are short on purpose: the strip
 * has one line, and a phone has one line's worth of room.
 */
function problems(status: Status | null, offline: boolean): Problem[] {
  const out: Problem[] = [];
  if (offline) {
    out.push({ tone: "down", text: "Daemon offline" });
    return out;
  }
  if (!status) {
    out.push({ tone: "work", text: "Connecting…" });
    return out;
  }
  if (status.openrgb === "down") out.push({ tone: "down", text: "OpenRGB down" });
  if (status.service === "down") out.push({ tone: "down", text: "Fan control off" });
  // The lights are running, but not at the rate they were asked for.
  if (status.power === "running" && !status.parked && status.fps_measured < status.fps_target - 1) {
    out.push({ tone: "warn", text: "Running slow" });
  }
  if (out.length === 0 && (status.openrgb === "connecting" || status.service === "connecting")) {
    out.push({ tone: "work", text: "Connecting…" });
  }
  return out;
}

/** The icon of the mark: a tick when all is well, a warning when it is not. */
function markIcon(tone: Tone) {
  if (tone === "down") return <WarningOctagon size={14} weight="fill" />;
  if (tone === "warn") return <Warning size={14} weight="fill" />;
  if (tone === "work") return <CircleDashed size={14} />;
  return <CheckCircle size={14} weight="fill" />;
}

/**
 * The strip: what the lights are doing.
 *
 * One row, in plain words: what is playing (with a chip of the running light's own colour),
 * how it is going, and the control. When everything is well the state collapses into one
 * quiet "all connected" mark; when something is not, the mark says what it is, in the colour
 * of that kind of fault, and it stays there until the fault clears.
 *
 * The precise numbers - the measured frame rate, each link's state - are one click away, in
 * the panel that hangs under the mark.
 *
 * The row is the same height and the same width in every state: the mark's text grows into
 * the space beside it and the controls stay where they are, so the page below the strip never
 * moves when the daemon answers or something breaks.
 */
export function StatusStrip({ onOpenPalette }: { onOpenPalette?: () => void }) {
  const status = useSnap((s) => s.status);
  const statusError = useSnap((s) => s.statusError);
  const effects = useSnap((s) => s.effects);
  const [busy, setBusy] = useState(false);
  const [details, setDetails] = useState(false);
  const toast = useToast();
  const stripRef = useRef<HTMLDivElement>(null);
  const markRef = useRef<HTMLButtonElement>(null);

  // The height of the strip, published for the overlays that sit under it.
  useEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    const publish = () => {
      document.documentElement.style.setProperty("--strip-h", `${el.offsetHeight}px`);
    };
    publish();
    const ro = new ResizeObserver(publish);
    ro.observe(el);
    return () => {
      ro.disconnect();
      document.documentElement.style.removeProperty("--strip-h");
    };
  }, []);

  const known = status !== null;
  const offline = known === false && statusError !== null;
  const paused = status ? status.power === "paused" : false;
  const effectName = status
    ? (effects?.find((e) => e.id === status.active_effect)?.name ?? status.active_effect ?? "")
    : "";

  // What is playing.
  const playing = (() => {
    if (!status) return { name: "—", detail: "", count: false };
    // Without a layout there is nowhere for the lights to go.
    if (!status.active_layout) return { name: "No layout yet", detail: "", count: false };
    if (!status.active_effect || !effectName) return { name: "Nothing is running", detail: "", count: false };
    if (paused) return { name: effectName, detail: "paused", count: false };
    if (status.parked) return { name: effectName, detail: "idle", count: false };
    const n = status.devices_connected;
    return { name: effectName, detail: `on ${n} ${n === 1 ? "device" : "devices"}`, count: true };
  })();

  const list = problems(status, offline);
  const tone: Tone = list[0]?.tone ?? "ok";
  const markText = list[0]?.text ?? ALL_GOOD;

  const togglePower = async () => {
    if (!status) return;
    setBusy(true);
    try {
      await api.patchOutput({ power: paused ? "running" : "paused", brightness: null });
      patchSnap({ status: { ...status, power: paused ? "running" : "paused" } });
      await refreshStatus();
      toast(paused ? "Lights resumed" : "Lights paused");
    } catch (e) {
      toast(messageOf(e), "down");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="strip" role="status" aria-label="Status" ref={stripRef}>
      <div className="strip-inner">
        <span className="strip-brand">
          <span className="rail-brand-dot" aria-hidden="true" />
          canvas-rgb
        </span>

        {/* What the lights are doing, with a chip of the running light's own colour. */}
        <span className="strip-now">
          <span className="strip-swatch" data-paused={paused ? "true" : "false"} aria-hidden="true" />
          <span className="strip-now-text">
            <span className="strip-effect-name">{playing.name}</span>
            {playing.detail && (
              <span className="strip-now-detail" data-count={playing.count ? "true" : "false"}>
                {playing.detail}
              </span>
            )}
          </span>
        </span>

        {/* How it is going: one quiet mark when all is well, words when it is not. */}
        <button
          ref={markRef}
          type="button"
          className="strip-mark"
          data-tone={tone}
          aria-label={markText}
          aria-expanded={details}
          onClick={() => setDetails((v) => !v)}
        >
          <Crossfade swapKey={`${tone}:${markText}`}>
            <span className="strip-mark-icon" aria-hidden="true">
              {markIcon(tone)}
            </span>
            <span className="strip-mark-text">{markText}</span>
          </Crossfade>
        </button>

        <span className="strip-controls">
          {known ? (
            <Button
              size="sm"
              variant={paused ? "primary" : "default"}
              disabled={busy}
              onClick={togglePower}
              aria-label={paused ? "Resume lights" : "Pause lights"}
              icon={paused ? <Play size={14} weight="fill" /> : <Pause size={14} weight="fill" />}
            >
              {/* On a phone the row has no room for the words: the button's own label says
                  what it does. */}
              <span className="strip-power-text">{paused ? "Resume lights" : "Pause lights"}</span>
            </Button>
          ) : null}
          {onOpenPalette && (
            <IconButton
              label="Open the command palette"
              size="sm"
              icon={<MagnifyingGlass size={16} />}
              onClick={onOpenPalette}
            />
          )}
        </span>
      </div>

      <Popover anchor={markRef} open={details} onClose={() => setDetails(false)} label="Status details" className="strip-details">
        <div className="strip-details-head">Details</div>
        <dl className="strip-details-list">
          <Detail label="Frame rate" value={status ? `${status.fps_measured.toFixed(1)} / ${status.fps_target} fps` : "—"} />
          <Detail label="Frame time" value={status ? `${status.frame_time_ms.toFixed(1)} ms` : "—"} />
          <Detail label="Lights" value={status ? (paused ? "paused" : status.parked ? "idle" : "running") : "—"} />
          <Detail label="OpenRGB" value={status ? status.openrgb : "—"} />
          <Detail label="Fan service" value={status ? status.service : "—"} />
          <Detail label="Devices" value={status ? `${status.devices_connected} connected` : "—"} />
        </dl>
        {list.length > 1 && (
          <ul className="strip-details-problems">
            {list.slice(1).map((p) => (
              <li key={p.text} data-tone={p.tone}>
                {p.text}
              </li>
            ))}
          </ul>
        )}
      </Popover>
    </div>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt>{label}</dt>
      <dd className="fig">{value}</dd>
    </>
  );
}
