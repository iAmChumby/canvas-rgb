"use client";

import { useEffect, useRef, useState } from "react";
import { CaretLeft, MagnifyingGlass, MonitorPlay, Play, SquaresFour, Star, Waveform } from "@phosphor-icons/react";
import { Button, ButtonLink, IconButton } from "@/components/Button";
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
import { Skeleton, SkeletonLines } from "@/components/Skeleton";
import { useToast } from "@/components/Toast";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { EFFECT_CATEGORIES, categoryOf, type EffectCategory } from "@/lib/effectCategories";
import { getSnap, refreshAll, useSnap } from "@/lib/store";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { PropValue } from "@/proto/PropValue";

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
  const status = useSnap((s) => s.status);
  const activeId = useSnap((s) => s.status?.active_effect ?? null);
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
  const [favPop, setFavPop] = useState<string | null>(null);
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
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const selectedRef = useRef<string | null>(null);
  // The values the daemon has: a failed edit goes back to these.
  const saved = useRef<Values>({});
  useEffect(() => {
    selectedRef.current = selectedId;
  }, [selectedId]);

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
    };
  }, [selectedId, activeId]);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const flush = async () => {
    timer.current = null;
    const id = selectedRef.current;
    const batch = pending.current;
    pending.current = {};
    if (!id || Object.keys(batch).length === 0) return;
    try {
      if (getSnap().status?.active_effect === id) {
        await api.patchActiveProps(batch);
      } else {
        // The first edit to a stopped effect starts it, with the edited values on top.
        await api.activateEffect(id, { props: batch, preset: null });
        await refreshAll();
      }
      saved.current = { ...saved.current, ...batch };
      setError(null);
    } catch (e) {
      setError(messageOf(e));
      toast(messageOf(e), "down");
      // The control goes back to the value the daemon has: nothing fails silently.
      setValues(saved.current);
    }
  };

  const onChange = (key: string, value: PropValue) => {
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

  const gallery = (
    <div className="effect-gallery">
      {effects === null
        ? Array.from({ length: 12 }, (_, i) => (
            <div className="effect-card effect-card-skeleton" key={`skeleton-${i}`} aria-hidden="true">
              <Skeleton className="skeleton-preview" />
              <span className="effect-card-meta">
                <Skeleton className="skeleton-line" height={20} width="55%" />
              </span>
            </div>
          ))
        : list.map((row, i) => {
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
      {list.length === 0 && (
        <Panel className="effect-empty">
          <p className="mute">No effect matches that.</p>
        </Panel>
      )}
    </div>
  );

  const detailPane = (
    <div className="stack">
      {noLayout && (
        <EmptyState
          icon={<SquaresFour size={24} />}
          title="There is no layout yet"
          action={
            <ButtonLink href="/" variant="primary" size="sm">
              Open the Layout
            </ButtonLink>
          }
        >
          Nothing can run until the desk canvas has a zone in it. The effects are still here, ready
          for one.
        </EmptyState>
      )}
      {!noLayout && !detail && (
        <Panel title="Loading">
          <SkeletonLines rows={4} />
        </Panel>
      )}
      {detail && (
        <>
          <Panel>
            <div className="stack" style={{ gap: 12 }}>
              <div className="effect-live" data-stale={stale ? "true" : "false"}>
                {needsInput ? (
                  <Thumb id={detail.schema.id} size="lg" label={detail.schema.name} live />
                ) : (
                  <div className="effect-live-frame" key={detail.schema.id}>
                    <EffectPreview id={detail.schema.id} values={values} />
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
                  {running ? "Running" : "Activate"}
                </Button>
              </div>
              <p className="dim">{detail.schema.description}</p>
              <div className="row-wrap">
                {detail.schema.needs.screen && <Chip icon={<MonitorPlay size={14} />}>Screen capture</Chip>}
                {detail.schema.needs.audio && <Chip icon={<Waveform size={14} />}>Audio input</Chip>}
                {selectedId?.startsWith("html:") && <Chip html>HTML effect</Chip>}
                <StateDot tone={running ? "ok" : "mute"}>{running ? "Running on the layout" : "Not running"}</StateDot>
              </div>
              <p className="effect-live-note">
                {needsInput
                  ? "This effect reads the screen or the audio input, so the preview is the daemon's frame."
                  : "Preview: the editor's own draw of this effect. The daemon owns the LEDs."}
              </p>
            </div>
          </Panel>

          {detail.schema.presets.length > 0 && (
            <Panel title="Presets">
              <div className="preset-row">
                {detail.schema.presets.map((p) => (
                  <Chip key={p.name} label={`Apply preset ${p.name}`} onClick={() => void activate(p.name)}>
                    {p.name}
                  </Chip>
                ))}
              </div>
            </Panel>
          )}

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
    </div>
  );

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Effects</h1>
        <p>Every control is live: the preview follows your edits as you make them.</p>
      </div>

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
        <span className="mute fig">{list.length} effects</span>
      </div>

      <p className="effect-hint">
        Add SignalRGB-format <code className="fig">.html</code> effects to{" "}
        <code className="fig">%APPDATA%\canvas-rgb\effects</code>.
      </p>

      {effectsError && <p className="error-line">{effectsError}</p>}

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
