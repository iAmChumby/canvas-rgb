"use client";

import { useEffect, useRef, useState } from "react";
import { Play, Star, Waveform, MonitorPlay } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip, RowButton } from "@/components/Chip";
import { Thumb } from "@/components/Thumb";
import { Panel } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { StateDot } from "@/components/Panel";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { getSnap, refreshAll, useSnap } from "@/lib/store";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { PropValue } from "@/proto/PropValue";

/** Live edits are sent this long after the last change (ms). */
const PATCH_DEBOUNCE_MS = 60;

export function EffectsView() {
  const effects = useSnap((s) => s.effects);
  const effectsError = useSnap((s) => s.effectsError);
  const activeId = useSnap((s) => s.status?.active_effect ?? null);
  const [filter, setFilter] = useState<"all" | "favorites">("all");
  const [picked, setPicked] = useState<string | null>(null);
  // The running effect is the default; a click on the list overrides it.
  const selectedId = picked ?? activeId ?? effects?.[0]?.id ?? null;
  const setSelectedId = setPicked;
  const [detail, setDetail] = useState<EffectDetail | null>(null);
  const [values, setValues] = useState<Values>({});
  const [error, setError] = useState<string | null>(null);

  const pending = useRef<Values>({});
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const selectedRef = useRef<string | null>(null);
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
        setDetail(d);
        setValues(d.props);
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
      setError(null);
    } catch (e) {
      setError(messageOf(e));
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
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const toggleFavorite = async (id: string, favorite: boolean) => {
    try {
      await api.setFavorite(id, { favorite });
      await refreshAll();
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const list = (effects ?? []).filter((e) => filter === "all" || e.favorite);
  const running = selectedId !== null && selectedId === activeId;

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Effects</h1>
        <p>Twelve built in, plus your HTML effects. Controls come from each effect&apos;s own property list.</p>
      </div>

      <div className="effects-grid">
        <div className="list-panel">
          <div className="list-head">
            <Segmented<"all" | "favorites">
              label="Filter effects"
              value={filter}
              options={[
                { value: "all", label: "All" },
                { value: "favorites", label: "Favorites" },
              ]}
              onChange={setFilter}
            />
            <span className="mute fig">{list.length}</span>
          </div>
          {effectsError && <p className="error-line" style={{ padding: 12 }}>{effectsError}</p>}
          {list.map((e) => {
            const html = e.id.startsWith("html:");
            return (
              <div key={e.id} className="effect-row">
                <RowButton current={e.id === selectedId} label={`Open ${e.name}`} onSelect={() => setSelectedId(e.id)}>
                  <Thumb id={e.id} size="sm" label={e.name} />
                  <span className="stack" style={{ gap: 2, minWidth: 0 }}>
                    <span className="row-title">
                      <span className="one-line">{e.name}</span>
                      {html && <Chip html>HTML</Chip>}
                    </span>
                    {e.id === activeId ? (
                      <StateDot tone="ok">running</StateDot>
                    ) : (
                      <span className="row-sub one-line">{e.description}</span>
                    )}
                  </span>
                  <span />
                </RowButton>
                <IconButton
                  label={e.favorite ? `Unfavorite ${e.name}` : `Favorite ${e.name}`}
                  pressed={e.favorite}
                  icon={<Star size={18} weight={e.favorite ? "fill" : "regular"} />}
                  onClick={() => void toggleFavorite(e.id, !e.favorite)}
                />
              </div>
            );
          })}
        </div>

        <div className="stack">
          {!detail && <Panel><p className="mute">{error ?? "Choose an effect."}</p></Panel>}
          {detail && (
            <>
              <Panel>
                <div className="stack" style={{ gap: 12 }}>
                  <div className="effect-hero">
                    <Thumb id={detail.schema.id} size="lg" label={detail.schema.name} />
                    <div className="stack" style={{ gap: 6, minWidth: 0 }}>
                      <h1 className="effect-title">{detail.schema.name}</h1>
                      <p className="dim">{detail.schema.description}</p>
                    </div>
                    <div className="row-wrap">
                      <Button
                        variant={running ? "default" : "primary"}
                        disabled={running}
                        onClick={() => void activate(null)}
                        icon={<Play size={14} weight="fill" />}
                      >
                        {running ? "Running" : "Activate"}
                      </Button>
                    </div>
                  </div>
                  <div className="row-wrap">
                    {detail.schema.needs.screen && <Chip icon={<MonitorPlay size={14} />}>Screen capture</Chip>}
                    {detail.schema.needs.audio && <Chip icon={<Waveform size={14} />}>Audio input</Chip>}
                    {selectedId?.startsWith("html:") && <Chip html>HTML effect</Chip>}
                    <StateDot tone={running ? "ok" : "mute"}>{running ? "Running on the layout" : "Not running"}</StateDot>
                  </div>
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
      </div>
    </div>
  );
}
