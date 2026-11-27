"use client";

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CheckCircle, CircleDashed, MagnifyingGlass, Pause, Play, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Button, IconButton, Swap } from "@/components/Button";
import { IconSwap } from "@/components/IconSwap";
import { Crossfade } from "@/components/Crossfade";
import { useMediaQuery } from "@/components/MediaQuery";
import { PHONE_QUERY } from "@/lib/MediaQuery";
import { BrandMark, Wordmark } from "@/components/BrandMark";
import { Popover } from "@/components/Popover";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { links, playing, problems, type SceneFaults, type Tone } from "@/lib/status";
import { patchSnap, refreshStatus, useSnap } from "@/lib/store";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { RunningPresetName, useTrackRunningPreset } from "@/shell/RunningPresetName";
import { StripSlot } from "@/shell/PageHeader";

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
  const statusFault = useSnap((s) => s.statusFault);
  const effects = useSnap((s) => s.effects);
  const scenesAnswer = useSnap((s) => s.scenes);
  const scenesError = useSnap((s) => s.scenesError);
  const [busy, setBusy] = useState(false);
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
  // The strip names the scene, so no one effect's preset belongs after it.
  const presetId: string | null = null;
  useTrackRunningPreset(presetId);
  // A group that cannot light and a scene list that could not be read are said in the mark, at
  // every width, so the row never claims "All connected" over a dark group.
  const faults = useMemo<SceneFaults>(
    () => ({
      dark: (scenesAnswer?.group_states ?? []).filter((g) => g.failed !== null).length,
      unread: scenesAnswer === null && scenesError !== null,
    }),
    [scenesAnswer, scenesError],
  );
  const list = problems(status, statusFault, faults);
  const tone: Tone = list[0]?.tone ?? "ok";
  // On a phone the row has room for the link's name, not for the whole sentence. The full
  // words are the button's own label, so nothing is lost.
  const phone = useMediaQuery(PHONE_QUERY);
  // #580 R2, R14, C1, C2: the mark counts every note, dismissed or not, and says so when the
  // counted notes are all hidden. The count does not swallow the first problem: the popover shows
  // the whole list whenever the mark shows a count.
  const faultEntries = list.filter((p) => p.fault);
  const warnings = faultEntries.length;
  const allDismissed = warnings > 0 && faultEntries.every((p) => p.dismissed);
  const countText = `${warnings} warning${warnings === 1 ? "" : "s"}${allDismissed ? ", dismissed" : ""}`;
  const markText = warnings > 0
    ? (phone ? String(warnings) : countText)
    : list[0]
      ? (phone ? (list[0].short ?? list[0].text) : list[0].text)
      : ALL_GOOD;
  const fullText = warnings > 0 ? markText : (list[0]?.text ?? ALL_GOOD);
  // The swatch's own name, for the widths where the effect's name is dropped from the row.
  // Before the daemon has answered there is nothing to name: the row says why, never a bare dash.
  // The daemon that is up with its engine not answering (#241) is not offline: its engine is.
  const nowName = status
    ? now.name
    : statusFault === "engine"
      ? "Engine down"
      : offline
        ? "Offline"
        : "Connecting";
  const swatchLabel = nowName === "—" ? "Nothing is running" : nowName;
  // Each link and what it is doing: the panel shows them all, one after another.
  const linkList = links(status, statusFault);
  // With a count the mark is a summary, so the panel has to carry every problem including the
  // first; without one the mark already says the first problem and the panel adds the rest.
  const shownProblems = warnings > 0 ? list : list.slice(1);

  const showAgain = async (key: string) => {
    try {
      await api.showFaultAgain(key);
      await refreshStatus();
    } catch (e) {
      notify(messageOf(e), "warn");
    }
  };

  const togglePower = async () => {
    if (!status) return;
    setBusy(true);
    try {
      await api.patchOutput({ power: paused ? "running" : "paused", brightness: null, warmth: null });
      patchSnap({ status: { ...status, power: paused ? "running" : "paused" } });
      await refreshStatus();
    } catch (e) {
      notify(messageOf(e), "down");
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

        {/* The page's own title and context, filled in by the page (R2). */}
        <StripSlot kind="title" />

        {/* What the lights are doing, with a chip of the running light's own colour. */}
        <span className="strip-now">
          {/* The swatch is the only part of "now playing" left below 1280, where the effect's
              name is dropped from the row: its label carries the name to a screen reader. */}
          <span
            className="strip-swatch"
            data-paused={paused ? "true" : "false"}
            data-idle={known ? "false" : "true"}
            role="img"
            aria-label={swatchLabel}
            title={swatchLabel}
          />
          <span className="strip-now-text">
            <span className="strip-effect-name">
              {nowName}
              <RunningPresetName effectId={presetId} />
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
            <span className="strip-mark-icon" data-count={warnings > 0 ? String(warnings) : undefined} aria-hidden="true">
              {markIcon(tone)}
            </span>
            <span className="strip-mark-text" data-muted={allDismissed ? "true" : undefined}>{markText}</span>
          </Crossfade>
        </button>

        <span className="strip-controls">
          {/* The page's own actions, before Pause and Search (R2, R4). */}
          <StripSlot kind="actions" />
          {known ? (
            <Button
              size="sm"
              variant={paused ? "primary" : "default"}
              busy={busy}
              onClick={togglePower}
              aria-label={paused ? "Resume lights" : "Pause lights"}
              // The Play/Pause swap cross-fades with scale and blur (issue #187, rule 8). No
              // label here: the button's own aria-label already says it, so the swap
              // carries no duplicate role="img" landmark.
              icon={
                <IconSwap
                  a={<Pause size={14} weight="fill" aria-hidden="true" />}
                  b={<Play size={14} weight="fill" aria-hidden="true" />}
                  at={paused ? 1 : 0}
                />
              }
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
        {shownProblems.length > 0 && (
          <ul className="strip-details-problems">
            {shownProblems.map((p) => (
              <li key={`${p.text}:${p.key ?? ""}`} data-tone={p.tone} data-dismissed={p.dismissed ? "true" : undefined}>
                <span>{p.text}</span>
                {p.dismissed && p.key && (
                  <Button size="sm" variant="ghost" onClick={() => void showAgain(p.key as string)}>
                    Show again
                  </Button>
                )}
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
