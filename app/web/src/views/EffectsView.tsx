"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import {
  CaretLeft,
  FileHtml,
  MagnifyingGlass,
  MonitorPlay,
  Play,
  SquaresFour,
  Star,
  Trash,
  UploadSimple,
  Waveform,
} from "@phosphor-icons/react";
import { Button, ButtonLink, FileButton, IconButton, Swap } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Thumb } from "@/components/Thumb";
import { Panel } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { StateDot } from "@/components/Panel";
import { EffectPreview } from "@/components/EffectPreview";
import { EmptyState } from "@/components/EmptyState";
import { Enter } from "@/components/Motion";
import { Highlighted, matchEffect } from "@/components/Search";
import { useMediaQuery } from "@/components/MediaQuery";
import { ErrorPanel, Details, LoadFrame, type LoadState } from "@/components/Loading";
import { EffectDetailShape, EffectGalleryShape } from "@/components/LoadShapes";
import { useToast } from "@/components/Toast";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { PresetStrip } from "@/views/PresetStrip";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { EFFECT_CATEGORIES, categoryOf, type EffectCategory } from "@/lib/effectCategories";
import { getSnap, refreshAll, retryEffects, useSnap } from "@/lib/store";
import { fillDefaults, nextFreeName } from "@/lib/presetMatch";
import { appliedName, runningOf, setApplied, setShown } from "@/lib/runningPreset";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { PropValue } from "@/proto/PropValue";
import { AnimatedNumber } from "@/components/AnimatedNumber";

/** Live edits are sent this long after the last change (ms). */
const PATCH_DEBOUNCE_MS = 60;

/** The height the shell published for the status strip. */
function currentStripHeight(): string {
  if (typeof document === "undefined") return "0px";
  const value = getComputedStyle(document.documentElement).getPropertyValue("--strip-h").trim();
  return value || "0px";
}

