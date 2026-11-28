"use client";

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CheckCircle, CircleDashed, MagnifyingGlass, Pause, Play, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Button, IconButton, Swap } from "@/components/Button";
import { IconSwap } from "@/components/IconSwap";
import { Crossfade } from "@/components/Crossfade";
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
import { fitBar } from "@/shell/barFit";

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
  const rowRef = useRef<HTMLDivElement>(null);
  const markRef = useRef<HTMLButtonElement>(null);

  // One rule, measured from the strip's own content box (#586 fix pass 1 B9, B10, B12, B13):
  //  - the context and the secondary keys stay in the bar while they fit; otherwise the page draws
  //    the context at the head of its body and the keys in the overflow (CSS shows one copy);
  //  - a second row appears only when a primary action cannot fit beside the other controls;
  //  - now playing lives in the strip only while the rail's deck is collapsed, never twice; a name
  //    the row cannot hold is said at the head of the page body instead.
  // All of it is decided from the strip's box, never from a viewport guess.
  useEffect(() => {
    const strip = stripRef.current;
    const row = rowRef.current;
    if (!strip || !row) return;
    const publishHeight = () => {
      document.documentElement.style.removeProperty("--strip-h");
      document.documentElement.style.setProperty("--strip-h", `${strip.offsetHeight}px`);
    };
    const update = () => {
      const root = document.documentElement;
      const deck = document.querySelector<HTMLElement>(".rail-deck");
      root.dataset.nowWhere = deck && getComputedStyle(deck).display !== "none" ? "rail" : "strip";
      // `data-measure` stops everything shrinking, so what is measured is what each part needs.
      strip.dataset.measure = "";
      const fit = fitBar(strip, row);
      delete strip.dataset.measure;
      root.dataset.barContext = fit.context;
      root.dataset.barSecondary = fit.secondary;
      strip.dataset.barRows = fit.rows;
      strip.dataset.barNow = fit.now;
      // B11: one place says the name. The page's copies read this and leave it out.
      if (root.dataset.nowWhere === "strip" && fit.now === "gone") root.dataset.nowWhere = "body";
      // B8: the status words and the now-playing detail step down by the strip's own container
      // rules. Whenever one of them is not drawn, the page body gets its home.
      const gone = (selector: string) => {
        const el = row.querySelector(selector);
        return el !== null && getComputedStyle(el).display === "none";
      };
      const detailGone = root.dataset.nowWhere === "strip" && gone(".strip-now-detail");
      const nameGone = root.dataset.nowWhere === "body";
      root.dataset.barStatus = gone(".strip-mark-text") || detailGone || nameGone ? "body" : "bar";
      publishHeight();
      row.dispatchEvent(new Event("barfit"));
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(row);
    ro.observe(strip);
    const mo = new MutationObserver(update);
    mo.observe(strip, { childList: true, subtree: true, characterData: true });
    // The faces arrive after the first measurement and change every width.
    let gone = false;
    void document.fonts?.ready.then(() => {
      if (!gone) update();
    });
    return () => {
      gone = true;
      ro.disconnect();
      mo.disconnect();
      // A window with no strip (the onboarding) has nothing that moved out of it.
      for (const key of ["barContext", "barSecondary", "barStatus", "nowWhere"]) delete document.documentElement.dataset[key];
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
  // #580 R2, R14, C1, C2: the mark counts every note, dismissed or not, and says so when the
  // counted notes are all hidden. The count does not swallow the first problem: the popover shows
  // the whole list whenever the mark shows a count.
  const faultEntries = list.filter((p) => p.fault);
  const warnings = faultEntries.length;
  const allDismissed = warnings > 0 && faultEntries.every((p) => p.dismissed);
  const countText = `${warnings} warning${warnings === 1 ? "" : "s"}${allDismissed ? ", dismissed" : ""}`;
  // The words are always the full sentence; the container rules drop them when the strip is too
  // narrow, and the button's own label still carries them (#586 R4).
  const markText = warnings > 0 ? countText : (list[0]?.text ?? ALL_GOOD);
  // The accessible name is always the full sentence, including ", dismissed" (fix pass 2): the
  // phone's visible number is short, but the button must still say what the count means.
  const fullText = warnings > 0 ? countText : (list[0]?.text ?? ALL_GOOD);
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
        <div className="strip-row" ref={rowRef}>
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
          data-dismissed={allDismissed ? "true" : undefined}
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

        {/* The page's keys own their slot: on a phone only a primary action that cannot fit in
            the first row moves to the second. Pause and search stay in the same place on every
            page (B12). */}
        <span className="strip-page-actions">
          <StripSlot kind="actions" />
        </span>

        <span className="strip-controls">
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
          {/* R10: the now-playing detail leaves the row before its name does, so the popover
              keeps the whole sentence one click from the mark. */}
          <Detail label="Now playing" value={now.detail ? `${nowName} ${now.detail}` : nowName} />
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
