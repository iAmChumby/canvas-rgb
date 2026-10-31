"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { CheckCircle, CircleDashed, MagnifyingGlass, Pause, Play, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Button, IconButton, Swap } from "@/components/Button";
import { Crossfade } from "@/components/Crossfade";
import { useMediaQuery } from "@/components/MediaQuery";
import { BrandMark, Wordmark } from "@/components/BrandMark";
import { Popover } from "@/components/Popover";
import { useToast } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { links, playing, problems, type Tone } from "@/lib/status";
import { patchSnap, refreshStatus, useSnap } from "@/lib/store";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { RunningPresetName, useTrackRunningPreset } from "@/shell/RunningPresetName";

/** What the strip says when nothing is wrong. */
const ALL_GOOD = "All connected";

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
 * of that kind of fault, and it stays there until the fault clears. A link that is still
 * coming up is named, so the reader knows which one it is.
 *
 * The precise numbers - the measured frame rate, each link's state - are one click away, in
 * the panel that hangs under the mark.
 *
 * The row is the same height and the same width in every state: the mark's text grows into
 * the space beside it and the controls stay where they are, so the page below the strip never
 * moves when the daemon answers or something breaks.
 */
export function StatusStrip({
  onOpenPalette,
  detailsOpen,
  onDetailsOpen,
}: {
  onOpenPalette?: () => void;
  /** Whether the details panel is open. The command palette opens it too, so the state lives
   *  in the shell. */
  detailsOpen: boolean;
  onDetailsOpen: (open: boolean) => void;
}) {
  const status = useSnap((s) => s.status);
  const statusError = useSnap((s) => s.statusError);
  const effects = useSnap((s) => s.effects);
  const [busy, setBusy] = useState(false);
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

  // What is playing.
  const now = playing(status, effects);
  useTrackRunningPreset(status?.active_effect ?? null);
  const list = problems(status, offline);
  const tone: Tone = list[0]?.tone ?? "ok";
  // On a phone the row has room for the link's name, not for the whole sentence. The full
  // words are the button's own label, so nothing is lost.
  const phone = useMediaQuery("(max-width: 720px)");
  const fullText = list[0]?.text ?? ALL_GOOD;
  const markText = list[0] ? (phone ? (list[0].short ?? list[0].text) : list[0].text) : ALL_GOOD;
  // Each link and what it is doing: the panel shows them all, one after another.
  const linkList = links(status, offline);

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
          <BrandMark size={16} />
          <Wordmark />
        </span>

        {/* What the lights are doing, with a chip of the running light's own colour. */}
        <span className="strip-now">
          <span className="strip-swatch" data-paused={paused ? "true" : "false"} aria-hidden="true" />
          <span className="strip-now-text">
            <span className="strip-effect-name">
              {now.name}
              <RunningPresetName effectId={status?.active_effect ?? null} />
            </span>
            {now.detail && (
              <span className="strip-now-detail" data-count={now.count ? "true" : "false"}>
                {now.detail}
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
          aria-label={fullText}
          title={fullText}
          aria-expanded={detailsOpen}
          onClick={() => onDetailsOpen(!detailsOpen)}
        >
          <Crossfade swapKey={`${tone}:${fullText}`}>
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
              <span className="strip-power-text">
                <Swap labels={["Pause lights", "Resume lights"]} at={paused ? 1 : 0} />
              </span>
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

      <Popover
        anchor={markRef}
        open={detailsOpen}
        onClose={() => onDetailsOpen(false)}
        label="Status details"
        className="strip-details"
      >
        <div className="strip-details-head">Details</div>
        <dl className="strip-details-list">
          <Detail label="Frame rate" value={status ? <><AnimatedNumber value={status.fps_measured} decimals={1} /> / <AnimatedNumber value={status.fps_target} /> fps</> : "—"} />
          <Detail label="Frame time" value={status ? <><AnimatedNumber value={status.frame_time_ms} decimals={1} /> ms</> : "—"} />
          <Detail label="Lights" value={status ? (paused ? "paused" : status.parked ? "idle" : "running") : "—"} />
          <Detail label="Devices" value={status ? <><AnimatedNumber value={status.devices_connected} /> connected</> : "—"} />
        </dl>
        {/* Each link and what it is doing, so "connecting" is never a mystery. */}
        <ul className="strip-details-links">
          {linkList.map((l) => (
            <li key={l.name} data-tone={l.tone}>
              <span className="strip-details-link-name">{l.name}</span>
              <span className="fig">{l.state}</span>
            </li>
          ))}
        </ul>
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

function Detail({ label, value }: { label: string; value: ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd className="fig">{value}</dd>
    </>
  );
}
