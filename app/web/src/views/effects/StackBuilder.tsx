"use client";

// The stack builder (D95.5): the Effects route in build mode. The picture and its layers on the
// left, the picked layer on the right, the stack's presets below it. The preview is the daemon's
// second engine (`?stack=1`); Try on the lights is a stack draft; Save writes the stack and
// activates it. Nothing here writes an effect file or the layout except those two routes.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowCounterClockwise,
  ArrowLeft,
  CopySimple,
  Eye,
  EyeSlash,
  FloppyDisk,
  Lightbulb,
  Monitor,
  Plus,
  Trash,
  Warning,
  X,
} from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { LoadFrame, ErrorPanel, type LoadState } from "@/components/Loading";
import { NameField } from "@/components/NameField";
import { Panel } from "@/components/Panel";
import { Popover } from "@/components/Popover";
import { Select } from "@/components/Select";
import { Skeleton } from "@/components/Skeleton";
import { Slider } from "@/components/Slider";
import { NumberField } from "@/components/NumberField";
import { Thumb } from "@/components/Thumb";
import { Toggle } from "@/components/Toggle";
import { useToast } from "@/components/Toast";
import { useMediaQuery } from "@/components/MediaQuery";
import { Highlighted, matchEffect } from "@/components/Search";
import { PropertyControls } from "@/views/PropertyControls";
import { StackAreaPicker } from "@/views/effects/StackAreaPicker";
import { StackPreviewCanvas } from "@/views/effects/StackPreviewCanvas";
import {
  addLayer,
  addLayerRefusal,
  applyMergedProps,
  areaName,
  duplicateLayer,
  htmlLayers,
  isHtmlEffect,
  layerNameProblem,
  layerValues,
  makeLayer,
  mergedProps,
  moveLayer,
  newEpoch,
  nextLayerId,
  patchLayer,
  removeLayer,
  stackNameProblem,
  statusWords,
  toggleLayer,
  MAX_HTML_LAYERS,
  MAX_LAYERS,
  StackSession,
} from "@/views/effects/stackDraft";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import {
  cancelNavigation,
  continueNavigation,
  hasPendingNavigation,
  setNavigationGuard,
} from "@/lib/navigationGuard";
import { PageActions, PageTitle } from "@/shell/PageHeader";
import { refreshAll, useSnap } from "@/lib/store";
import type { BlendMode } from "@/proto/BlendMode";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { LayerArea } from "@/proto/LayerArea";
import type { Preset } from "@/proto/Preset";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";
import type { StackLayerStatus } from "@/proto/StackLayerStatus";

/** Sent this long after the last change, and no later than this after the first. */
const PREVIEW_DEBOUNCE_MS = 400;
const HEARTBEAT_FALLBACK_MS = 5_000;
const LEASE_FALLBACK_MS = 15_000;

const BLENDS: { value: BlendMode; label: string; note: string }[] = [
  { value: "normal", label: "Normal", note: "Normal: covers what is under it; a black part hides it." },
  { value: "add", label: "Add", note: "Add: lightens what is under it; a black part leaves it alone." },
  { value: "multiply", label: "Multiply", note: "Multiply: darkens what is under it; a black part makes it black." },
  { value: "screen", label: "Screen", note: "Screen: lifts what is under it; a black part is invisible." },
];

type DraftState = "off" | "starting" | "running" | "expiring" | "restoring" | "restore-error";

function nameOf(id: string): string {
  return id.startsWith("stack:") ? id.slice("stack:".length) : id;
}

function layerLabel(schemas: Record<string, PropertySchema | null>, effect: string): string {
  return schemas[effect]?.name ?? effect;
}

/** A name for a new layer that is not already one of the stack's. */
function freeLayerName(base: string, layers: StackLayer[]): string {
  const used = new Set(layers.map((layer) => layer.name.toLowerCase()));
  if (!used.has(base.toLowerCase())) return base;
  for (let n = 2; n < 99; n++) {
    const candidate = `${base} ${n}`;
    if (!used.has(candidate.toLowerCase())) return candidate;
  }
  return base;
}