export function EffectsView() {
  const effects = useSnap((s) => s.effects);
  const effectsError = useSnap((s) => s.effectsError);
  const failed = useSnap((s) => s.effectsFailed);
  const status = useSnap((s) => s.status);
  const activeId = useSnap((s) => s.status?.active_effect ?? null);
  // A load that is in flight after a Try again: the frame shows its placeholder until it ends.
  const retrying = useSnap((s) => s.pending.includes("effects"));
  const [filter, setFilter] = useState<"all" | "favorites">("all");
  const [category, setCategory] = useState<EffectCategory | null>(null);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  // The running effect is the default; a click on the list overrides it.
  const selectedId = picked ?? activeId ?? effects?.[0]?.id ?? null;
  const setSelectedId = setPicked;
  const [loaded, setLoaded] = useState<{ id: string; detail: EffectDetail } | null>(null);
  const [values, setValues] = useState<Values>({});
  const [error, setError] = useState<string | null>(null);
  // Asks the detail to load again for the same effect: a preset was applied or changed, so the
  // sliders and the preset list are read back from the daemon.
  const [reloadTick, setReloadTick] = useState(0);
  const [favPop, setFavPop] = useState<string | null>(null);
  // The import: what is being sent, what the daemon said, and the file that is waiting for a
  // replace.
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [replaceWith, setReplaceWith] = useState<{ fileName: string; html: string } | null>(null);
  // A file is over the page: the drop overlay shows.
  const [dragging, setDragging] = useState(false);
  // The user effect that is about to be deleted.
  const [deleting, setDeleting] = useState<string | null>(null);
  // The failed files' list, folded away until it is asked for.
  const [showFailed, setShowFailed] = useState(false);
  // On a phone the detail is pushed over the gallery.
  const narrow = useMediaQuery("(max-width: 720px)");
  const [pushed, setPushed] = useState(false);
  const [closing, setClosing] = useState(false);
  // Set when the push has settled: the gallery is then out of the way and out of the tab order.
  const [settled, setSettled] = useState(false);
  // The height of the status strip when the sheet opened.
  const [sheetTop, setSheetTop] = useState<string | null>(null);
  const toast = useToast();

  const pending = useRef<Values>({});
  /** The effect the waiting values were edited on. */
  const pendingFor = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const selectedRef = useRef<string | null>(null);
  const valuesRef = useRef<Values>({});
  // The values the daemon has: a failed edit goes back to these.
  const saved = useRef<Values>({});
  useEffect(() => {
    selectedRef.current = selectedId;
  }, [selectedId]);
  useEffect(() => {
    valuesRef.current = values;
  }, [values]);

  /** Sends the waiting values to the effect they were edited on, and to no other. */
  const flush = async () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    const id = pendingFor.current;
    const batch = pending.current;
    pending.current = {};
    pendingFor.current = null;
    if (!id || Object.keys(batch).length === 0) return;
    try {
      if (getSnap().status?.active_effect === id) {
        await api.patchActiveProps(batch);
      } else {
        // The first edit to a stopped effect starts it, with the edited values on top.
        await api.activateEffect(id, { props: batch, preset: null });
        await refreshAll();
      }
      if (selectedRef.current === id) saved.current = { ...saved.current, ...batch };
      setError(null);
    } catch (e) {
      setError(messageOf(e));
      toast(messageOf(e), "down");
      // The control goes back to the value the daemon has: nothing fails silently.
      if (selectedRef.current === id) setValues(saved.current);
    }
  };

  // Load the detail (schema and current values) for the selected effect.
  useEffect(() => {
    if (!selectedId) return;
    let stop = false;
    api
      .effect(selectedId)
      .then((d) => {
        if (stop) return;
        setLoaded({ id: selectedId, detail: d });
        setValues(d.props);
        saved.current = d.props;
        setError(null);
      })
      .catch((e: unknown) => {
        if (!stop) setError(messageOf(e));
      });
    return () => {
      stop = true;
      // A drag that is still waiting belongs to the effect it was made on: it goes there now,
      // before the next effect's values load.
      void flush();
    };
    // flush reads refs and stable setters only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, activeId, reloadTick]);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const onChange = (key: string, value: PropValue) => {
    const id = selectedRef.current;
    // Values waiting for another effect go to it first; they never mix into this one's batch.
    if (pendingFor.current && pendingFor.current !== id) void flush();
    pendingFor.current = id;
    setValues((prev) => ({ ...prev, [key]: value }));
    pending.current = { ...pending.current, [key]: value };
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void flush(), PATCH_DEBOUNCE_MS);
  };

  const activate = async (preset: string | null) => {
    if (!selectedId) return;
    setError(null);
    try {
      await api.activateEffect(selectedId, { props: {}, preset });
      await refreshAll();
      toast(preset ? `${detail?.schema.name ?? "Effect"}: ${preset}` : `${detail?.schema.name ?? "Effect"} is running`);
    } catch (e) {
      setError(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  // The preset deleted last, while it can still be put back (8 s).
  const [undo, setUndoState] = useState<{ name: string; run: () => Promise<void> } | null>(null);
  const undoTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const setUndo = (next: { name: string; run: () => Promise<void> } | null) => {
    if (undoTimer.current) clearTimeout(undoTimer.current);
    setUndoState(next);
    if (next) undoTimer.current = setTimeout(() => setUndoState(null), 8000);
  };
  const detailRef = useRef<EffectDetail | null>(null);
  useEffect(() => {
    detailRef.current = loaded && loaded.id === selectedId ? loaded.detail : null;
  });

  /** Puts the daemon's answer to a preset change into the loaded detail. */
  const setUserPresets = (id: string, user_presets: EffectDetail["user_presets"]) => {
    setLoaded((l) => (l && l.id === id ? { ...l, detail: { ...l.detail, user_presets } } : l));
  };

  /** A preset another window removed or changed: say so and read the list again. */
  const gone = (message: string) => {
    toast(message, "warn");
    setReloadTick((t) => t + 1);
  };

  const applyPreset = async (name: string) => {
    const d = detailRef.current;
    if (!d) return;
    try {
      await api.activateEffect(d.schema.id, { props: {}, preset: name });
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 404) return gone("That preset was removed in another window");
      throw e;
    }
    setApplied(d.schema.id, name);
    await refreshAll();
    setReloadTick((t) => t + 1);
    toast(`${d.schema.name}: ${name}`);
  };

  const savePreset = async (name: string) => {
    const d = detailRef.current;
    if (!d) return;
    const r = await api.createPreset(d.schema.id, {
      name,
      props: fillDefaults(d.schema.properties, valuesRef.current),
    });
    setUserPresets(d.schema.id, r.user_presets);
    setApplied(d.schema.id, name);
    toast(`Saved ${name}`);
  };

  const updatePreset = async (name: string) => {
    const d = detailRef.current;
    const preset = d?.user_presets.find((p) => p.name === name);
    if (!d || !preset) return;
    try {
      const r = await api.replacePreset(d.schema.id, name, {
        rename_to: null,
        props: fillDefaults(d.schema.properties, valuesRef.current),
        expected: preset.props,
      });
      setUserPresets(d.schema.id, r.user_presets);
      setApplied(d.schema.id, name);
      toast(`Updated ${name}`);
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 404) return gone("That preset was removed in another window");
      if (e instanceof ApiFailure && e.status === 409) return gone("Changed in another window");
      throw e;
    }
  };

  const renamePreset = async (name: string, to: string) => {
    const d = detailRef.current;
    const preset = d?.user_presets.find((p) => p.name === name);
    if (!d || !preset) return;
    try {
      const r = await api.replacePreset(d.schema.id, name, {
        rename_to: to,
        props: preset.props,
        expected: preset.props,
      });
      setUserPresets(d.schema.id, r.user_presets);
      if (appliedName(d.schema.id) === name) setApplied(d.schema.id, to);
      toast(`Renamed to ${to}`);
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 404) return gone("That preset was removed in another window");
      throw e;
    }
  };

  const duplicatePreset = async (name: string) => {
    const d = detailRef.current;
    const preset = d?.user_presets.find((p) => p.name === name);
    if (!d || !preset) return;
    const copy = nextFreeName(name, [d.schema.presets, d.user_presets]);
    const r = await api.createPreset(d.schema.id, { name: copy, props: preset.props });
    setUserPresets(d.schema.id, r.user_presets);
    toast(`Duplicated as ${copy}`);
  };

  const deletePreset = async (name: string) => {
    const d = detailRef.current;
    const preset = d?.user_presets.find((p) => p.name === name);
    if (!d || !preset) return;
    const id = d.schema.id;
    const wasApplied = appliedName(id) === name;
    try {
      const r = await api.deletePreset(id, name);
      setUserPresets(id, r.user_presets);
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 404) return gone("That preset was removed in another window");
      throw e;
    }
    // The live values stay as they are; they just no longer belong to a preset.
    if (wasApplied) setApplied(id, null);
    // One way back, shared by the toast and the strip: it works once, then both go.
    let used = false;
    const putBack = async () => {
      if (used) return;
      used = true;
      setUndo(null);
      try {
        const r = await api.createPreset(id, { name, props: preset.props });
        setUserPresets(id, r.user_presets);
        if (wasApplied) setApplied(id, name);
        // The store appends, so the preset comes back at the end of the list.
        toast(`Put back ${name} at the end of the list`);
      } catch (e) {
        used = false;
        throw e;
      }
    };
    setUndo({ name, run: putBack });
    toast(`Deleted ${name}`, "info", {
      label: "Undo",
      onClick: () => void putBack().catch((e: unknown) => toast(messageOf(e), "down")),
    });
  };

  const toggleFavorite = async (id: string, favorite: boolean, name: string) => {
    setFavPop(id);
    setTimeout(() => setFavPop(null), 260);
    try {
      await api.setFavorite(id, { favorite });
      await refreshAll();
      toast(favorite ? `${name} added to favourites` : `${name} removed from favourites`, "info");
    } catch (e) {
      setError(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  /** Sends an HTML effect to the daemon. A clash asks for a replace first. */
  const sendImport = async (file: File, replace: boolean) => {
    setImporting(true);
    try {
      const html = await file.text();
      const r = await api.importEffect({ file_name: file.name, html, replace });
      await refreshAll();
      setPicked(r.effect.id);
      setReplaceWith(null);
      setImportError(null);
      toast(r.replaced ? `${r.effect.name} is replaced` : `${r.effect.name} is added`);
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 409) {
        setReplaceWith({ fileName: file.name, html: await file.text() });
        setImportError(null);
      } else {
        setReplaceWith(null);
        setImportError(messageOf(e));
      }
    } finally {
      setImporting(false);
    }
  };

  /** Removes a user effect. The one that is playing asks first. */
  const deleteEffect = async (id: string, name: string, keepPresets = false) => {
    setDeleting(null);
    try {
      await api.deleteEffect(id, keepPresets);
      await refreshAll();
      toast(`${name} is removed`);
    } catch (e) {
      setError(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  const q = query.trim();
  // The search is loose: the words may come in any order, one letter may be wrong, and a word
  // may match the description. The letters that matched are marked in the name.
  const list = (effects ?? [])
    .map((e, i) => ({ effect: e, match: matchEffect(q, e.name, e.description, i) }))
    .filter(
      (row) =>
        row.match !== null &&
        (filter === "all" || row.effect.favorite) &&
        (category === null || categoryOf(row.effect.id) === category),
    )
    .sort((a, b) => (b.match?.score ?? 0) - (a.match?.score ?? 0));
  // While the next effect's detail is in flight the previous one stays on screen, so the preview
  // is never blank; the Activate button waits for the real one.
  const detail = loaded ? loaded.detail : null;
  const stale = loaded !== null && loaded.id !== selectedId;
  const running = selectedId !== null && selectedId === activeId;
  const needsInput = detail ? detail.schema.needs.screen || detail.schema.needs.audio : false;
  // Nothing can run until there is a layout.
  const noLayout = status !== null && !status.active_layout;

  // Which preset the values on screen mean, and whether they have moved off it. The rail and the
  // strip read it from the same place.
  const runningPreset = useMemo(
    () => (detail && !stale ? runningOf(detail, { ...detail.props, ...values }) : null),
    // runningOf also reads the name applied last, which every handler above changes together
    // with the list or the values
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [detail, stale, values, reloadTick],
  );
  const runningKey = runningPreset ? `${runningPreset.name}|${runningPreset.modified}` : "";
  useEffect(() => {
    if (detail && !stale) {
      // Values that equal a preset are that preset from now on: moving a slider then marks it
      // modified instead of dropping its name.
      if (runningPreset && !runningPreset.modified) setApplied(detail.schema.id, runningPreset.name);
      setShown(detail.schema.id, runningPreset);
    }
    // runningKey stands for runningPreset
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [detail?.schema.id, stale, runningKey]);

  // On a phone the detail becomes its own view: the gallery slides left and the detail comes
  // in from the right, with a back control.
  const pick = (id: string) => {
    setSelectedId(id);
    if (narrow) {
      setPushed(true);
      setSettled(false);
      // The sheet's top edge is fixed when it opens: the strip may change height later.
      setSheetTop(currentStripHeight());
      setTimeout(() => setSettled(true), 240);
    }
  };

  // The back control plays the push in reverse before the sheet goes.
  const close = () => {
    setClosing(true);
    setSettled(false);
    setTimeout(() => {
      setClosing(false);
      setPushed(false);
    }, 240);
  };

  // The files that did not parse. They stand inside the gallery's frame, because they come with
  // the list: above the frame they would push the page down when the list lands.
  const failedStrip =
    failed.length > 0 ? (
      <div className="failed-strip" data-open={showFailed ? "true" : "false"}>
        <button
          type="button"
          className="failed-strip-head"
          aria-expanded={showFailed}
          onClick={() => setShowFailed((v) => !v)}
        >
          <FileHtml size={14} aria-hidden="true" />
          <span>
            {failed.length} file{failed.length === 1 ? "" : "s"} did not parse
          </span>
        </button>
        {showFailed && (
          <div className="failed-strip-body">
            {failed.map((f) => (
              <p key={f.file} className="failed-line">
                <span className="fig">{f.file}</span>
                <span className="failed-error">{f.error}</span>
              </p>
            ))}
          </div>
        )}
      </div>
    ) : null;

  // A refresh that failed after the list was read does not take the gallery away: the cards stay
  // and the page says they may be old. Only a list that was never read has an error frame.
  const listStale = effects !== null && effectsError !== null;
  const galleryState: LoadState =
    effects === null
      ? effectsError && !retrying
        ? "error"
        : "loading"
      : list.length === 0
        ? "empty"
        : "ready";
  const gallery = (
    <LoadFrame
      className="stack"
      state={galleryState}
      shape={<EffectGalleryShape />}
      label="Loading the effects"
      error={
        <ErrorPanel
          title="The effects could not be read"
          summary="canvas-rgb could not read the effect list. Nothing was changed."
          message={effectsError}
          onRetry={() => void retryEffects()}
        />
      }
      empty={
        <div className="stack">
          <Panel className="effect-empty">
            {effects !== null && effects.length === 0 ? (
              // There is no effect at all: the filter is not the reason.
              <EmptyState icon={<Waveform size={22} />} title="No effects yet">
                This machine has no effect to run. Import an HTML effect to add one.
              </EmptyState>
            ) : (
              <EmptyState icon={<MagnifyingGlass size={22} />} title="No effect matches that">
                Try fewer words, or clear the filter and the category.
              </EmptyState>
            )}
          </Panel>
          {failedStrip}
        </div>
      }
    >
    <div className="effect-gallery">
      {list.map((row, i) => {
        const e = row.effect;
        const html = e.id.startsWith("html:");
        const selected = e.id === selectedId;
        return (
          <Enter key={e.id} delay={Math.min(i, 8) * 26}>
            <div className="effect-card" data-selected={selected ? "true" : "false"}>
              <button
                type="button"
                className="effect-card-open"
                aria-label={`Open ${e.name}`}
                aria-current={selected ? "true" : undefined}
                onClick={() => pick(e.id)}
              >
                <span className="effect-card-preview">
                  <Thumb id={e.id} size="md" label={e.name} live={selected} />
                </span>
                <span className="effect-card-meta">
                  <span className="effect-card-text">
                    <span className="effect-card-name">
                      <Highlighted text={e.name} ranges={row.match?.ranges ?? []} />
                    </span>
                    {row.match?.excerpt && (
                      <span className="effect-card-excerpt">
                        <Highlighted text={row.match.excerpt.text} ranges={row.match.excerpt.ranges} />
                      </span>
                    )}
                  </span>
                  {html && <Chip html>HTML</Chip>}
                  {e.id === activeId && (
                    <span className="effect-card-badge">
                      <span className="effect-card-badge-dot" aria-hidden="true" />
                      running
                    </span>
                  )}
                </span>
              </button>
              <span className="effect-card-fav-wrap">
                <IconButton
                  label={e.favorite ? `Unfavorite ${e.name}` : `Favorite ${e.name}`}
                  pressed={e.favorite}
                  className={favPop === e.id ? "effect-card-fav pop" : "effect-card-fav"}
                  icon={<Star size={16} weight={e.favorite ? "fill" : "regular"} />}
                  onClick={() => void toggleFavorite(e.id, !e.favorite, e.name)}
                />
              </span>
            </div>
          </Enter>
        );
      })}
    </div>
    {/* The files that did not parse stand under the gallery: they come with the list, and above
        the cards they would push every card down. */}
    {failedStrip}
    </LoadFrame>
  );

  const detailPane = (
    <div className="stack">
      {noLayout && (
        <EmptyState
          icon={<SquaresFour size={24} />}
          title="No layout to run on yet"
          action={
            <ButtonLink href="/" variant="primary" size="sm">
              Open the Layout
            </ButtonLink>
          }
        >
          The effects are still here, ready for one.
        </EmptyState>
      )}
      {!(noLayout && !detail) && (
      <LoadFrame
        className="stack"
        state={detail ? "ready" : error ? "error" : "loading"}
        shape={<EffectDetailShape />}
        label="Loading the effect"
        error={
          <ErrorPanel
            title="The effect could not be read"
            summary="canvas-rgb could not read that effect. The others are still here."
            message={error}
            onRetry={() => {
              setError(null);
              setReloadTick((t) => t + 1);
            }}
          />
        }
      >
      {detail && (
        <>
          <Panel>
            <div className="stack" style={{ gap: 12 }}>
              <div className="effect-live" data-stale={stale ? "true" : "false"}>
                {needsInput ? (
                  <Thumb id={detail.schema.id} size="lg" label={detail.schema.name} live />
                ) : (
                  <div className="effect-live-frame" key={detail.schema.id}>
                    <EffectPreview id={detail.schema.id} />
                  </div>
                )}
              </div>
              <div className="row-wrap" style={{ justifyContent: "space-between" }}>
                <h2 className="effect-title">{detail.schema.name}</h2>
                <Button
                  variant={running ? "default" : "primary"}
                  disabled={running || stale}
                  onClick={() => void activate(null)}
                  icon={<Play size={14} weight="fill" />}
                >
                  <Swap labels={["Activate", "Running"]} at={running ? 1 : 0} />
                </Button>
              </div>
              <p className="dim">{detail.schema.description}</p>
              <div className="row-wrap">
                {detail.schema.needs.screen && <Chip icon={<MonitorPlay size={14} />}>Screen capture</Chip>}
                {detail.schema.needs.audio && <Chip icon={<Waveform size={14} />}>Audio input</Chip>}
                {selectedId?.startsWith("html:") && <Chip html>HTML effect</Chip>}
                <StateDot tone={running ? "ok" : "mute"}>{running ? "Running" : "Idle"}</StateDot>
              </div>
              <p className="effect-live-note">
                {needsInput ? "Live frame" : "Preview"}
              </p>
              {selectedId?.startsWith("html:") && (
                <div className="effect-delete">
                  {deleting === selectedId ? (
                    <>
                      <span>
                        {running ? `${detail.schema.name} is playing. ` : ""}
                        {detail.user_presets.length > 0
                          ? `Delete it? It has ${detail.user_presets.length} preset${detail.user_presets.length === 1 ? "" : "s"} of yours.`
                          : "Delete it anyway?"}
                      </span>
                      <div className="row-wrap">
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => void deleteEffect(selectedId, detail.schema.name)}
                        >
                          {detail.user_presets.length > 0 ? "Delete with presets" : "Delete"}
                        </Button>
                        {detail.user_presets.length > 0 && (
                          <Button
                            size="sm"
                            onClick={() => void deleteEffect(selectedId, detail.schema.name, true)}
                          >
                            Keep presets
                          </Button>
                        )}
                        <Button variant="ghost" size="sm" onClick={() => setDeleting(null)}>
                          Cancel
                        </Button>
                      </div>
                    </>
                  ) : (
                    <Button
                      variant="ghost"
                      size="sm"
                      icon={<Trash size={16} />}
                      onClick={() => setDeleting(selectedId)}
                    >
                      Delete effect
                    </Button>
                  )}
                </div>
              )}
            </div>
          </Panel>

          <PresetStrip
            builtIn={detail.schema.presets}
            user={detail.user_presets}
            defs={detail.schema.properties}
            live={{ ...detail.props, ...values }}
            running={runningPreset}
            disabled={stale}
            onApply={applyPreset}
            onSave={savePreset}
            onUpdate={updatePreset}
            onReset={applyPreset}
            onRename={renamePreset}
            onDuplicate={duplicatePreset}
            onDelete={deletePreset}
            undo={undo && detail.schema.id === selectedId ? { name: undo.name } : null}
            onUndo={async () => {
              if (undo) await undo.run();
            }}
          />

          <Panel title="Properties">
            <PropertyControls
              schema={detail.schema}
              values={values}
              onChange={onChange}
            />
            {error && <p className="error-line" style={{ marginTop: 8 }}>{error}</p>}
          </Panel>
        </>
      )}
      </LoadFrame>
      )}
    </div>
  );

  return (
    <div
      className="stack"
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={(e) => {
        if (e.currentTarget === e.target) setDragging(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        setDragging(false);
        const file = e.dataTransfer.files?.[0];
        if (file) void sendImport(file, false);
      }}
    >
      <div className="page-head">
        <h1>Effects</h1>
      </div>

      {dragging && (
        <div className="drop-overlay" aria-hidden="true">
          <div className="drop-card">
            <FileHtml size={26} />
            <span>Drop an .html effect to add it</span>
          </div>
        </div>
      )}

      <div className="effect-toolbar">
        <Segmented<"all" | "favorites">
          label="Filter effects"
          value={filter}
          options={[
            { value: "all", label: "All" },
            { value: "favorites", label: "Favorites" },
          ]}
          onChange={setFilter}
        />
        <span className="effect-categories" role="group" aria-label="Filter by category">
          {EFFECT_CATEGORIES.map((c) => (
            <Chip
              key={c.value}
              pressed={category === c.value}
              title={c.hint}
              onClick={() => setCategory((cur) => (cur === c.value ? null : c.value))}
            >
              {c.label}
            </Chip>
          ))}
        </span>
        <span className="effect-search">
          <MagnifyingGlass className="effect-search-icon" size={14} aria-hidden="true" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search effects"
            aria-label="Search effects"
          />
        </span>
        {/* A count is shown once the list is known: "0 effects" while loading or on an error would be a false answer. */}
        <span className="mute fig" data-testid="effect-count">
          {effects === null || listStale ? "–" : <AnimatedNumber value={list.length} />} effects
        </span>
        <FileButton
          accept=".html,text/html"
          icon={<UploadSimple size={16} />}
          disabled={importing}
          onFile={(file) => void sendImport(file, false)}
        >
          Import
        </FileButton>
      </div>

      {(replaceWith || importError) && (
        <div className="import-note" data-tone={replaceWith ? "ask" : "down"}>
          {replaceWith ? (
            <>
              <span>Replace {replaceWith.fileName}?</span>
              <div className="row-wrap">
                <Button
                  variant="primary"
                  size="sm"
                  disabled={importing}
                  onClick={() => void sendImport(new File([replaceWith.html], replaceWith.fileName), true)}
                >
                  Replace
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setReplaceWith(null)}>
                  Cancel
                </Button>
              </div>
            </>
          ) : (
            <span className="error-line">{importError}</span>
          )}
        </div>
      )}

      {listStale && (
        // The list is on screen but the last refresh failed: the cards may be old, and the count
        // is a dash until the list is read again. The raw words are behind the disclosure.
        <div className="rule rule-warn" role="status">
          The effect list may be out of date.
          <Details>{effectsError}</Details>
        </div>
      )}

      {narrow ? (
        <div
          className="effects-push"
          data-pushed={pushed ? "true" : "false"}
          data-closing={closing ? "true" : "false"}
          data-settled={settled ? "true" : "false"}
        >
          <div className="effect-gallery-wrap" inert={pushed}>
            {gallery}
          </div>
          {pushed && (
            <div className="effect-sheet" data-closing={closing ? "true" : "false"} style={{ "--sheet-top": sheetTop ?? "0px" } as React.CSSProperties}>
              <div className="effect-sheet-head">
                <IconButton label="Back to the effects" icon={<CaretLeft size={18} />} onClick={close} />
                <span>Effects</span>
              </div>
              {detailPane}
            </div>
          )}
        </div>
      ) : (
        <div className="effects-grid">
          {gallery}
          {detailPane}
        </div>
      )}
    </div>
  );
}