export function StackBuilder({
  initialName,
  onClose,
  onName,
}: {
  /** The `?stack=` value: a saved stack's name, or `new`. */
  initialName: string;
  onClose: () => void;
  /** A newly saved stack's name: the URL becomes `?stack=<name>` without reloading. */
  onName: (name: string) => void;
}) {
  const isNew = initialName === "new";
  const devices = useSnap((s) => s.devices);
  const layouts = useSnap((s) => s.layouts);
  const activeLayout = useSnap((s) => s.activeLayout);
  const status = useSnap((s) => s.status);
  const effects = useSnap((s) => s.effects);
  const layout = layouts?.find((entry) => entry.id === activeLayout) ?? layouts?.[0] ?? null;
  const toast = useToast();

  const [phase, setPhase] = useState<LoadState>(isNew ? "ready" : "loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [base, setBase] = useState<Stack | null>(null);
  const [draft, setDraft] = useState<Stack>({ id: "stack:", layers: [] });
  const [nameDraft, setNameDraft] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<Record<string, StackLayerStatus>>({});
  const [schemas, setSchemas] = useState<Record<string, PropertySchema | null>>({});
  const [presets, setPresets] = useState<{ builtIn: Preset[]; user: Preset[] }>({ builtIn: [], user: [] });
  const [showAreas, setShowAreas] = useState(true);
  const [addOpen, setAddOpen] = useState(false);
  const [addQuery, setAddQuery] = useState("");
  const [pickerFor, setPickerFor] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");
  const [saveState, setSaveState] = useState<"idle" | "saving">("idle");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [clash, setClash] = useState<string | null>(null);
  const [naming, setNaming] = useState(false);
  const [namingError, setNamingError] = useState<string | null>(null);
  /** A leave the unsaved prompt asked for, carried out once the new stack has a name. */
  const leaveAfterName = useRef<(() => void) | null>(null);
  const [leaving, setLeaving] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [draftState, setDraftState] = useState<DraftState>("off");
  const [draftError, setDraftError] = useState<string | null>(null);
  const [draftNote, setDraftNote] = useState<string | null>(null);
  const [expiresIn, setExpiresIn] = useState(0);
  const [presetName, setPresetName] = useState<string | null>(null);
  const [otherStacks, setOtherStacks] = useState<{ id: string; name: string }[]>([]);
  const narrow = useMediaQuery("(max-width: 720px)");

  const addRef = useRef<HTMLSpanElement>(null);
  const draftRef = useRef(draft);
  const nameRef = useRef("");
  const baseRef = useRef<Stack | null>(null);
  const pickedRef = useRef<string | null>(null);
  const mounted = useRef(true);
  const session = useRef<StackSession | null>(null);
  if (!session.current) session.current = new StackSession(newEpoch());
  const previewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const previewSentAt = useRef(0);
  const previewChangedAt = useRef(0);
  const appliedSeq = useRef(0);
  const previewRetry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastDraftKey = useRef("");
  const schemasRef = useRef<Record<string, PropertySchema | null>>({});
  const fetching = useRef(new Set<string>());
  // One add and one effect choice at a time: a second click while the effect is being read is
  // dropped, so two fast clicks cannot share a layer id or pass the HTML cap twice.
  const adding = useRef(false);
  const choosing = useRef(false);
  const draftOn = useRef(false);
  const heartbeatMs = useRef(HEARTBEAT_FALLBACK_MS);
  const leaseMs = useRef(LEASE_FALLBACK_MS);
  const lastBeatAt = useRef(0);
  const restoreStarted = useRef(false);
  const backAsked = useRef(false);
  const allowBack = useRef(false);
  const sentinelPushed = useRef(false);
  /** A Save or Discard that will leave the builder: the guard entry is not popped separately. */
  const pendingLeave = useRef(false);
  /** True when this builder was opened from another page, so there are two entries behind the
   *  prompt's own sentinel: the page and the guard. A direct load has none. */
  const openedInApp = useRef(
    typeof window !== "undefined" && window.history.state?.canvasRgbStack !== undefined,
  );

  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);
  useEffect(() => {
    nameRef.current = nameDraft;
  }, [nameDraft]);
  useEffect(() => {
    baseRef.current = base;
  }, [base]);
  useEffect(() => {
    pickedRef.current = picked;
  }, [picked]);
  useEffect(() => {
    schemasRef.current = schemas;
  }, [schemas]);

  const name = isNew
    ? nameDraft.trim() || "New stack"
    : nameOf(base?.id ?? (initialName.startsWith("stack:") ? initialName : `stack:${initialName}`));
  const otherNames = otherStacks
    .filter((entry) => !base || entry.id !== base.id)
    .map((entry) => entry.name);
  const savedKey = base ? JSON.stringify({ id: base.id, layers: base.layers }) : null;
  const currentKey = JSON.stringify({ id: `stack:${name}`, layers: draft.layers });
  const dirty = base ? currentKey !== savedKey : draft.layers.length > 0 || nameDraft.length > 0;
  const pickedLayer = draft.layers.find((layer) => layer.id === picked) ?? null;
  const count = draft.layers.length;
  const merged = useMemo(() => mergedProps(draft.layers, schemas), [draft.layers, schemas]);
  const appliedPreset = useMemo(() => {
    const all = [...presets.builtIn, ...presets.user];
    return (
      all.find((preset) => {
        const keys = Object.keys(preset.props);
        return keys.length > 0 && keys.every((key) => JSON.stringify(merged[key]) === JSON.stringify(preset.props[key]));
      })?.name ?? null
    );
  }, [presets, merged]);
  const fpsTarget = status?.fps_target ?? 30;
  const htmlBudget = Math.min(8, 400 / Math.max(1, fpsTarget));
  const html = htmlLayers(draft.layers);

  // ---- load -------------------------------------------------------------------------

  const loadDetails = useCallback((stack: Stack) => {
    for (const layer of stack.layers) {
      const id = layer.effect;
      if (schemasRef.current[id] !== undefined || fetching.current.has(id)) continue;
      fetching.current.add(id);
      api
        .effect(id)
        .then((detail) => {
          if (!mounted.current) return;
          setSchemas((current) => ({ ...current, [id]: detail.schema }));
        })
        .catch(() => {
          if (!mounted.current) return;
          setSchemas((current) => ({ ...current, [id]: null }));
        })
        .finally(() => fetching.current.delete(id));
    }
  }, []);

  const loadPresets = useCallback((id: string) => {
    api
      .effect(id)
      .then((detail) => {
        if (!mounted.current) return;
        setPresets({ builtIn: detail.schema.presets, user: detail.user_presets });
      })
      .catch(() => {
        if (!mounted.current) return;
        setPresets({ builtIn: [], user: [] });
      });
  }, []);

  const load = useCallback(() => {
    if (isNew) {
      setPhase("ready");
      setBase(null);
      setDraft({ id: "stack:", layers: [] });
      setPicked(null);
      return;
    }
    setPhase("loading");
    setLoadError(null);
    api
      .stack(initialName)
      .then((detail) => {
        if (!mounted.current) return;
        const stack: Stack = { id: detail.stack.id, layers: detail.stack.layers };
        setBase(stack);
        setDraft(stack);
        setStatuses(Object.fromEntries(detail.layers.map((layer) => [layer.id, layer])));
        setPicked(stack.layers[stack.layers.length - 1]?.id ?? null);
        loadDetails(stack);
        loadPresets(stack.id);
        setPhase("ready");
      })
      .catch((e: unknown) => {
        if (!mounted.current) return;
        setLoadError(messageOf(e));
        setPhase("error");
      });
  }, [initialName, isNew, loadDetails, loadPresets]);

  useEffect(() => {
    mounted.current = true;
    // One tick after the first paint: the builder's own placeholder is on screen before the
    // read starts, and a load started here never calls a setter in the effect body.
    const timer = setTimeout(load, 0);
    api
      .stacks()
      .then((answer) => {
        if (mounted.current) setOtherStacks(answer.stacks.map((entry) => ({ id: entry.id, name: entry.name })));
      })
      .catch(() => undefined);
    return () => {
      clearTimeout(timer);
      mounted.current = false;
    };
    // The builder is mounted for one stack; opening another mounts a new one.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A layer's schema arrives after the draft: fill its controls, and send the preview again so the
  // daemon's statuses name the effect.
  useEffect(() => {
    if (phase === "ready") loadDetails(draft);
  }, [draft, phase, loadDetails]);

  // ---- preview ----------------------------------------------------------------------

  function stackForServer(): Stack {
    const id = isNew
      ? `stack:${nameRef.current.trim() || "New stack"}`
      : baseRef.current?.id ?? (initialName.startsWith("stack:") ? initialName : `stack:${initialName}`);
    return { id, layers: draftRef.current.layers };
  }

  async function sendPreview(retried = false) {
    previewSentAt.current = Date.now();
    const current = session.current;
    if (!current) return;
    const { epoch, seq } = current.next();
    try {
      const answer = await api.putStackPreview({ stack: stackForServer(), epoch, seq });
      if (!mounted.current) return;
      current.observe(answer);
      // A superseded PUT is never resent (D95.4.4): only a new change sends again.
      if (!answer.applied) return;
      if (seq > appliedSeq.current) {
        appliedSeq.current = seq;
        setStatuses(Object.fromEntries(answer.layers.map((layer) => [layer.id, layer])));
        setPreviewError(null);
      }
    } catch (e) {
      if (!mounted.current) return;
      if (!retried && e instanceof ApiFailure && e.status === 503) {
        previewRetry.current = setTimeout(() => void sendPreview(true), e.retryAfterMs ?? 1_000);
        return;
      }
      setPreviewError(messageOf(e));
    }
  }

  function queuePreview() {
    const now = Date.now();
    // A change that follows another within one debounce is a continuing drag: while it lasts,
    // a send goes at least every 400 ms. A lone change always waits 400 ms after itself, so a
    // burst that ends at t=300 sends at t=700.
    const inStream = previewChangedAt.current > 0 && now - previewChangedAt.current < PREVIEW_DEBOUNCE_MS;
    previewChangedAt.current = now;
    const overdue = inStream && previewSentAt.current > 0 && now - previewSentAt.current >= PREVIEW_DEBOUNCE_MS;
    if (previewTimer.current) clearTimeout(previewTimer.current);
    if (overdue) {
      previewTimer.current = null;
      void sendPreview();
      return;
    }
    previewTimer.current = setTimeout(() => {
      previewTimer.current = null;
      void sendPreview();
    }, PREVIEW_DEBOUNCE_MS);
  }

  // The first draft on screen opens the preview at once; every later change waits out the
  // trailing debounce, and never longer than the debounce after the first of a run (D95.4.4).
  useEffect(() => {
    if (phase !== "ready") return;
    const key = JSON.stringify(draft);
    if (key === lastDraftKey.current) return;
    const first = lastDraftKey.current === "";
    lastDraftKey.current = key;
    if (first) void sendPreview();
    else queuePreview();
    // The two senders read the draft from refs and the session's own epoch; only a change to the
    // draft or the phase asks for a new PUT.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft, phase]);

  useEffect(
    () => () => {
      if (previewTimer.current) clearTimeout(previewTimer.current);
      if (previewRetry.current) clearTimeout(previewRetry.current);
      void api.deleteStackPreview().catch(() => undefined);
    },
    [],
  );

  // ---- the draft on the lights ------------------------------------------------------

  function applyStatus(answer: DraftStatus) {
    heartbeatMs.current = answer.heartbeat_ms || HEARTBEAT_FALLBACK_MS;
    leaseMs.current = answer.lease_ms || LEASE_FALLBACK_MS;
    lastBeatAt.current = Date.now();
  }

  async function beat() {
    const current = session.current;
    if (!draftOn.current || !current) return;
    const { epoch, seq } = current.next();
    try {
      const answer = await api.putDraft({ html: "", stack: stackForServer(), epoch, seq, props: {} });
      if (!mounted.current || !draftOn.current) return;
      current.observe(answer);
      if (!answer.active) {
        draftOn.current = false;
        setDraftState("off");
        return;
      }
      applyStatus(answer);
      setDraftState("running");
      setDraftError(null);
    } catch (e) {
      if (!mounted.current || !draftOn.current) return;
      setDraftError(messageOf(e));
      setDraftState("expiring");
    }
  }

  // The 5 s beat while a draft runs (D95.4.5). The interval reads the answer's own timing and
  // the draft from refs, so a render never resets the beat.
  useEffect(() => {
    if (draftState !== "running" && draftState !== "expiring") return;
    const timer = setInterval(() => void beat(), heartbeatMs.current);
    return () => clearInterval(timer);
    // `beat` reads the draft, the session and the engine from refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftState]);

  const startDraft = async () => {
    if (count === 0) return;
    setDraftState("starting");
    setDraftError(null);
    setDraftNote(null);
    restoreStarted.current = false;
    const current = session.current;
    if (!current) return;
    const { epoch, seq } = current.next();
    try {
      const answer = await api.putDraft({ html: "", stack: stackForServer(), epoch, seq, props: {} });
      if (!mounted.current) return;
      current.observe(answer);
      if (!answer.active) {
        setDraftState("off");
        return;
      }
      draftOn.current = true;
      applyStatus(answer);
      setDraftState("running");
    } catch (e) {
      if (!mounted.current) return;
      setDraftState("off");
      setDraftError(messageOf(e));
    }
  };

  const stopDraft = async () => {
    draftOn.current = false;
    setDraftState("restoring");
    setDraftError(null);
    try {
      const answer = await api.deleteDraft();
      if (!mounted.current) return;
      setDraftState("off");
      if (answer.note) setDraftNote(answer.note);
    } catch (e) {
      if (!mounted.current) return;
      setDraftState("restore-error");
      setDraftError(messageOf(e));
    }
  };

  // Leaving the builder stops the draft: nothing is left on the lights without a page to stop it.
  useEffect(
    () => () => {
      if (draftOn.current) {
        draftOn.current = false;
        void api.deleteDraft().catch(() => undefined);
      }
    },
    [],
  );

  // The lease countdown, as #2's editor counts it (D95.5.8).
  useEffect(() => {
    if (draftState !== "expiring") return;
    const tick = () => {
      const left = Math.max(0, lastBeatAt.current + leaseMs.current - Date.now());
      setExpiresIn(Math.ceil(left / 1000));
      if (left > 0 || restoreStarted.current) return;
      restoreStarted.current = true;
      setDraftState("restoring");
      let tries = 0;
      const ask = async () => {
        tries += 1;
        try {
          const answer = await api.draft();
          if (!mounted.current) return;
          if (!answer.active) {
            draftOn.current = false;
            setDraftState("off");
            if (answer.note) setDraftNote(answer.note);
            return;
          }
        } catch {
          // A read that fails is not an ending: the next one may say it is gone.
        }
        if (tries >= 5) {
          setDraftState("restore-error");
          setDraftError("the draft is still on the lights; its ending could not be seen");
          restoreStarted.current = false;
          return;
        }
        setTimeout(() => void ask(), 1_000);
      };
      void ask();
    };
    tick();
    const timer = setInterval(tick, 1_000);
    return () => clearInterval(timer);
  }, [draftState]);

  // ---- unsaved changes --------------------------------------------------------------

  /** Carries out the way out the prompt was asked about: Back, a held page change, or a close. */
  const goAfterLeave = () => {
    if (backAsked.current) {
      backAsked.current = false;
      allowBack.current = true;
      pendingLeave.current = true;
      // Back goes past the guard entry and past the builder, one page in all. A direct load
      // has no page behind it, so the builder closes onto the effects page instead.
      if (openedInApp.current) window.history.go(-2);
      else onClose();
      return;
    }
    if (hasPendingNavigation()) {
      continueNavigation();
      return;
    }
    onClose();
  };

  useEffect(() => {
    if (!dirty) {
      setNavigationGuard(null);
      if (sentinelPushed.current) {
        sentinelPushed.current = false;
        // A leave that is under way navigates itself; popping the guard first would move
        // history one extra time.
        if (!pendingLeave.current) window.history.back();
      }
      return;
    }
    setNavigationGuard(() => {
      backAsked.current = false;
      setLeaving(true);
      return false;
    });
    const onPop = () => {
      if (allowBack.current) return;
      window.history.pushState(
        { ...(window.history.state ?? {}), canvasRgbStackGuard: true },
        "",
        window.location.href,
      );
      sentinelPushed.current = true;
      backAsked.current = true;
      setLeaving(true);
    };
    window.history.pushState(
      { ...(window.history.state ?? {}), canvasRgbStackGuard: true },
      "",
      window.location.href,
    );
    sentinelPushed.current = true;
    window.addEventListener("popstate", onPop);
    return () => {
      window.removeEventListener("popstate", onPop);
      setNavigationGuard(null);
    };
  }, [dirty]);

  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  useEffect(
    () => () => {
      setNavigationGuard(null);
      cancelNavigation();
    },
    [],
  );

  // ---- save -------------------------------------------------------------------------

  const save = async (then?: () => void, replacing = false, replaceId?: string) => {
    if (saveState === "saving") return;
    if (count === 0) {
      setSaveError("Add a layer before saving.");
      return;
    }
    // A saved stack's name is the one it was opened with; a new stack takes the name the
    // prompt collected, which is in the ref because the state update has not rendered yet.
    const wanted = (isNew ? nameRef.current : name).trim();
    if (wanted.length === 0) {
      setNaming(true);
      setNamingError(null);
      return;
    }
    if (!replacing) {
      const problem = stackNameProblem(wanted, otherNames);
      if (problem) {
        setSaveError(problem);
        return;
      }
    }
    // A save that will leave the builder must not also pop its guard entry.
    pendingLeave.current = then !== undefined;
    setSaveState("saving");
    setSaveError(null);
    // A replace writes under the other stack's own id: the daemon overwrites that stack and
    // nothing is deleted first, so a failed write leaves it exactly as it was.
    const body: Stack = { id: replaceId ?? `stack:${wanted}`, layers: draftRef.current.layers };
    let saved: Stack;
    try {
      saved = await api.putStack(body.id, body);
    } catch (e) {
      setSaveState("idle");
      // The save failed, so the builder is not leaving after all.
      pendingLeave.current = false;
      if (e instanceof ApiFailure && e.status === 409) {
        setClash(wanted);
        return;
      }
      setSaveError(messageOf(e));
      return;
    }
    // The activation ends the draft on the lights without putting the old effect back.
    draftOn.current = false;
    setDraftState("off");
    setBase(saved);
    setDraft({ id: saved.id, layers: saved.layers });
    if (isNew) {
      // A replace keeps the other stack's own name, so the strip and the URL take the name
      // that was stored, not the spelling the prompt collected.
      const stored = nameOf(saved.id);
      setNameDraft(stored);
      onName(stored);
    }
    setSaveState("idle");
    setClash(null);
    setNaming(false);
    setOtherStacks((current) =>
      current.some((entry) => entry.id === saved.id)
        ? current
        : [...current, { id: saved.id, name: nameOf(saved.id) }],
    );
    void refreshAll();
    loadPresets(saved.id);
    toast(`${nameOf(saved.id)} is saved`);
    let startError: string | null = null;
    try {
      await api.activateEffect(saved.id, { props: {}, preset: null });
      await refreshAll();
    } catch (e) {
      startError = messageOf(e);
      setSaveError(`Saved, but it did not start: ${startError}`);
    }
    if (!startError) toast(`${nameOf(saved.id)} is running`);
    then?.();
  };

  /** A taken name: overwrite the other stack under its own id in one write. Nothing is deleted
   *  first, so a failed write leaves the other stack exactly as it was. */
  const replaceOther = async () => {
    const wanted = clash;
    if (!wanted) return;
    setClash(null);
    let existing = otherStacks.find((entry) => entry.name.toLowerCase() === wanted.toLowerCase());
    if (!existing) {
      // The clash came from another window; read the list again to find the id to replace.
      try {
        const answer = await api.stacks();
        const found = answer.stacks.find((entry) => entry.name.toLowerCase() === wanted.toLowerCase());
        if (found) existing = { id: found.id, name: found.name };
      } catch {
        // The PUT below answers the same clash again if it is still there.
      }
    }
    await save(undefined, true, existing?.id);
  };

  const nameTheStack = () => {
    const wanted = nameDraft.trim();
    const problem = stackNameProblem(wanted, otherNames);
    if (problem) {
      setNamingError(problem);
      return;
    }
    setNamingError(null);
    setNameDraft(wanted);
    nameRef.current = wanted;
    setNaming(false);
    const after = leaveAfterName.current;
    leaveAfterName.current = null;
    void save(after ?? undefined);
  };

  const revert = () => {
    if (base) {
      setDraft({ id: base.id, layers: base.layers });
      setPicked(base.layers[base.layers.length - 1]?.id ?? null);
    } else {
      setDraft({ id: "stack:", layers: [] });
      setNameDraft("");
      setPicked(null);
    }
    setSaveError(null);
    setClash(null);
  };

  const askLeave = () => {
    if (dirty) setLeaving(true);
    else onClose();
  };

  const leavePrompt = (
    <div className="rd-scrim" role="presentation">
      <div className="rd-picker rd-confirm" role="alertdialog" aria-modal="true" aria-label="Unsaved changes">
        <div className="rd-picker-head">
          <div>
            <h2>Save your changes?</h2>
            <p>The stack has changes that are not saved. Leaving now loses them.</p>
          </div>
        </div>
        <div className="rd-picker-foot">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              setLeaving(false);
              backAsked.current = false;
              cancelNavigation();
            }}
          >
            Keep editing
          </Button>
          <Button
            size="sm"
            variant="danger"
            disabled={saveState === "saving"}
            onClick={() => {
              setLeaving(false);
              goAfterLeave();
            }}
          >
            Discard
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={saveState === "saving"}
            onClick={() => {
              setLeaving(false);
              const wanted = (isNew ? nameRef.current : name).trim();
              if (wanted.length === 0) {
                // The name prompt comes first; saving it also carries out the leave.
                leaveAfterName.current = goAfterLeave;
                setNaming(true);
                setNamingError(null);
                return;
              }
              void save(goAfterLeave);
            }}
          >
            Save
          </Button>
        </div>
      </div>
    </div>
  );

  // ---- layer edits ------------------------------------------------------------------

  const patch = (id: string, change: Partial<StackLayer>) => {
    setDraft((current) => ({ ...current, layers: patchLayer(current.layers, id, change) }));
  };

  const chooseEffect = async (id: string, effect: string) => {
    if (choosing.current) return;
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer || effect === layer.effect) return;
    if (isHtmlEffect(effect) && !isHtmlEffect(layer.effect) && htmlLayers(draftRef.current.layers) >= MAX_HTML_LAYERS) {
      toast(`A stack holds ${MAX_HTML_LAYERS} HTML layers`, "warn");
      return;
    }
    choosing.current = true;
    try {
      let detail = null;
      try {
        detail = await api.effect(effect);
        if (!mounted.current) return;
        setSchemas((current) => ({ ...current, [effect]: detail!.schema }));
      } catch {
        if (!mounted.current) return;
        setSchemas((current) => ({ ...current, [effect]: null }));
      }
      // The draft may have moved while the effect was read: check the cap again.
      const latest = draftRef.current.layers.find((entry) => entry.id === id);
      if (!latest || effect === latest.effect) return;
      if (isHtmlEffect(effect) && !isHtmlEffect(latest.effect) && htmlLayers(draftRef.current.layers) >= MAX_HTML_LAYERS) {
        toast(`A stack holds ${MAX_HTML_LAYERS} HTML layers`, "warn");
        return;
      }
      patch(id, { effect, props: detail ? layerValues(detail.schema, detail.props) : {} });
    } finally {
      choosing.current = false;
    }
  };

  const addEffect = async (effect: string) => {
    if (adding.current) return;
    const refusal = addLayerRefusal(draftRef.current.layers, effect);
    if (refusal) {
      toast(refusal, "warn");
      return;
    }
    adding.current = true;
    try {
      let schema: PropertySchema | null = null;
      let props: Record<string, PropValue> = {};
      try {
        const detail = await api.effect(effect);
        schema = detail.schema;
        props = detail.props;
        setSchemas((current) => ({ ...current, [effect]: detail.schema }));
      } catch {
        setSchemas((current) => ({ ...current, [effect]: null }));
      }
      if (!mounted.current) return;
      // The draft may have moved while the effect was read: check the cap and pick the id
      // against the latest draft, so two fast clicks cannot exceed the HTML cap or share an id.
      const layers = draftRef.current.layers;
      const lateRefusal = addLayerRefusal(layers, effect);
      if (lateRefusal) {
        toast(lateRefusal, "warn");
        return;
      }
      const id = nextLayerId(layers);
      if (!id) return;
      const layer: StackLayer = {
        ...makeLayer(effect, freeLayerName(schema?.name ?? effect, layers), schema, props),
        id,
      };
      // The ref is updated with the draft, so a second click in the same tick sees this layer.
      const next = { ...draftRef.current, layers: addLayer(layers, layer) };
      draftRef.current = next;
      setDraft(next);
      setPicked(id);
      setAddOpen(false);
      setAddQuery("");
    } finally {
      adding.current = false;
    }
  };

  const remove = (id: string) => {
    const index = draft.layers.findIndex((layer) => layer.id === id);
    const next = removeLayer(draft.layers, id);
    setDraft((current) => ({ ...current, layers: next }));
    if (picked === id) setPicked(next[Math.min(index, next.length - 1)]?.id ?? null);
  };

  const duplicate = (id: string) => {
    if (draft.layers.length >= MAX_LAYERS) {
      toast(`A stack holds ${MAX_LAYERS} layers`, "warn");
      return;
    }
    const answer = duplicateLayer(draft.layers, id);
    if (!answer.id) {
      toast(`A stack holds ${MAX_HTML_LAYERS} HTML layers`, "warn");
      return;
    }
    setDraft((current) => ({ ...current, layers: answer.layers }));
    setPicked(answer.id);
  };

  const reorder = (id: string, to: number) => {
    setDraft((current) => ({ ...current, layers: moveLayer(current.layers, id, to) }));
  };

  const beginRename = (layer: StackLayer) => {
    setRenaming(layer.id);
    setRenameText(layer.name);
  };

  const commitRename = () => {
    if (!renaming) return;
    const problem = layerNameProblem(renameText.trim());
    if (problem) {
      toast(problem, "warn");
      return;
    }
    patch(renaming, { name: renameText.trim() });
    setRenaming(null);
  };

  const applyArea = (area: LayerArea) => {
    if (pickerFor) patch(pickerFor, { area });
    setPickerFor(null);
  };

  // ---- presets ----------------------------------------------------------------------

  const applyPreset = (preset: Preset) => {
    setDraft((current) => ({
      ...current,
      layers: applyMergedProps(current.layers, preset.props, schemasRef.current),
    }));
  };

  const savePreset = async () => {
    const wanted = presetName?.trim() ?? "";
    if (!wanted || !base) return;
    try {
      const answer = await api.createPreset(base.id, { name: wanted, props: merged });
      setPresets((current) => ({ ...current, user: answer.user_presets }));
      setPresetName(null);
      toast(`Saved ${wanted}`);
    } catch (e) {
      toast(messageOf(e), "down");
    }
  };

  // ---- the list of effects for Add layer --------------------------------------------

  const addable = useMemo(() => {
    const list = (effects ?? []).filter((effect) => !effect.id.startsWith("stack:"));
    const q = addQuery.trim();
    return list
      .map((effect) => ({ effect, match: matchEffect(q, effect.name, effect.description) }))
      .filter((row): row is { effect: EffectSummary; match: NonNullable<typeof row.match> } => row.match !== null)
      .sort((a, b) => (b.match.score ?? 0) - (a.match.score ?? 0));
  }, [effects, addQuery]);

  // ---- rendering --------------------------------------------------------------------

  if (narrow) {
    return (
      <>
        <PageTitle title={name} context={<span>Stack · {count} layers</span>} leading={<BackKey onClick={askLeave} />} />
        <div className="rd-page rd-stacks" data-testid="stack-builder">
          <Panel title="Open on a wider window" className="rd-builder-narrow">
            <p className="dim">
              The stack builder needs a window wider than 720 pixels. Everything that is on the lights can still be
              seen and activated from this page.
            </p>
            <Button size="sm" variant="primary" onClick={askLeave}>
              Back to the effects
            </Button>
          </Panel>
        </div>
      </>
    );
  }

  return (
    <div className="rd-page rd-stacks" data-testid="stack-builder">
      <PageTitle
        title={name}
        context={<span className="rd-stacks-context">Stack · {count} {count === 1 ? "layer" : "layers"}</span>}
        leading={<BackKey onClick={askLeave} />}
      />
      <PageActions>
        {dirty && <span className="mute rd-unsaved">Unsaved</span>}
        <Button
          size="sm"
          variant="ghost"
          icon={<ArrowCounterClockwise size={14} />}
          disabled={!dirty || saveState === "saving"}
          onClick={revert}
        >
          Revert
        </Button>
        <Button
          size="sm"
          variant="primary"
          icon={<FloppyDisk size={14} />}
          disabled={count === 0 || saveState === "saving"}
          onClick={() => void save()}
        >
          {saveState === "saving" ? "Saving…" : "Save"}
        </Button>
      </PageActions>

      {draftNote && (
        <p className="eff-note" role="status">
          <Warning size={14} aria-hidden="true" /> {draftNote}
        </p>
      )}

      {clash && (
        <div className="import-note" data-tone="ask" role="alert">
          <span>
            <span className="eff-clash-tag">name exists</span> A stack named <span className="fig">{clash}</span> already
            exists. Replace it with this stack? Its layers are overwritten; its presets and favourite stay. If it is
            running, the lights change to this stack.
          </span>
          <div className="row-wrap">
            <Button size="sm" variant="danger" disabled={saveState === "saving"} onClick={() => void replaceOther()}>
              Replace
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setClash(null)}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      {saveError && (
        <p className="error-line" role="alert">
          {saveError}
        </p>
      )}

      {phase === "error" ? (
        <div className="rd-stack-body">
          <div className="rd-stack-main">
            <ErrorPanel
              title="The stack could not be read"
              summary="canvas-rgb could not read that stack. Nothing was changed."
              message={loadError}
              onRetry={load}
            />
          </div>
        </div>
      ) : (
        <LoadFrame
          className="rd-stack-frame"
          state={phase === "loading" ? "loading" : "ready"}
          shape={
            <div className="rd-stack-shape" aria-hidden="true">
              <Skeleton />
              <Skeleton />
            </div>
          }
          label="Loading the stack"
        >
          <div className="rd-stack-body">
            <div className="rd-stack-main">
              <Panel
                className="rd-stack-stage"
                title="Preview"
                actions={
                  <span className="rd-stage-meta">
                    <span className="fig mute">320 × 200 · 15 fps</span>
                    <span className="rd-show-areas">
                      <span className="mute">Show areas</span>
                      <Toggle label="Show the layers' areas" checked={showAreas} onChange={setShowAreas} />
                    </span>
                  </span>
                }
              >
                <StackPreviewCanvas
                  stack={draft}
                  layout={layout}
                  devices={devices}
                  picked={pickedLayer}
                  showAreas={showAreas}
                />
                {draftState === "off" ? (
                  <div className="rd-stage-foot">
                    <span className="note">
                      {previewError
                        ? `The preview stopped: ${previewError}`
                        : "Preview only — nothing is on the lights."}
                    </span>
                    <span className="row-wrap">
                      {fpsTarget > 50 && html > 0 && (
                        <span className="rd-budget-note" title="The per-layer budget above 50 fps">
                          At {fpsTarget} fps an HTML layer&apos;s budget is {htmlBudget.toFixed(1)} ms; a heavy one
                          will be held.
                        </span>
                      )}
                      <Button
                        size="sm"
                        icon={<Lightbulb size={15} />}
                        disabled={count === 0}
                        title={count === 0 ? "Add a layer first" : undefined}
                        onClick={() => void startDraft()}
                      >
                        Try on the lights
                      </Button>
                    </span>
                  </div>
                ) : (
                  <div className="rd-stage-foot rd-draft-foot" data-state={draftState} role="status">
                    <span className="eff-draft-mark" data-state={draftState} aria-hidden="true" />
                    <span className="note">
                      {draftState === "starting"
                        ? "Starting the draft…"
                        : draftState === "running"
                          ? "Draft on the lights"
                          : draftState === "expiring"
                            ? `Draft on the lights, expiring in ${expiresIn}s`
                            : draftState === "restoring"
                              ? "Restoring the previous effect…"
                              : "The previous effect could not be restored."}
                    </span>
                    <span className="row-wrap">
                      {draftState === "restore-error" ? (
                        <Button size="sm" icon={<Lightbulb size={14} />} onClick={() => void stopDraft()}>
                          Retry
                        </Button>
                      ) : (
                        <>
                          <Button size="sm" variant="ghost" disabled={draftState === "restoring"} onClick={() => void stopDraft()}>
                            Stop
                          </Button>
                          <Button
                            size="sm"
                            variant={leaving ? "default" : "primary"}
                            disabled={saveState === "saving" || count === 0}
                            onClick={() => void save()}
                          >
                            Save
                          </Button>
                        </>
                      )}
                    </span>
                  </div>
                )}
                {draftState === "restore-error" && draftError && (
                  <p className="error-line" role="alert">
                    {draftError}
                  </p>
                )}
              </Panel>

              <Panel
                className="rd-layers"
                title="Layers"
                actions={
                  <>
                    <span className="mute fig" style={{ fontSize: "var(--t14)" }}>
                      {count} / {MAX_LAYERS}
                    </span>
                    <span ref={addRef}>
                      <Button
                        size="sm"
                        variant="primary"
                        icon={<Plus size={15} />}
                        disabled={count >= MAX_LAYERS}
                        title={count >= MAX_LAYERS ? `A stack holds ${MAX_LAYERS} layers` : undefined}
                        onClick={() => {
                          setAddQuery("");
                          setAddOpen((open) => !open);
                        }}
                      >
                        Add layer
                      </Button>
                    </span>
                  </>
                }
              >
                {count === 0 ? (
                  <div className="rd-layers-empty">
                    <p>Add the first layer</p>
                    <ul className="rd-how-list">
                      <li>The bottom layer is painted first.</li>
                      <li>Each layer has its own area and blend.</li>
                      <li>A stack runs as one effect on the lights.</li>
                    </ul>
                    <Button size="sm" variant="primary" icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
                      Add the first layer
                    </Button>
                  </div>
                ) : (
                  <ol className="layer-list" aria-label="Layers, top first">
                    {[...draft.layers].reverse().map((layer) => {
                      const status = statuses[layer.id];
                      const words = statusWords(status?.status, status?.last_ms);
                      const index = draft.layers.indexOf(layer);
                      return (
                        <li
                          key={layer.id}
                          className="layer-row"
                          data-selected={layer.id === picked ? "true" : "false"}
                          data-hidden={layer.visible ? "false" : "true"}
                          onDragOver={(event) => {
                            event.preventDefault();
                            event.dataTransfer.dropEffect = "move";
                          }}
                          onDrop={(event) => {
                            event.preventDefault();
                            const id = event.dataTransfer.getData("text/plain");
                            if (id) reorder(id, index);
                          }}
                          onClick={() => setPicked(layer.id)}
                        >
                          <button
                            type="button"
                            className="layer-grip"
                            draggable
                            aria-label={`Reorder ${layer.name}`}
                            title="Drag to reorder; Alt+↑/↓ on the keyboard"
                            onDragStart={(event) => {
                              event.dataTransfer.effectAllowed = "move";
                              event.dataTransfer.setData("text/plain", layer.id);
                            }}
                            onKeyDown={(event) => {
                              if (!event.altKey) return;
                              if (event.key === "ArrowUp" && index < count - 1) {
                                event.preventDefault();
                                reorder(layer.id, index + 1);
                              } else if (event.key === "ArrowDown" && index > 0) {
                                event.preventDefault();
                                reorder(layer.id, index - 1);
                              }
                            }}
                          >
                            <DotsIcon />
                          </button>
                          <Thumb id={layer.effect} size="sm" label={layerLabel(schemas, layer.effect)} />
                          <span className="layer-text">
                            {renaming === layer.id ? (
                              <span onClick={(event) => event.stopPropagation()}>
                                <NameField
                                  value={renameText}
                                  label={`Rename ${layer.name}`}
                                  maxLength={40}
                                  onChange={setRenameText}
                                  onEnter={commitRename}
                                  onEscape={() => setRenaming(null)}
                                />
                              </span>
                            ) : (
                              <span
                                className="layer-name"
                                title="Double-click to rename"
                                onDoubleClick={(event) => {
                                  event.stopPropagation();
                                  beginRename(layer);
                                }}
                              >
                                {layer.name}
                              </span>
                            )}
                            <span className={`layer-meta${words ? " layer-warn" : ""}`}>
                              {words ?? `${blendOf(layer.blend)} ${layer.opacity}% · ${areaName(layer.area, layout, devices)}`}
                            </span>
                          </span>
                          <IconButton
                            label={layer.visible ? `Hide ${layer.name}` : `Show ${layer.name}`}
                            icon={layer.visible ? <Eye size={16} /> : <EyeSlash size={16} />}
                            pressed={!layer.visible}
                            onClick={() =>
                              setDraft((current) => ({ ...current, layers: toggleLayer(current.layers, layer.id) }))
                            }
                          />
                        </li>
                      );
                    })}
                  </ol>
                )}
              </Panel>
            </div>

            <div className="rd-stack-side">
              {pickedLayer ? (
                <Panel className="rd-layer-edit">
                  <div className="layer-edit-head">
                    <div className="layer-edit-title">
                      <Thumb id={pickedLayer.effect} size="md" label={layerLabel(schemas, pickedLayer.effect)} />
                      <div>
                        <h2>{pickedLayer.name}</h2>
                        <span className="mute">
                          {layerLabel(schemas, pickedLayer.effect)} · layer {draft.layers.indexOf(pickedLayer) + 1} of{" "}
                          {count}
                        </span>
                      </div>
                    </div>
                    <span className="layer-edit-actions">
                      <IconButton
                        label={`Duplicate ${pickedLayer.name}`}
                        icon={<CopySimple size={16} />}
                        onClick={() => duplicate(pickedLayer.id)}
                      />
                      <IconButton
                        label={`Delete ${pickedLayer.name}`}
                        icon={<Trash size={16} />}
                        onClick={() => remove(pickedLayer.id)}
                      />
                    </span>
                  </div>

                  <div className="layer-fields">
                    <div className="layer-field">
                      <span>Effect</span>
                      <Select
                        label="Effect"
                        value={pickedLayer.effect}
                        options={(effects ?? [])
                          .filter((effect) => !effect.id.startsWith("stack:"))
                          .map((effect) => ({
                            value: effect.id,
                            label: html && !isHtmlEffect(pickedLayer.effect) && isHtmlEffect(effect.id)
                              ? `${effect.name} (HTML layers full)`
                              : effect.name,
                          }))}
                        onChange={(next) => void chooseEffect(pickedLayer.id, next)}
                      />
                    </div>
                    <div className="layer-field">
                      <span>Blend</span>
                      <Select
                        label="Blend"
                        value={pickedLayer.blend}
                        options={BLENDS.map((blend) => ({ value: blend.value, label: blend.label }))}
                        onChange={(next) => patch(pickedLayer.id, { blend: next as BlendMode })}
                      />
                    </div>
                    <div className="layer-field wide">
                      <span>Opacity</span>
                      <div className="field-slider">
                        <Slider
                          label="Opacity"
                          min={0}
                          max={100}
                          value={pickedLayer.opacity}
                          onChange={(next) => patch(pickedLayer.id, { opacity: next })}
                        />
                        <NumberField
                          label="Opacity value"
                          min={0}
                          max={100}
                          value={pickedLayer.opacity}
                          onCommit={(next) => patch(pickedLayer.id, { opacity: next })}
                        />
                      </div>
                    </div>
                  </div>
                  <p className="blend-note">{BLENDS.find((blend) => blend.value === pickedLayer.blend)?.note}</p>

                  <div className="layer-area">
                    <div className="layer-area-text">
                      <Monitor size={16} aria-hidden="true" />
                      <span>{areaName(pickedLayer.area, layout, devices)}</span>
                    </div>
                    <Button size="sm" variant="ghost" onClick={() => setPickerFor(pickedLayer.id)}>
                      Change area
                    </Button>
                  </div>

                  {schemas[pickedLayer.effect] === null || statuses[pickedLayer.id]?.status === "missing_effect" ? (
                    <p className="error-line">This effect is gone. Choose another.</p>
                  ) : schemas[pickedLayer.effect] === undefined ? (
                    <p className="dim">Loading the effect&apos;s properties…</p>
                  ) : (
                    <PropertyControls
                      schema={schemas[pickedLayer.effect] as PropertySchema}
                      values={layerValues(schemas[pickedLayer.effect], pickedLayer.props)}
                      onChange={(key, value) =>
                        setDraft((current) => ({
                          ...current,
                          layers: current.layers.map((layer) =>
                            layer.id === pickedLayer.id ? { ...layer, props: { ...layer.props, [key]: value } } : layer,
                          ),
                        }))
                      }
                    />
                  )}
                  {(statuses[pickedLayer.id]?.status === "held" || statuses[pickedLayer.id]?.status === "faulted") && (
                    <p className="layer-warn" role="status">
                      This layer is slower than the frame; the rest of the stack keeps its rate.
                    </p>
                  )}
                </Panel>
              ) : (
                <Panel className="rd-layer-edit rd-layer-none" title="No layer picked">
                  <p className="dim">Add a layer to tune it.</p>
                </Panel>
              )}

              <Panel title="Presets" className="rd-presets">
                <div className="preset-chips" role="group" aria-label="Presets">
                  {presets.builtIn.map((preset) => (
                    <Chip
                      key={preset.name}
                      pressed={appliedPreset === preset.name}
                      label={`Apply preset ${preset.name}`}
                      onClick={() => applyPreset(preset)}
                    >
                      {preset.name}
                    </Chip>
                  ))}
                  {presets.user.map((preset) => (
                    <Chip
                      key={preset.name}
                      pressed={appliedPreset === preset.name}
                      label={`Apply preset ${preset.name}`}
                      onClick={() => applyPreset(preset)}
                    >
                      {preset.name}
                    </Chip>
                  ))}
                  <Chip
                    pressed={false}
                    label="Save the current values as a preset"
                    onClick={() => {
                      if (!base) {
                        toast("Save the stack before saving a preset", "warn");
                        return;
                      }
                      setPresetName("");
                    }}
                  >
                    + Save current
                  </Chip>
                </div>
                {presetName !== null && (
                  <div className="preset-save">
                    <NameField
                      value={presetName}
                      label="Preset name"
                      placeholder="Preset name"
                      maxLength={40}
                      onChange={setPresetName}
                      onEnter={() => void savePreset()}
                      onEscape={() => setPresetName(null)}
                    />
                    <Button size="sm" variant="primary" onClick={() => void savePreset()}>
                      Save
                    </Button>
                  </div>
                )}
                <div className="preset-line" data-mode="idle">
                  <span className="preset-status">{appliedPreset ?? "Custom values"}</span>
                </div>
              </Panel>
            </div>
          </div>
        </LoadFrame>
      )}

      <Popover anchor={addRef} open={addOpen} onClose={() => setAddOpen(false)} label="Add a layer" className="rd-add-layer">
        <div className="stack rd-add-layer-body">
          <div className="rd-add-search">
            <input
              className="text-in"
              value={addQuery}
              onChange={(event) => setAddQuery(event.target.value)}
              placeholder="Search effects"
              aria-label="Search effects"
              autoFocus
            />
          </div>
          <ul className="rd-add-list" role="listbox" aria-label="Effects to add">
            {addable.map((row) => {
              const refusal = addLayerRefusal(draft.layers, row.effect.id);
              return (
                <li key={row.effect.id}>
                  <button
                    type="button"
                    className="rd-add-row"
                    role="option"
                    aria-selected={false}
                    disabled={refusal !== null}
                    title={refusal ?? undefined}
                    onClick={() => void addEffect(row.effect.id)}
                  >
                    <Thumb id={row.effect.id} size="sm" label={row.effect.name} />
                    <span className="rd-add-text">
                      <span className="rd-add-name">
                        <Highlighted text={row.effect.name} ranges={row.match.ranges} />
                      </span>
                      <span className="rd-add-desc">{refusal ?? row.effect.description}</span>
                    </span>
                    {isHtmlEffect(row.effect.id) && <Chip html>HTML</Chip>}
                  </button>
                </li>
              );
            })}
            {addable.length === 0 && (
              <li className="dim rd-add-empty">No effect matches that.</li>
            )}
          </ul>
        </div>
      </Popover>

      {pickerFor && (
        <StackAreaPicker
          layer={draft.layers.find((layer) => layer.id === pickerFor) ?? draft.layers[0]!}
          layout={layout}
          devices={devices}
          onUse={(area) => applyArea(area)}
          onClose={() => setPickerFor(null)}
        />
      )}

      {naming && (
        <div className="rd-scrim" role="presentation">
          <div className="rd-picker rd-naming" role="dialog" aria-modal="true" aria-label="Name the stack">
            <div className="rd-picker-head">
              <div>
                <h2>Name this stack</h2>
                <p>A name is not another effect&apos;s, and it is not shared with another stack.</p>
              </div>
              <IconButton
                label="Close"
                icon={<X size={16} />}
                onClick={() => {
                  leaveAfterName.current = null;
                  setNaming(false);
                }}
              />
            </div>
            <NameField
              value={nameDraft}
              label="Stack name"
              placeholder="Sunset glow"
              maxLength={60}
              invalid={namingError !== null}
              describedBy="stack-name-error"
              onChange={(value) => {
                setNameDraft(value);
                setNamingError(null);
              }}
              onEnter={nameTheStack}
              onEscape={() => setNaming(false)}
            />
            {namingError && (
              <p className="error-line" id="stack-name-error" role="alert">
                {namingError}
              </p>
            )}
            <div className="rd-picker-foot">
              <Button
                variant="ghost"
                onClick={() => {
                  leaveAfterName.current = null;
                  setNaming(false);
                }}
              >
                Cancel
              </Button>
              <Button variant="primary" onClick={nameTheStack}>
                Save
              </Button>
            </div>
          </div>
        </div>
      )}

      {leaving && leavePrompt}
    </div>
  );
}

function BackKey({ onClick }: { onClick: () => void }) {
  return (
    <IconButton
      label="Back to the effects"
      size="sm"
      icon={<ArrowLeft size={16} />}
      onClick={onClick}
    />
  );
}

function DotsIcon() {
  return (
    <svg width={16} height={16} viewBox="0 0 16 16" aria-hidden="true">
      {[4, 8, 12].map((cy) =>
        [5, 11].map((cx) => <circle key={`${cx}-${cy}`} cx={cx} cy={cy} r={1.3} fill="currentColor" />),
      )}
    </svg>
  );
}

function blendOf(blend: BlendMode): string {
  return BLENDS.find((entry) => entry.value === blend)?.label ?? "Normal";
}
