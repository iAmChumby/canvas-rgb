"use client";

// The stack builder (D95.5): the Effects route in build mode. The picture and its layers on the
// left, the picked layer on the right, the stack's presets below it. The preview is the daemon's
// second engine (`?stack=1`); Try on the lights is a stack draft; Save writes the stack and
// activates it. Nothing here writes an effect file or the layout except those two routes.

import { Fragment, useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import {
  ArrowCounterClockwise,
  ArrowLeft,
  CaretLeft,
  CircleHalf,
  CopySimple,
  DotsSixVertical,
  Eye,
  EyeSlash,
  FloppyDisk,
  Lightbulb,
  Monitor,
  Play,
  Plus,
  SlidersHorizontal,
  Trash,
  Warning,
  X,
} from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { LoadFrame, ErrorPanel, type LoadState } from "@/components/Loading";
import { NameField } from "@/components/NameField";
import { OptionRow } from "@/components/OptionRow";
import { Panel } from "@/components/Panel";
import { Popover } from "@/components/Popover";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { InlineSkeleton, Skeleton } from "@/components/Skeleton";
import { Slider } from "@/components/Slider";
import { NumberField } from "@/components/NumberField";
import { Thumb } from "@/components/Thumb";
import { Toggle } from "@/components/Toggle";
import { notify } from "@/components/Notify";
import { useMediaQuery } from "@/components/MediaQuery";
import { Highlighted, matchEffect } from "@/components/Search";
import { PropertyControls } from "@/views/PropertyControls";
import { StackAreaPicker } from "@/views/effects/StackAreaPicker";
import { Library, type LibraryMode } from "@/views/effects/Library";
import { MaskChip } from "@/views/effects/MaskChip";
import { Stage, StageTools } from "@/views/effects/Stage";
import {
  addLayer,
  addLayerRefusal,
  applyMergedProps,
  areaAsRect,
  areaName,
  areaPosition,
  duplicateLayer,
  htmlLayers,
  insertLayerAt,
  isHtmlEffect,
  layerNameProblem,
  layerValues,
  makeLayer,
  makeMask,
  maskRefusal,
  maskValues,
  mergedProps,
  moveLayer,
  newEpoch,
  nextLayerId,
  patchLayer,
  patchMask,
  removeLayer,
  setLayerMask,
  stackNameProblem,
  statusWords,
  toggleLayer,
  MASK_LEAD_EFFECTS,
  MAX_HTML_LAYERS,
  MAX_LAYERS,
  StackSession,
  layersForServer,
} from "@/views/effects/stackDraft";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { canRedo, canUndo, endRun, historyOf, record, redo, reset, undo, type History } from "@/lib/draftHistory";
import { dropArea, dropTargetAt, dropTargetWords, pointInBox, type DropBoxes, type DropTarget } from "@/lib/dropTargets";
import {
  CANCEL_ANNOUNCEMENT,
  MASK_EMPTY_HINT,
  TOUCH_HOLD_MS,
  cycleTarget,
  dropAnnouncement,
  liftAnnouncement,
  lifted,
  liftsOnHold,
  liftsOnMove,
  maskTargets,
  NO_DRAG,
  pressMoved,
  readDropBoxes,
  reorderToSlot,
  stackSlots,
  stageTargets,
  targetAnnouncement,
  targetKey,
  type DragSource,
  type DragState,
  type TargetNames,
} from "@/lib/dragSession";
import { onStackPreviewFeed } from "@/lib/stackPreview";
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
import type { LayerMask } from "@/proto/LayerMask";
import type { MaskSource } from "@/proto/MaskSource";
import type { Preset } from "@/proto/Preset";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";
import type { StackLayerStatus } from "@/proto/StackLayerStatus";

/** Sent this long after the last change, and no later than this after the first. */
const PREVIEW_DEBOUNCE_MS = 400;
/**
 * The longest the builder waits for the daemon's own "try again later" before it tries again
 * (issue #97). The daemon's contract is one second, and a render takes at most thirty: a cap
 * that keeps the retry inside one render cycle. Without it a wrong, an abusive or a future
 * `Retry-After` header would leave the preview on its last frame for an hour.
 */
const MAX_PREVIEW_RETRY_MS = 30_000;
const HEARTBEAT_FALLBACK_MS = 5_000;
const LEASE_FALLBACK_MS = 15_000;
/** What Tab reaches inside a dialog, in document order. */
const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

const BLENDS: { value: BlendMode; label: string; note: string }[] = [
  { value: "normal", label: "Normal", note: "Normal: covers what is under it; a black part hides it." },
  { value: "add", label: "Add", note: "Add: lightens what is under it; a black part leaves it alone." },
  { value: "multiply", label: "Multiply", note: "Multiply: darkens what is under it; a black part makes it black." },
  { value: "screen", label: "Screen", note: "Screen: lifts what is under it; a black part is invisible." },
];

type DraftState = "off" | "starting" | "running" | "expiring" | "restoring" | "restore-error";
/** Why the unsaved prompt is open, recorded when it opens and read when it is answered. */
type LeaveReason = "back" | "navigation" | "close";
/** A press that has not yet travelled far enough to be a drag (#257 phase 2). */
interface Press {
  source: DragSource;
  pointerId: number;
  pointerType: string;
  start: { x: number; y: number };
  lifted: boolean;
  timer: ReturnType<typeof setTimeout> | null;
  element: HTMLElement | null;
  move: (event: PointerEvent) => void;
  up: (event: PointerEvent) => void;
  /** The capture went away under a lifted press (the element left the page, the browser took it). */
  lost: () => void;
  /** Once lifted, a finger's move is the drag's, not a page scroll (dnd-kit's TouchSensor rule). */
  touch: (event: TouchEvent) => void;
  /** Escape ends a lifted pointer or touch drag, as it ends a keyboard one. */
  key: (event: KeyboardEvent) => void;
}

function nameOf(id: string): string {
  return id.startsWith("stack:") ? id.slice("stack:".length) : id;
}

/** An effect's name for a row and the summary: the schema's, then the library's, and never the
 *  raw id (its fields arrive after the stack's own read). */
function layerLabel(
  schemas: Record<string, PropertySchema | null>,
  effects: EffectSummary[] | null,
  effect: string,
): string | null {
  const schema = schemas[effect];
  if (schema?.name) return schema.name;
  const listed = effects?.find((entry) => entry.id === effect)?.name;
  if (listed) return listed;
  // null: the name has not arrived. The caller draws an InlineSkeleton, never a word standing in for it.
  return schema === null ? "Missing effect" : null;
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
  onWriteCode,
}: {
  /** The `?stack=` value: a saved stack's name, or `new`. */
  initialName: string;
  onClose: () => void;
  /** A newly saved stack's name: the URL becomes `?stack=<name>` without reloading. */
  onName: (name: string) => void;
  /** The library's last tile: leave the builder and open the HTML effect flow. */
  onWriteCode?: () => void;
}) {
  const isNew = initialName === "new";
  const devices = useSnap((s) => s.devices);
  const layouts = useSnap((s) => s.layouts);
  const activeLayout = useSnap((s) => s.activeLayout);
  const status = useSnap((s) => s.status);
  const effects = useSnap((s) => s.effects);
  const layout = layouts?.find((entry) => entry.id === activeLayout) ?? layouts?.[0] ?? null;

  const [phase, setPhase] = useState<LoadState>(isNew ? "ready" : "loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [base, setBase] = useState<Stack | null>(null);
  /**
   * The draft and the steps that led to it (#257). Every edit is one step, so undo takes back a
   * whole drag or slider scrub and not one frame of it; the draft and its history live in one
   * piece of state, so a change and its step can never come apart.
   */
  const [edit, setEdit] = useState<History<Stack>>(() => historyOf({ id: "stack:", layers: [] }));
  const draft = edit.draft;
  /** A change to the draft: a step of its own, or part of the undo run it names. */
  const setDraft = (update: Stack | ((current: Stack) => Stack), run?: string, hold = false) => {
    setEdit((history) =>
      record(
        history,
        typeof update === "function" ? update(history.draft) : update,
        run === undefined ? {} : { run, hold },
      ),
    );
  };
  /** A checkpoint with no steps behind it: a load, a save or a revert. */
  const resetDraft = (next: Stack) => setEdit((history) => reset(history, next));
  const [nameDraft, setNameDraft] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<Record<string, StackLayerStatus>>({});
  const [schemas, setSchemas] = useState<Record<string, PropertySchema | null>>({});
  const [presets, setPresets] = useState<{ builtIn: Preset[]; user: Preset[] }>({ builtIn: [], user: [] });
  // Parts (the desk's outlines) starts off; a drag shows them anyway (spec §3).
  const [parts, setParts] = useState(false);
  /** The library drag session (#257 phase 2): what is lifted and where it would land. */
  const [drag, setDrag] = useState<DragState>(NO_DRAG);
  /** The live region's line: every lift, target and drop is said here. */
  const [announce, setAnnounce] = useState("");
  /** The phone's tab under the stage: the layer list, or the library. */
  const [phoneTab, setPhoneTab] = useState<"layers" | "library">("layers");
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
  /** The name a new stack was saved under: the guard entry the builder lands on takes it. */
  const pendingName = useRef<string | null>(null);
  /** The code flow was asked for from the library: it opens once a dirty builder has left. */
  const pendingCode = useRef(false);
  const [leaving, setLeaving] = useState<LeaveReason | false>(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  /** What the preview feed says is wrong with it: the daemon refuses a second stack socket. */
  const [feedProblem, setFeedProblem] = useState<string | null>(null);
  const [draftState, setDraftState] = useState<DraftState>("off");
  const [draftError, setDraftError] = useState<string | null>(null);
  const [draftNote, setDraftNote] = useState<string | null>(null);
  const [expiresIn, setExpiresIn] = useState(0);
  const [presetName, setPresetName] = useState<string | null>(null);
  /** The message a control refuses with, shown next to the control itself. */
  const [layerProblem, setLayerProblem] = useState<string | null>(null);
  /** A mask control's own refusal (the HTML cap), shown beside the mask effect Select (#256). */
  const [maskProblem, setMaskProblem] = useState<string | null>(null);
  const [addProblem, setAddProblem] = useState<string | null>(null);
  const [renameProblem, setRenameProblem] = useState<string | null>(null);
  const [presetProblem, setPresetProblem] = useState<string | null>(null);
  const [otherStacks, setOtherStacks] = useState<{ id: string; name: string }[]>([]);
  const narrow = useMediaQuery("(max-width: 720px)");
  /** The library's shape at this width (spec §3): a rail, a tray under the stage, or a tab. */
  const wide = useMediaQuery("(min-width: 1180px)");
  const libraryMode: LibraryMode = narrow ? "tab" : wide ? "rail" : "tray";
  // The phone's layer sheet: Tune pushes it over the builder (D95.5.7).
  const [sheetOpen, setSheetOpen] = useState(false);
  const [chooseFor, setChooseFor] = useState<string | null>(null);
  /** The layer whose mask picker is open, and the control it hangs under (#256). */
  const [maskFor, setMaskFor] = useState<string | null>(null);
  /** The row the mask picker's keyboard cursor is on: an index into leads + rest (#256). */
  const [maskActive, setMaskActive] = useState(0);
  /** A layer row picked up by a finger on its grip, and the row it is over. */
  const dragRef = useRef<DragState>(NO_DRAG);
  /** A press that has not yet become a drag: the tile or grip it started on, and its pointer. */
  const press = useRef<Press | null>(null);
  /** A drag just ended: the click that follows the release is not a tap-to-add. */
  const suppressClick = useRef(false);
  /** The safety timer that drops `suppressClick` when the click it waited for never came. */
  const suppressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** The drop boxes of the drag in progress, read once at the lift and again when the page moves. */
  const boxesRef = useRef<DropBoxes | null>(null);
  /** The release that follows a cancelled drag, still to be swallowed as a click. */
  const orphanUp = useRef<(() => void) | null>(null);

  const libraryRef = useRef<HTMLDivElement>(null);
  const tuneRef = useRef<HTMLSpanElement>(null);
  const chooseRef = useRef<HTMLSpanElement>(null);
  const maskAnchor = useRef<HTMLElement | null>(null);
  const maskListRef = useRef<HTMLUListElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const backRef = useRef<HTMLSpanElement>(null);
  const promptRef = useRef<HTMLDivElement>(null);
  /** Where focus was when the leave prompt opened: it goes back there when the prompt closes. */
  const leaveOpener = useRef<HTMLElement | null>(null);
  const leaveWasOpen = useRef(false);
  /** The key that opened the area picker: focus returns to it on close. */
  const areaOpener = useRef<HTMLElement | null>(null);
  const draftRef = useRef(draft);
  /** A pointer drag or resize on the stage is in progress: undo and redo wait for it. */
  const areaDragging = useRef(false);
  /** Undo or redo was asked for: the next draft change may have taken the picked layer away. */
  const stepped = useRef(false);
  const nameRef = useRef("");
  const baseRef = useRef<Stack | null>(null);
  const pickedRef = useRef<string | null>(null);
  const mounted = useRef(true);
  const session = useRef<StackSession | null>(null);
  if (!session.current) session.current = new StackSession(newEpoch());
  const previewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** The max wait's own timer: a change never moves it, so a steady stream of changes cannot put
   *  a send off for ever. */
  const previewWait = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** When the last change arrived: a change that follows another within one debounce is part of
   *  the same drag, and only then does the max wait apply. */
  const previewChangedAt = useRef(0);
  const previewSentAt = useRef(0);
  const appliedSeq = useRef(0);
  const previewRetry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastDraftKey = useRef("");
  const schemasRef = useRef<Record<string, PropertySchema | null>>({});
  const fetching = useRef(new Set<string>());
  // One add and one effect choice at a time: adds wait their turn in `addChain`, so two fast
  // clicks cannot share a layer id or pass the HTML cap twice.
  const addChain = useRef<Promise<void>>(Promise.resolve());
  const addQueued = useRef(new Set<string>());
  const choosing = useRef(false);
  const draftOn = useRef(false);
  const heartbeatMs = useRef(HEARTBEAT_FALLBACK_MS);
  const leaseMs = useRef(LEASE_FALLBACK_MS);
  const lastBeatAt = useRef(0);
  const restoreStarted = useRef(false);
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

  /** The mask picker's list (#256): the six effects that read well as masks, then every other. */
  const maskList = useMemo(() => {
    const all = (effects ?? []).filter((effect) => !effect.id.startsWith("stack:"));
    const leads: EffectSummary[] = [];
    for (const id of MASK_LEAD_EFFECTS) {
      const effect = all.find((entry) => entry.id === id);
      if (effect) leads.push(effect);
    }
    const leadIds = new Set(leads.map((effect) => effect.id));
    return { leads, rest: all.filter((effect) => !leadIds.has(effect.id)) };
  }, [effects]);

  /** The picker's rows in the order they are drawn: the six leads, then the rest. */
  const maskChoices = useMemo(() => [...maskList.leads, ...maskList.rest], [maskList]);

  // A mask refusal belongs to the layer it was said on: `pick` clears it (#256).

  // The mask picker's listbox takes the focus when it opens, and its keyboard cursor starts on
  // the layer's current mask (the APG listbox pattern, #256).
  useEffect(() => {
    if (maskFor === null) return;
    maskListRef.current?.focus();
  }, [maskFor]);

  // The active row scrolls into view as the arrow keys move it.
  useEffect(() => {
    if (maskFor === null) return;
    const effect = maskChoices[maskActive];
    if (effect) {
      document.getElementById(`mask-option-${effect.id}`)?.scrollIntoView({ block: "nearest" });
    }
  }, [maskActive, maskChoices, maskFor]);

  // ---- focus and keys (D95.5.7, D95.7 item 4) ---------------------------------------

  /** A popover closed by Escape puts focus back on the key that opened it. A click outside
   *  closes it without moving the focus the person already has. */
  const closePopover = (selector: string, opener: RefObject<HTMLElement | null>, close: () => void) => {
    const panel = document.querySelector(selector);
    const inside = panel !== null && document.activeElement instanceof Node && panel.contains(document.activeElement);
    close();
    if (inside) {
      setTimeout(() => opener.current?.querySelector<HTMLElement>("button")?.focus(), 0);
    }
  };

  const closeChoose = () => closePopover(".rd-choose-effect", chooseRef, () => setChooseFor(null));

  /** The area picker closed: focus goes back to the key that opened it. */
  const closePicker = () => {
    setPickerFor(null);
    const opener = areaOpener.current;
    if (opener) setTimeout(() => opener.focus(), 0);
  };

  const openSheet = () => setSheetOpen(true);

  /** Picking a layer: a mask refusal belongs to the layer it was said on, so it clears here (#256). */
  const pick = (id: string | null) => {
    setMaskProblem(null);
    setPicked(id);
  };

  const closeSheet = useCallback(() => setSheetOpen(false), []);

  /** Keep editing: the leave prompt closes, and focus goes back where it was. */
  const keepEditing = useCallback(() => {
    pendingCode.current = false;
    setLeaving(false);
    cancelNavigation();
  }, []);

  // The sheet's own keys: Tab stays inside it while it is open, and Escape closes it. While the
  // area picker or the leave prompt is over the sheet, that dialog owns the keys and the sheet's
  // trap steps aside. Tab from outside the sheet comes back in rather than leaving the keyboard
  // on the page behind it.
  useEffect(() => {
    if (!sheetOpen || pickerFor || leaving) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setSheetOpen(false);
        return;
      }
      if (event.key !== "Tab") return;
      const sheet = sheetRef.current;
      if (!sheet) return;
      const items = Array.from(sheet.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (items.length === 0) {
        event.preventDefault();
        sheet.focus();
        return;
      }
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      if (!sheet.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
        return;
      }
      if (event.shiftKey && (active === first || active === sheet)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || active === sheet)) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [sheetOpen, pickerFor, leaving]);

  // The sheet is opened by Tune: it takes focus, and gives it back to Tune when it closes. Tune
  // is a key and not a remembered element, so a tap on a phone (where the body is the active
  // element) still puts focus on the key the sheet came from. When the picked layer is gone —
  // its Delete closed the sheet and there is no Tune left — focus goes to the library.
  const sheetWasOpen = useRef(false);
  /** The library tab was opened to catch the focus: its first tile takes the keyboard next. */
  const wantLibraryFocus = useRef(false);
  useEffect(() => {
    if (sheetOpen) {
      sheetWasOpen.current = true;
      const sheet = sheetRef.current;
      const first = sheet?.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? sheet)?.focus();
      return;
    }
    if (!sheetWasOpen.current) return;
    sheetWasOpen.current = false;
    const tune = tuneRef.current?.querySelector<HTMLElement>("button");
    const library = libraryRef.current?.querySelector<HTMLElement>("[data-lib-tile]");
    if (!tune && !library) {
      // A phone with no layer left has no Tune and no library on this tab: open the library and
      // its first tile takes the focus, so the keyboard is never stranded on the body.
      wantLibraryFocus.current = true;
      setPhoneTab("library");
      return;
    }
    (tune ?? library)?.focus();
  }, [sheetOpen]);

  // The library tab opened for a fallback focus: its first tile takes the keyboard.
  useEffect(() => {
    if (!wantLibraryFocus.current || phoneTab !== "library") return;
    wantLibraryFocus.current = false;
    libraryRef.current?.querySelector<HTMLElement>("[data-lib-tile]")?.focus();
  }, [phoneTab]);

  // The leave prompt is a modal over the builder and over the sheet: it takes focus, Tab stays
  // inside it, and Escape is Keep editing. Focus goes back where it was when it closes.
  useEffect(() => {
    if (leaving) {
      leaveWasOpen.current = true;
      const active = document.activeElement;
      leaveOpener.current = active instanceof HTMLElement && active !== document.body ? active : null;
      // Keep editing is the prompt's first key: it takes focus when the prompt opens.
      promptRef.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
      return;
    }
    if (!leaveWasOpen.current) return;
    leaveWasOpen.current = false;
    const opener = leaveOpener.current;
    leaveOpener.current = null;
    const back = backRef.current?.querySelector<HTMLElement>("button");
    const tune = tuneRef.current?.querySelector<HTMLElement>("button");
    const library = libraryRef.current?.querySelector<HTMLElement>("[data-lib-tile]");
    const target = opener && opener.isConnected ? opener : (back ?? tune ?? library);
    target?.focus();
  }, [leaving]);

  useEffect(() => {
    if (!leaving) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        keepEditing();
        return;
      }
      if (event.key !== "Tab") return;
      const prompt = promptRef.current;
      if (!prompt) return;
      const items = Array.from(prompt.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (items.length === 0) {
        event.preventDefault();
        prompt.focus();
        return;
      }
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      if (!prompt.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
        return;
      }
      if (event.shiftKey && (active === first || active === prompt)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || active === prompt)) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [leaving, keepEditing]);

  // A phone-to-desktop resize removes the sheet's Back key from the page, and a focused key that
  // is removed leaves the keyboard on the body. Focus moves to the picked layer's row, which the
  // desktop layout has.
  const wasNarrow = useRef(narrow);
  useEffect(() => {
    const was = wasNarrow.current;
    wasNarrow.current = narrow;
    if (!was || narrow) return;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    const row = picked
      ? document.querySelector<HTMLElement>(`.rd-layers .layer-row[data-layer-id="${picked}"]`)
      : null;
    row?.focus();
  }, [narrow, picked]);

  // A layer removed while its sheet is open closes the sheet: `remove` does it, where the change
  // happens, rather than an effect watching the list.

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
      // A mask's effect has its own schema and values (#256), read the same way.
      const ids = [layer.effect, ...(layer.mask ? [layer.mask.effect] : [])];
      for (const id of ids) {
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
    // A drag cut off by a reload never sent its end: undo and redo are free again (#257).
    areaDragging.current = false;
    if (isNew) {
      setPhase("ready");
      setBase(null);
      resetDraft({ id: "stack:", layers: [] });
      pick(null);
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
        resetDraft(stack);
        setStatuses(Object.fromEntries(detail.layers.map((layer) => [layer.id, layer])));
        pick(stack.layers[stack.layers.length - 1]?.id ?? null);
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
    return { id, layers: layersForServer(draftRef.current.layers) };
  }

  async function sendPreview(retried = false) {
    // One retry waits at a time (issue #97): a send that starts now takes the place of the retry
    // that was waiting, instead of leaving it to fire as well. A change that sends again while a
    // retry waits would otherwise cost two requests for one busy answer.
    if (previewRetry.current) {
      clearTimeout(previewRetry.current);
      previewRetry.current = null;
    }
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
      // A busy daemon says when to ask again. Anything else is a transport failure or a server
      // error, and one more try 400 ms later is the difference between a preview that catches up
      // and one that stays on the last frame until the next change. The daemon's wait has a
      // ceiling, so a wrong header cannot strand the preview (issue #97).
      const wait =
        e instanceof ApiFailure
          ? e.status === 503
            ? Math.min(e.retryAfterMs ?? 1_000, MAX_PREVIEW_RETRY_MS)
            : null
          : PREVIEW_DEBOUNCE_MS;
      if (!retried && wait !== null) {
        previewRetry.current = setTimeout(() => void sendPreview(true), wait);
        return;
      }
      setPreviewError(messageOf(e));
    }
  }

  /** Sends now, and forgets both timers: a send is the end of the wait either way. */
  function flushPreview() {
    if (previewTimer.current) clearTimeout(previewTimer.current);
    if (previewWait.current) clearTimeout(previewWait.current);
    previewTimer.current = null;
    previewWait.current = null;
    void sendPreview();
  }

  function queuePreview() {
    const now = Date.now();
    // A change waits 400 ms after itself. A change that follows another within one debounce is
    // part of a drag, and the drag gets a wait of its own that a change does not move, due 400 ms
    // after the last send: a change every 300 or 390 ms then sends every 400 ms, not every 600
    // or 780 (D95.4.4). A change after a quiet spell is alone, and waits out the debounce.
    const inStream = previewChangedAt.current > 0 && now - previewChangedAt.current <= PREVIEW_DEBOUNCE_MS;
    previewChangedAt.current = now;
    if (previewTimer.current) clearTimeout(previewTimer.current);
    previewTimer.current = setTimeout(() => {
      previewTimer.current = null;
      flushPreview();
    }, PREVIEW_DEBOUNCE_MS);
    if (inStream && !previewWait.current) {
      const due = Math.max(0, previewSentAt.current + PREVIEW_DEBOUNCE_MS - now);
      previewWait.current = setTimeout(() => {
        previewWait.current = null;
        flushPreview();
      }, due);
    }
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
      if (previewWait.current) clearTimeout(previewWait.current);
      if (previewRetry.current) clearTimeout(previewRetry.current);
      void api.deleteStackPreview(session.current?.epoch).catch(() => undefined);
    },
    [],
  );

  // The daemon opens one stack preview at a time: a second builder's socket is refused with a
  // reason (`api/preview.rs`), and the page says so instead of showing a still picture.
  useEffect(() => onStackPreviewFeed(setFeedProblem), []);

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
    setDraftNote(null);
    const epoch = session.current?.epoch;
    try {
      const answer = await api.deleteDraft(epoch);
      if (!mounted.current) return;
      setDraftState("off");
      if (answer.applied === false) {
        // Another page owns the lights now: this Stop ended nothing, and saying it did would be
        // a lie (D95.4.4, item 4).
        setDraftNote("Another window has the lights now; this window's Stop ended nothing.");
      } else if (answer.note) setDraftNote(answer.note);
    } catch (e) {
      if (!mounted.current) return;
      setDraftState("restore-error");
      setDraftError(messageOf(e));
    }
  };

  // Leaving the builder stops the draft: nothing is left on the lights without a page to stop it.
  // The page's epoch goes with it, so a builder that lost the lights leaves the newer page's
  // draft alone.
  useEffect(
    () => () => {
      if (draftOn.current) {
        draftOn.current = false;
        void api.deleteDraft(session.current?.epoch).catch(() => undefined);
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

  /** A direct load has no page behind the builder, and a save or a discard leaves the guard entry
   *  on top of the builder's own. Land on that entry and rewrite it, so no entry the browser can
   *  reach reopens the builder (D95.5.8). */
  const closeDirect = () => {
    pendingLeave.current = true;
    allowBack.current = true;
    if (!sentinelPushed.current) {
      onClose();
      return;
    }
    sentinelPushed.current = false;
    const done = () => {
      window.removeEventListener("popstate", done);
      onClose();
    };
    window.addEventListener("popstate", done);
    window.history.go(-1);
  };

  /** The guard entry is a copy of the builder's. When the builder was saved under a new name, the
   *  entry behind it still reads the `new` sentinel: land on it and rewrite its URL (D95.5.1). */
  const dropGuardEntry = () => {
    const wanted = pendingName.current;
    pendingName.current = null;
    if (!wanted) {
      window.history.back();
      return;
    }
    const done = () => {
      window.removeEventListener("popstate", done);
      onName(wanted);
    };
    window.addEventListener("popstate", done);
    window.history.back();
  };

  /** Carries out the way out the prompt was asked about, as it was asked: a Back the guard
   *  caught is not the same leave as the close key pressed afterwards. */
  const goAfterLeave = (reason: LeaveReason) => {
    pendingName.current = null;
    if (reason === "back") {
      allowBack.current = true;
      pendingLeave.current = true;
      // Back goes past the guard entry and past the builder, one page in all. A direct load
      // has no page behind it, so the builder closes onto the effects page instead.
      if (openedInApp.current) window.history.go(-2);
      else closeDirect();
      return;
    }
    if (hasPendingNavigation()) {
      continueNavigation();
      return;
    }
    if (sentinelPushed.current) closeDirect();
    else onClose();
    if (pendingCode.current) {
      pendingCode.current = false;
      onWriteCode?.();
    }
  };

  useEffect(() => {
    if (!dirty) {
      setNavigationGuard(null);
      if (sentinelPushed.current) {
        sentinelPushed.current = false;
        // A leave that is under way navigates itself; popping the guard first would move
        // history one extra time.
        if (!pendingLeave.current) dropGuardEntry();
      }
      pendingName.current = null;
      return;
    }
    setNavigationGuard(() => {
      setLeaving("navigation");
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
      setLeaving("back");
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
    // The guard's own history and the leave helpers read refs and props; only a change to
    // `dirty` registers or drops it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    const body: Stack = { id: replaceId ?? `stack:${wanted}`, layers: layersForServer(draftRef.current.layers) };
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
    resetDraft({ id: saved.id, layers: saved.layers });
    if (isNew) {
      // A replace keeps the other stack's own name, so the strip and the URL take the name
      // that was stored, not the spelling the prompt collected.
      const stored = nameOf(saved.id);
      setNameDraft(stored);
      // Leaving now: the URL is rewritten here, because no guard pop will follow. Otherwise the
      // guard entry is dropped and the entry the builder lands on takes the name (D95.5.1).
      if (pendingLeave.current) onName(stored);
      else pendingName.current = stored;
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
    // The stack is saved. Nothing on screen says so on a phone (the Unsaved mark is a desktop
    // key), so the note is the confirmation there.
    notify(`${nameOf(saved.id)} is saved`);
    try {
      await api.activateEffect(saved.id, { props: {}, preset: null });
      await refreshAll();
    } catch (e) {
      setSaveError(`Saved, but it did not start: ${messageOf(e)}`);
    }
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
      resetDraft({ id: base.id, layers: base.layers });
      pick(base.layers[base.layers.length - 1]?.id ?? null);
    } else {
      resetDraft({ id: "stack:", layers: [] });
      setNameDraft("");
      pick(null);
    }
    setSheetOpen(false);
    setSaveError(null);
    setClash(null);
  };

  const askLeave = () => {
    if (dirty) setLeaving("close");
    else onClose();
  };

  /** The library's "Write code" tile: the code flow, once a dirty builder has left. */
  const writeCode = () => {
    if (dirty) {
      pendingCode.current = true;
      setLeaving("close");
      return;
    }
    onWriteCode?.();
  };

  // ---- undo and redo (#257) ---------------------------------------------------------

  // Undo and redo go through the functional updater, so a change and a step asked for in the same
  // tick compose in order (a ref set in an effect would still hold the draft before the change).
  // A drag in progress finishes first: its gesture is one step, and a step taken in the middle
  // would tear it in two.
  const undoNow = () => {
    if (areaDragging.current) return;
    stepped.current = true;
    setEdit((history) => undo(history));
  };
  const redoNow = () => {
    if (areaDragging.current) return;
    stepped.current = true;
    setEdit((history) => redo(history));
  };

  // After a step: a pick that the draft no longer holds (an undo took the layer back) moves to
  // the top layer. The layer sheet closes too: it was the removed layer's.
  useEffect(() => {
    if (!stepped.current) return;
    stepped.current = false;
    const current = pickedRef.current;
    if (current === null || edit.draft.layers.some((layer) => layer.id === current)) return;
    setMaskProblem(null);
    setPicked(edit.draft.layers[edit.draft.layers.length - 1]?.id ?? null);
    setSheetOpen(false);
  }, [edit]);

  // Ctrl+Z and ctrl+shift+Z (and ctrl+Y) take a step back and take it again, as the Layout page
  // does (views/LayoutView.tsx:530-550). A text field keeps its own undo: the reader there is
  // undoing what they typed. Ctrl+D copies the picked layer; the browser's bookmark key is not
  // wanted in the builder. An open sheet or dialog owns its own keys.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      // The stage and a row handle their own keys first; a key they took is not taken again.
      if (event.defaultPrevented) return;
      const target = event.target as HTMLElement | null;
      // A field, a select and a combobox trigger keep their own keys (a select's list answers
      // Ctrl+letters as type-ahead), and so does anything inside an open popover or dialog.
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable ||
          target.closest('[role="combobox"], [role="listbox"], [role="dialog"], [role="alertdialog"], [role="menu"]'))
      ) {
        return;
      }
      // Any open surface owns the keyboard: the mask picker, the layer sheet, the area picker,
      // the leave and name prompts, and a drag in progress. The DOM check catches a popover or
      // dialog the flags above do not know about.
      if (sheetOpen || pickerFor || leaving || naming || maskFor || drag.source) return;
      if (document.querySelector('.popover[role="dialog"]:not([inert]), [role="alertdialog"], [aria-modal="true"]')) return;
      const key = event.key.toLowerCase();
      if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        undoNow();
        return;
      }
      if ((key === "z" && event.shiftKey) || key === "y") {
        event.preventDefault();
        redoNow();
        return;
      }
      if (key === "d" && pickedRef.current !== null) {
        event.preventDefault();
        duplicate(pickedRef.current);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // `duplicate` reads the draft it was rendered with; the listener is re-registered with it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft, sheetOpen, pickerFor, leaving, naming, maskFor, drag.source]);

  const leavePrompt = (
    <div className="rd-scrim" role="presentation">
      <div
        className="rd-picker rd-confirm"
        role="alertdialog"
        aria-modal="true"
        aria-label="Unsaved changes"
        ref={promptRef}
      >
        <div className="rd-picker-head">
          <div>
            <h2>Save your changes?</h2>
            <p>The stack has changes that are not saved. Leaving now loses them.</p>
          </div>
        </div>
        <div className="rd-picker-foot">
          <Button size="sm" variant="ghost" onClick={keepEditing}>
            Keep editing
          </Button>
          <Button
            size="sm"
            variant="danger"
            disabled={saveState === "saving"}
            onClick={() => {
              const reason: LeaveReason = leaving || "close";
              setLeaving(false);
              goAfterLeave(reason);
            }}
          >
            Discard
          </Button>
          <Button
            size="sm"
            variant="primary"
            busy={saveState === "saving"}
            onClick={() => {
              // The leave the prompt was asked about, carried out once there is a name.
              const reason: LeaveReason = leaving || "close";
              setLeaving(false);
              const wanted = (isNew ? nameRef.current : name).trim();
              if (wanted.length === 0) {
                // The name prompt comes first; saving it also carries out the leave.
                leaveAfterName.current = () => goAfterLeave(reason);
                setNaming(true);
                setNamingError(null);
                return;
              }
              void save(() => goAfterLeave(reason));
            }}
          >
            Save
          </Button>
        </div>
      </div>
    </div>
  );

  // ---- layer edits ------------------------------------------------------------------

  const patch = (id: string, change: Partial<StackLayer>, run?: string, hold = false) => {
    setDraft((current) => ({ ...current, layers: patchLayer(current.layers, id, change) }), run, hold);
  };

  /** The picked layer's area made into the rectangle it covers now, by its own key's name (#257). */
  /**
   * A refusal from a key on the stage: the inspector shows it where it always has, but the
   * inspector may be off screen (a short window, a phone), so it is also a note.
   */
  const refuse = (words: string) => {
    setLayerProblem(words);
    notify(words, "warn");
  };

  const makeRect = (id: string) => {
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer) return;
    const rect = areaAsRect(layer.area, layout);
    if (!rect) {
      refuse("That area is not on the desk, so there is no rectangle to make.");
      return;
    }
    setLayerProblem(null);
    patch(id, { area: rect });
  };

  const chooseEffect = async (id: string, effect: string) => {
    if (choosing.current) return;
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer || effect === layer.effect) return;
    if (isHtmlEffect(effect) && !isHtmlEffect(layer.effect) && htmlLayers(draftRef.current.layers) >= MAX_HTML_LAYERS) {
      setLayerProblem(`A stack holds ${MAX_HTML_LAYERS} HTML layers`);
      return;
    }
    setLayerProblem(null);
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
        setLayerProblem(`A stack holds ${MAX_HTML_LAYERS} HTML layers`);
        return;
      }
      patch(id, { effect, props: detail ? layerValues(detail.schema, detail.props) : {} });
    } finally {
      choosing.current = false;
    }
  };

  /** The mask picker hangs under the control that opened it, and returns focus there on Escape. */
  const openMaskPicker = (id: string, anchor: HTMLElement) => {
    maskAnchor.current = anchor;
    setMaskProblem(null);
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    const at = maskChoices.findIndex((effect) => effect.id === layer?.mask?.effect);
    setMaskActive(at >= 0 ? at : 0);
    setMaskFor(id);
  };

  const closeMaskPicker = () => closePopover(".rd-mask-picker", maskAnchor, () => setMaskFor(null));

  const removeMask = (id: string) => {
    setMaskProblem(null);
    setDraft((current) => ({ ...current, layers: setLayerMask(current.layers, id, null) }));
  };

  const patchMaskDraft = (id: string, change: Partial<LayerMask>, run?: string) => {
    setDraft((current) => ({ ...current, layers: patchMask(current.layers, id, change) }), run);
  };

  /** One mask property changed: a functional patch, so two quick changes cannot drop one (#256). */
  const patchMaskProps = (id: string, key: string, value: PropValue) => {
    setDraft(
      (current) => ({
        ...current,
        layers: current.layers.map((layer) =>
          layer.id === id && layer.mask !== undefined
            ? { ...layer, mask: { ...layer.mask, props: { ...layer.mask.props, [key]: value } } }
            : layer,
        ),
      }),
      `mask:${id}:${key}`,
    );
  };

  /** A mask effect's schema and starting values, read from the daemon; nothing when it is gone. */
  const maskOf = async (effect: string): Promise<{ schema: PropertySchema | null; props: Record<string, PropValue> }> => {
    try {
      const detail = await api.effect(effect);
      if (mounted.current) setSchemas((current) => ({ ...current, [effect]: detail.schema }));
      return { schema: detail.schema as PropertySchema | null, props: detail.props };
    } catch {
      if (mounted.current) setSchemas((current) => ({ ...current, [effect]: null }));
      return { schema: null, props: {} };
    }
  };

  /** Gives a layer a mask of `effect`, or changes the mask it has (#256). */
  const chooseMaskEffect = async (id: string, effect: string) => {
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer) return;
    if (layer.mask?.effect === effect) {
      closeMaskPicker();
      return;
    }
    const refusal = maskRefusal(draftRef.current.layers, id, effect);
    if (refusal) {
      setMaskProblem(refusal);
      return;
    }
    setMaskProblem(null);
    const { schema, props } = await maskOf(effect);
    if (!mounted.current) return;
    // The draft may have moved while the effect was read: check the cap again.
    const latest = draftRef.current.layers.find((entry) => entry.id === id);
    if (!latest) return;
    const lateRefusal = maskRefusal(draftRef.current.layers, id, effect);
    if (lateRefusal) {
      setMaskProblem(lateRefusal);
      return;
    }
    setDraft((current) => ({
      ...current,
      layers: setLayerMask(current.layers, id, makeMask(effect, schema, props)),
    }));
    closeMaskPicker();
  };

  /**
   * A mask dropped on a layer (#257 phase 3): the layer a row or the picture names gets it,
   * replacing the mask it had in the same single step. Nothing changes when the drop is refused.
   */
  const applyMask = async (id: string, effect: string) => {
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer || layer.mask?.effect === effect) return;
    const refusal = maskRefusal(draftRef.current.layers, id, effect);
    if (refusal) {
      setAnnounce(refusal);
      return;
    }
    const { schema, props } = await maskOf(effect);
    if (!mounted.current) return;
    const latest = draftRef.current.layers.find((entry) => entry.id === id);
    if (!latest) return;
    const lateRefusal = maskRefusal(draftRef.current.layers, id, effect);
    if (lateRefusal) {
      setAnnounce(lateRefusal);
      return;
    }
    setDraft((current) => ({
      ...current,
      layers: setLayerMask(current.layers, id, makeMask(effect, schema, props)),
    }));
  };

  /** The mask picker's keys, as a listbox: arrows move, Home/End jump, Enter/Space choose. */
  const onMaskKey = (event: ReactKeyboardEvent<HTMLUListElement>) => {
    const last = maskChoices.length - 1;
    if (last < 0) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const effect = maskChoices[maskActive];
      if (effect && maskFor) void chooseMaskEffect(maskFor, effect.id);
      return;
    }
    let next: number | null = null;
    if (event.key === "ArrowDown") next = Math.min(last, maskActive + 1);
    else if (event.key === "ArrowUp") next = Math.max(0, maskActive - 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = last;
    if (next === null) return;
    event.preventDefault();
    setMaskActive(next);
  };

  const addOne = async (effect: string, place?: { area?: LayerArea; at?: number }) => {
    const refusal = addLayerRefusal(draftRef.current.layers, effect);
    if (refusal) {
      setAddProblem(refusal);
      notify(refusal, "warn");
      return;
    }
    setAddProblem(null);
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
      setAddProblem(lateRefusal);
      notify(lateRefusal, "warn");
      return;
    }
    const id = nextLayerId(layers);
    if (!id) return;
    const layer: StackLayer = {
      ...makeLayer(effect, freeLayerName(schema?.name ?? effect, layers), schema, props),
      id,
      ...(place?.area ? { area: place.area } : {}),
    };
    // The ref is updated with the draft, so a queued add sees this layer.
    const next = {
      ...draftRef.current,
      layers:
        place?.at === undefined ? addLayer(layers, layer) : insertLayerAt(layers, layer, place.at),
    };
    draftRef.current = next;
    setDraft(next);
    pick(id);
  };

  const addEffect = (effect: string, place?: { area?: LayerArea; at?: number }) => {
    // One add at a time, in the order the clicks came (D95.5.3). A second click on a different
    // row waits its turn and lands; a second click on the same row is the same add.
    if (addQueued.current.has(effect)) return;
    addQueued.current.add(effect);
    const run = addChain.current.then(async () => {
      try {
        await addOne(effect, place);
      } finally {
        addQueued.current.delete(effect);
      }
    });
    addChain.current = run.catch(() => undefined);
    return run;
  };

  const remove = (id: string) => {
    const index = draft.layers.findIndex((layer) => layer.id === id);
    const next = removeLayer(draft.layers, id);
    setDraft((current) => ({ ...current, layers: next }));
    if (picked === id) {
      pick(next[Math.min(index, next.length - 1)]?.id ?? null);
      // The sheet is the removed layer's: it closes rather than showing the next one unsought.
      setSheetOpen(false);
    }
  };

  const duplicate = (id: string) => {
    if (draft.layers.length >= MAX_LAYERS) {
      refuse(`A stack holds ${MAX_LAYERS} layers`);
      return;
    }
    const answer = duplicateLayer(draft.layers, id);
    if (!answer.id) {
      refuse(`A stack holds ${MAX_HTML_LAYERS} HTML layers`);
      return;
    }
    setLayerProblem(null);
    setDraft((current) => ({ ...current, layers: answer.layers }));
    pick(answer.id);
  };

  const reorder = (id: string, to: number) => {
    setDraft((current) => ({ ...current, layers: moveLayer(current.layers, id, to) }));
  };

  const beginRename = (layer: StackLayer) => {
    setRenaming(layer.id);
    setRenameText(layer.name);
    setRenameProblem(null);
  };

  const commitRename = () => {
    if (!renaming) return;
    const problem = layerNameProblem(renameText.trim());
    if (problem) {
      setRenameProblem(problem);
      return;
    }
    setRenameProblem(null);
    patch(renaming, { name: renameText.trim() });
    setRenaming(null);
  };

  const applyArea = (area: LayerArea) => {
    if (pickerFor) patch(pickerFor, { area });
    closePicker();
  };

  // ---- the drag session (#257 phase 2) ------------------------------------------------
  // One place lifts a tile or a row, follows the pointer or the keyboard, and says what a drop
  // would do. The old pointer reorder and the HTML5 drag lived here; both are gone.

  const targetNames = (): TargetNames => ({ layout, devices, layers: draftRef.current.layers });

  const showDrag = (next: DragState) => {
    dragRef.current = next;
    setDrag(next);
  };

  /** A target changed: the live region says where the drop would land now. */
  const sayTarget = (target: DropTarget | null) => {
    setAnnounce(target ? targetAnnouncement(target, targetNames()) : "");
  };

  const endPress = () => {
    const current = press.current;
    if (!current) return;
    if (current.timer !== null) clearTimeout(current.timer);
    window.removeEventListener("pointermove", current.move);
    window.removeEventListener("pointerup", current.up);
    window.removeEventListener("pointercancel", current.up);
    window.removeEventListener("touchmove", current.touch);
    window.removeEventListener("keydown", current.key, true);
    current.element?.removeEventListener("lostpointercapture", current.lost);
    press.current = null;
  };

  const clearDrag = () => {
    endPress();
    boxesRef.current = null;
    showDrag(NO_DRAG);
  };

  /** The click that follows a release or a key is not a tap-to-add; the next input or a second ends it. */
  const armSuppress = () => {
    suppressClick.current = true;
    if (suppressTimer.current !== null) clearTimeout(suppressTimer.current);
    suppressTimer.current = setTimeout(() => {
      suppressClick.current = false;
      suppressTimer.current = null;
    }, 1000);
  };

  /**
   * A Space or Enter that steers a lift is a key whose click must not add: Space clicks on its
   * keyup, so the keyup is swallowed with it, and the click it still sends finds the guard armed.
   */
  const guardKeyClick = () => {
    armSuppress();
    const release = (event: KeyboardEvent) => {
      if (event.key === " ") event.preventDefault();
    };
    window.addEventListener("keyup", release, { capture: true, once: true });
  };

  /**
   * The session ends with no drop: the capture is released, the ghost goes, the source is cleared
   * and the live region says nothing changed. A press that never lifted just ends, silently. The
   * release that may still come is swallowed as a click, so a cancelled drag is never also an add.
   */
  const cancelSession = () => {
    const wasLifted = dragRef.current.source !== null;
    const current = press.current;
    if (current?.lifted) {
      try {
        current.element?.releasePointerCapture(current.pointerId);
      } catch {
        // The capture is already gone.
      }
      const swallow = () => {
        window.removeEventListener("pointerup", swallow);
        window.removeEventListener("pointercancel", swallow);
        orphanUp.current = null;
        armSuppress();
      };
      window.addEventListener("pointerup", swallow);
      window.addEventListener("pointercancel", swallow);
      orphanUp.current = () => {
        window.removeEventListener("pointerup", swallow);
        window.removeEventListener("pointercancel", swallow);
        orphanUp.current = null;
      };
    }
    clearDrag();
    if (wasLifted) setAnnounce(CANCEL_ANNOUNCEMENT);
  };

  const moveDrag = (point: { x: number; y: number }, shift: boolean) => {
    const current = dragRef.current;
    if (!current.source) return;
    // The boxes were read at the lift and are read again when the page scrolls or resizes, not
    // on every move: a move only compares a point with them.
    const boxes = (boxesRef.current ??= readDropBoxes());
    const target = dropTargetAt({
      point,
      boxes,
      layout,
      layers: draftRef.current.layers,
      payload: current.source.kind,
      shift,
    });
    // A mask over bare picture has nothing to land on: the drop would refuse, and the reason
    // follows the pointer (§4).
    const hint =
      current.source.kind === "mask" && target === null && pointInBox(point, boxes.stage)
        ? MASK_EMPTY_HINT
        : null;
    if (hint !== current.hint && hint !== null) setAnnounce(hint);
    else if (targetKey(target) !== targetKey(current.target)) sayTarget(target);
    showDrag({ ...current, target, point, hint });
  };

  /** The drop, applied once: one drop is one undo step. */
  const commitDrop = (source: DragSource, target: DropTarget | null) => {
    const names = targetNames();
    if (!target) {
      setAnnounce(source.kind === "mask" ? MASK_EMPTY_HINT : CANCEL_ANNOUNCEMENT);
      clearDrag();
      return;
    }
    if (source.kind === "mask") {
      if (target.kind === "row" || target.kind === "maskLayer") {
        void applyMask(target.id, source.effect);
        setAnnounce(dropAnnouncement(source, target, names));
      }
      clearDrag();
      return;
    }
    if (source.kind === "effect") {
      if (target.kind === "stackSlot" || target.kind === "row") {
        void addEffect(source.effect, { at: target.kind === "stackSlot" ? target.at : target.index + 1 });
      } else {
        const area = dropArea(target);
        if (area) void addEffect(source.effect, { area });
      }
    } else if (target.kind === "stackSlot" || target.kind === "row") {
      // A drop on a row takes that row's place: it goes above it.
      const at = target.kind === "stackSlot" ? target.at : target.index + 1;
      setDraft((now) => ({ ...now, layers: reorderToSlot(now.layers, source.id, at) }));
    }
    setAnnounce(dropAnnouncement(source, target, names));
    clearDrag();
  };

  const beginKeyboard = (source: DragSource) => {
    const layers = draftRef.current.layers;
    let first: DropTarget | null;
    if (source.kind === "mask") {
      // A mask starts on the picked layer's row when there is one, otherwise the top row (the
      // last in the stack's own bottom-first order).
      const targets = maskTargets(layers);
      first =
        targets.find((target) => target.kind === "maskLayer" && target.id === picked) ??
        targets[targets.length - 1] ??
        null;
    } else if (source.kind === "effect") {
      first = stageTargets(layout)[0] ?? null;
    } else {
      first = { kind: "stackSlot", at: Math.max(0, layers.findIndex((layer) => layer.id === source.id)) };
    }
    // The key that lifts is a key whose click must not also add the tile.
    guardKeyClick();
    showDrag(lifted(source, "keyboard", first));
    setAnnounce(liftAnnouncement(source));
  };

  const beginPress = (source: DragSource, event: ReactPointerEvent<HTMLElement>) => {
    if (event.button > 0 || dragRef.current.source) return;
    endPress();
    orphanUp.current?.();
    const pointerType = event.pointerType || "mouse";
    const start = { x: event.clientX, y: event.clientY };
    const element = event.currentTarget;
    const current: Press = {
      source,
      pointerId: event.pointerId,
      pointerType,
      start,
      lifted: false,
      timer: null,
      element,
      move: () => {},
      up: () => {},
      lost: () => {},
      touch: () => {},
      key: () => {},
    };
    /** The move or the hold went past the slop: the drag begins where the press did. */
    const lift = () => {
      if (current.lifted) return;
      current.lifted = true;
      if (current.timer !== null) {
        clearTimeout(current.timer);
        current.timer = null;
      }
      try {
        element.setPointerCapture(current.pointerId);
      } catch {
        // A pointer that is already gone has nothing to capture.
      }
      const boxes = readDropBoxes();
      boxesRef.current = boxes;
      const target = dropTargetAt({ point: start, boxes, layout, layers: draftRef.current.layers, payload: source.kind });
      showDrag({ source, mode: pointerType === "touch" ? "touch" : "pointer", target, point: start, hint: null });
      setAnnounce(liftAnnouncement(source));
    };
    current.move = (e) => {
      if (e.pointerId !== current.pointerId || !press.current) return;
      if (!current.lifted) {
        if (!liftsOnMove(pointerType)) {
          // A finger that travels before the hold is a scroll, not a lift: the hold is off.
          if (pressMoved(start, { x: e.clientX, y: e.clientY })) endPress();
          return;
        }
        if (!pressMoved(start, { x: e.clientX, y: e.clientY })) return;
        lift();
      }
      // The press is the drag's: a finger does not scroll the page under the ghost.
      e.preventDefault();
      moveDrag({ x: e.clientX, y: e.clientY }, e.shiftKey);
    };
    current.up = (e) => {
      if (e.pointerId !== current.pointerId) return;
      const wasLifted = current.lifted;
      endPress();
      if (!wasLifted) return;
      // The click that follows a release must not also add the tile.
      armSuppress();
      if (e.type === "pointercancel") {
        setAnnounce(CANCEL_ANNOUNCEMENT);
        clearDrag();
        return;
      }
      commitDrop(source, dragRef.current.target);
    };
    // The capture went while the drag was lifted: the session ends rather than waiting for a
    // release that will not come to this element.
    current.lost = () => {
      if (press.current === current && current.lifted) cancelSession();
    };
    // `touch-action` lets the page pan, so a still hold can still scroll. After the lift the pan
    // must not start: the browser would take the gesture and send `pointercancel` (#257). A
    // non-passive touchmove is the one place that can stop it.
    current.touch = (e) => {
      if (current.lifted && e.cancelable) e.preventDefault();
    };
    current.key = (e) => {
      if (e.key !== "Escape" || !current.lifted) return;
      e.preventDefault();
      e.stopPropagation();
      cancelSession();
    };
    press.current = current;
    window.addEventListener("touchmove", current.touch, { passive: false });
    window.addEventListener("keydown", current.key, true);
    element.addEventListener("lostpointercapture", current.lost);
    window.addEventListener("pointermove", current.move, { passive: false });
    window.addEventListener("pointerup", current.up);
    window.addEventListener("pointercancel", current.up);
    if (liftsOnHold(pointerType)) {
      current.timer = setTimeout(() => {
        current.timer = null;
        lift();
      }, TOUCH_HOLD_MS);
    }
  };

  /** The keyboard's target step for Up/Down: the nearest slot, then one place at a time. */
  const slotStep = (current: DropTarget | null, step: 1 | -1): DropTarget | null => {
    const slots = stackSlots(draftRef.current.layers.length);
    if (current?.kind !== "stackSlot") return (step > 0 ? slots[slots.length - 1] : slots[0]) ?? null;
    return cycleTarget(slots, current, step);
  };

  // The keyboard sensor: the arrows pick, Enter drops, Escape cancels. Capture, and stopped there,
  // so a key that steers the drag is not also a key the tile or the row under it acts on: the
  // Enter that drops must not reach the focused tile and lift it again.
  useEffect(() => {
    if (!drag.source || drag.mode !== "keyboard") return;
    const source = drag.source;
    const onKey = (event: KeyboardEvent) => {
      const current = dragRef.current;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setAnnounce(CANCEL_ANNOUNCEMENT);
        clearDrag();
        return;
      }
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        event.stopPropagation();
        guardKeyClick();
        commitDrop(source, current.target);
        return;
      }
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        if (source.kind !== "effect") return;
        event.preventDefault();
        event.stopPropagation();
        const next = cycleTarget(stageTargets(layout), current.target, event.key === "ArrowRight" ? 1 : -1);
        showDrag({ ...current, target: next });
        sayTarget(next);
        return;
      }
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        event.stopPropagation();
        const step = event.key === "ArrowUp" ? 1 : -1;
        const next =
          source.kind === "mask"
            ? cycleTarget(maskTargets(draftRef.current.layers), current.target, step)
            : slotStep(current.target, step);
        showDrag({ ...current, target: next });
        sayTarget(next);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // The names and the handlers read the draft and the desk from the render they began in; a
    // move re-renders the page, and only a new lift starts a new listener.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag.source, drag.mode, layout, devices]);

  // The session's loose ends, for the life of the builder: a window that loses focus or a tab
  // that goes to the background ends any press or drag (the pointer's release will not come
  // here), the next press or key ends a click guard, and leaving the page ends everything with
  // no setState and no add.
  const sessionRef = useRef({ cancel: cancelSession, end: endPress });
  useEffect(() => {
    sessionRef.current = { cancel: cancelSession, end: endPress };
  });
  useEffect(() => {
    const onBlur = () => sessionRef.current.cancel();
    const onHidden = () => {
      if (document.visibilityState === "hidden") sessionRef.current.cancel();
    };
    const onInput = () => {
      suppressClick.current = false;
    };
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pointerdown", onInput, true);
    window.addEventListener("keydown", onInput, true);
    return () => {
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pointerdown", onInput, true);
      window.removeEventListener("keydown", onInput, true);
      // Leaving mid-press or mid-drag: the capture goes, the timer stops, the window listeners
      // come off. Nothing is set and nothing is added: the release that follows finds no press.
      const held = press.current;
      if (held?.lifted) {
        try {
          held.element?.releasePointerCapture(held.pointerId);
        } catch {
          // Nothing left to release.
        }
      }
      sessionRef.current.end();
      orphanUp.current?.();
      if (suppressTimer.current !== null) clearTimeout(suppressTimer.current);
      dragRef.current = NO_DRAG;
      boxesRef.current = null;
    };
  }, []);

  // The drop boxes are read when a drag lifts, and again when the page scrolls or resizes under it
  // or when the insertion line changes the list's height; a pointer move reads none.
  const liftedMode = drag.source ? drag.mode : null;
  useEffect(() => {
    if (liftedMode !== "pointer" && liftedMode !== "touch") return;
    const reread = () => {
      boxesRef.current = readDropBoxes();
    };
    reread();
    window.addEventListener("scroll", reread, true);
    window.addEventListener("resize", reread);
    return () => {
      window.removeEventListener("scroll", reread, true);
      window.removeEventListener("resize", reread);
    };
  }, [liftedMode]);
  const insertAt =
    drag.source && drag.target
      ? drag.target.kind === "stackSlot"
        ? drag.target.at
        : drag.target.kind === "row"
          ? drag.target.index + 1
          : null
      : null;
  useEffect(() => {
    if (liftedMode !== "pointer" && liftedMode !== "touch") return;
    boxesRef.current = readDropBoxes();
  }, [liftedMode, insertAt]);

  /** A tile's click: a tap or a click adds the effect on top, whole canvas (spec §4). */
  const onTileAdd = (effect: string) => {
    if (suppressClick.current) {
      suppressClick.current = false;
      return;
    }
    void addEffect(effect);
  };

  const onTileLiftStart = (effect: EffectSummary, event: ReactPointerEvent<HTMLButtonElement>, asMask: boolean) => {
    beginPress(
      asMask
        ? { kind: "mask", effect: effect.id, name: effect.name }
        : { kind: "effect", effect: effect.id, name: effect.name },
      event,
    );
  };

  const onTileLiftKey = (effect: EffectSummary, event: ReactKeyboardEvent<HTMLButtonElement>, asMask: boolean) => {
    if (dragRef.current.source) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    event.preventDefault();
    beginKeyboard(
      asMask
        ? { kind: "mask", effect: effect.id, name: effect.name }
        : { kind: "effect", effect: effect.id, name: effect.name },
    );
  };

  /** The grip's press: a mouse or pen lifts on the move, a finger on the hold. */
  const onGripDown = (layer: StackLayer, event: ReactPointerEvent<HTMLButtonElement>) => {
    beginPress({ kind: "layer", id: layer.id, name: layer.name }, event);
  };

  /** Space or Enter on the grip lifts the row for the keyboard sensor. */
  const onGripKey = (layer: StackLayer, event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (dragRef.current.source) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    event.preventDefault();
    beginKeyboard({ kind: "layer", id: layer.id, name: layer.name });
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
    } catch (e) {
      notify(messageOf(e), "down");
    }
  };

  // ---- the list of effects for Choose effect ----------------------------------------

  /** Every effect a layer may take: a stack is never offered (D95.4.8). */
  const chooseList = useMemo(() => {
    return (effects ?? [])
      .filter((effect) => !effect.id.startsWith("stack:"))
      .map((effect) => ({ effect, match: matchEffect("", effect.name, effect.description) }))
      .filter((row): row is { effect: EffectSummary; match: NonNullable<typeof row.match> } => row.match !== null);
  }, [effects]);

  /** One row of the mask picker: choosing it gives the open layer this mask. */
  const maskRow = (effect: EffectSummary, index: number) => {
    const refusal = maskFor ? maskRefusal(draft.layers, maskFor, effect.id) : null;
    const current = maskFor ? draft.layers.find((layer) => layer.id === maskFor)?.mask?.effect === effect.id : false;
    return (
      <li key={effect.id}>
        <OptionRow
          id={`mask-option-${effect.id}`}
          active={index === maskActive}
          selected={current}
          disabled={refusal !== null}
          title={refusal ?? undefined}
          onSelect={() => {
            if (maskFor) void chooseMaskEffect(maskFor, effect.id);
          }}
          leading={<Thumb id={effect.id} size="sm" label={effect.name} />}
          name={effect.name}
          description={refusal ?? effect.description}
          trailing={isHtmlEffect(effect.id) ? <Chip html>HTML</Chip> : undefined}
        />
      </li>
    );
  };

  /** One row of the Choose-effect list: it gives the picked layer another effect. */
  const effectRow = (row: { effect: EffectSummary; match: NonNullable<ReturnType<typeof matchEffect>> }) => {
    const full = html > 0 && !isHtmlEffect(pickedLayer?.effect ?? "") && isHtmlEffect(row.effect.id);
    return (
      <li key={row.effect.id}>
        <button
          type="button"
          className="rd-add-row"
          role="option"
          aria-selected={pickedLayer?.effect === row.effect.id}
          onClick={() => {
            if (!chooseFor) return;
            void chooseEffect(chooseFor, row.effect.id);
            closeChoose();
          }}
        >
          <Thumb id={row.effect.id} size="sm" label={row.effect.name} />
          <span className="rd-add-text">
            <span className="rd-add-name">
              <Highlighted text={row.effect.name} ranges={row.match.ranges} />
            </span>
            <span className="rd-add-desc">{full ? "HTML layers are full" : row.effect.description}</span>
          </span>
          {isHtmlEffect(row.effect.id) && <Chip html>HTML</Chip>}
        </button>
      </li>
    );
  };

  // ---- rendering --------------------------------------------------------------------

  /** What the stage calls the target a drag is over, in the live region's own words. */
  const dragWords = drag.target
    ? dropTargetWords(drag.target, { layout, devices, layers: draft.layers })
    : null;

  /** The library, in whichever shape this width gives it. */
  const libraryPanel = (mode: LibraryMode) => (    <Panel className="rd-library-panel" title="Library">
      <Library
        mode={mode}
        effects={effects}
        layers={draft.layers}
        problem={addProblem}
        containerRef={libraryRef}
        onAdd={onTileAdd}
        onLiftStart={onTileLiftStart}
        onLiftKey={onTileLiftKey}
        onWriteCode={writeCode}
      />
    </Panel>
  );

  return (
    <div
      className={narrow ? "rd-page rd-stacks rd-stacks-phone" : "rd-page rd-stacks"}
      data-testid="stack-builder"
    >
      <PageTitle
        title={name}
        context={<span className="rd-stacks-context">Stack · {count} {count === 1 ? "layer" : "layers"}</span>}
        leading={
          <span ref={backRef}>
            <BackKey onClick={askLeave} />
          </span>
        }
      />
      <PageActions>
        {/* On a phone the strip carries one key: Save, as an icon with its name (D95.5.7). The
            word Unsaved and Revert are desktop keys: the phone's way out asks Save or Discard. */}
        {!narrow && dirty && <span className="mute rd-unsaved">Unsaved</span>}
        {!narrow && (
          <Button
            size="sm"
            variant="ghost"
            icon={<ArrowCounterClockwise size={14} />}
            disabled={!dirty || saveState === "saving"}
            onClick={revert}
          >
            Revert
          </Button>
        )}
        <Button
          className="rd-stack-strip-save"
          size="sm"
          variant="primary"
          icon={<FloppyDisk size={14} />}
          aria-label={narrow ? "Save" : undefined}
          title={narrow ? "Save the stack" : undefined}
          busy={saveState === "saving"}
          disabled={count === 0 || phase !== "ready" || loadError !== null}
          onClick={() => void save()}
        >
          <span className="rd-btn-text">Save</span>
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
            <Button size="sm" variant="danger" busy={saveState === "saving"} onClick={() => void replaceOther()}>
              Replace
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setClash(null)}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      {phase === "error" ? (
        <div className="rd-stack-body" data-library={libraryMode}>
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
          <div className="rd-stack-body" data-library={libraryMode}>
            {libraryMode === "rail" && libraryPanel("rail")}
            <div className="rd-stack-main">
              {saveError && (
                <ErrorPanel
                  title="The stack could not be saved"
                  summary="canvas-rgb could not save that stack. Nothing on the lights changed."
                  message={saveError}
                  onRetry={() => void save()}
                />
              )}

              <Panel
                className="rd-stack-stage"
                title="Preview"
                actions={
                  <StageTools
                    canUndo={canUndo(edit)}
                    canRedo={canRedo(edit)}
                    onUndo={undoNow}
                    onRedo={redoNow}
                    parts={parts}
                    onParts={setParts}
                  />
                }
              >
                <Stage
                  stack={draft}
                  layout={layout}
                  devices={devices}
                  picked={pickedLayer}
                  parts={parts}
                  onSelect={pick}
                  onAreaChange={(area, run, hold) => pickedLayer && patch(pickedLayer.id, { area }, run, hold)}
                  onAreaChangeEnd={() => setEdit((history) => endRun(history))}
                  onDragActive={(active) => {
                    areaDragging.current = active;
                  }}
                  onDelete={remove}
                  onDuplicate={duplicate}
                  onMakeRect={makeRect}
                  drop={
                    drag.source?.kind === "effect" || (drag.source?.kind === "mask" && drag.target)
                      ? { target: drag.target, words: dragWords }
                      : null
                  }
                  dragging={drag.source !== null}
                  schema={pickedLayer ? (schemas[pickedLayer.effect] ?? null) : null}
                  onPropChange={(id, key, value, run, hold) =>
                    setDraft(
                      (current) => ({
                        ...current,
                        layers: current.layers.map((layer) =>
                          layer.id === id ? { ...layer, props: { ...layer.props, [key]: value } } : layer,
                        ),
                      }),
                      run,
                      hold,
                    )
                  }
                  onPropChangeEnd={() => setEdit((history) => endRun(history))}
                />
                {draftState === "off" ? (
                  <div className="rd-stage-foot">
                    <span className="note">
                      {previewError
                        ? `The preview stopped: ${previewError}`
                        : draftError
                          ? `Try on the lights failed: ${draftError}`
                          : feedProblem
                            ? `The preview is not showing: ${feedProblem}`
                            : count === 0
                              ? "An empty stack runs nothing; the lights keep their effect."
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
                        icon={<Play size={15} />}
                        aria-label="Try on the lights"
                        disabled={count === 0}
                        title={count === 0 ? "Add a layer first" : undefined}
                        onClick={() => void startDraft()}
                      >
                        <span className="rd-btn-text">Try on the lights</span>
                      </Button>
                    </span>
                  </div>
                ) : (
                  <div className="rd-stage-foot rd-draft-foot" data-state={draftState} role="status">
                    <span className="eff-draft-mark" data-state={draftState} aria-hidden="true" />
                    <span className="note">
                      {draftState === "starting"
                        ? "Starting the draft"
                        : draftState === "running"
                          ? "Draft on the lights"
                          : draftState === "expiring"
                            ? `Draft on the lights, expiring in ${expiresIn}s`
                            : draftState === "restoring"
                              ? "Restoring the previous effect"
                              : "The previous effect could not be restored."}
                    </span>
                    <span className="row-wrap">
                      {draftState === "restore-error" ? (
                        <Button size="sm" icon={<Lightbulb size={14} />} onClick={() => void stopDraft()}>
                          Retry
                        </Button>
                      ) : (
                        <>
                          <Button size="sm" variant="ghost" busy={draftState === "restoring"} onClick={() => void stopDraft()}>
                            Stop
                          </Button>
                          <Button
                            size="sm"
                            variant={leaving ? "default" : "primary"}
                            busy={saveState === "saving"}
                            disabled={count === 0}
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

              {libraryMode === "tray" && libraryPanel("tray")}

              {narrow && (
                <div className="rd-phone-tabs">
                  <Segmented<"layers" | "library">
                    label="What shows under the stage"
                    value={phoneTab}
                    options={[
                      { value: "layers", label: "Layers" },
                      { value: "library", label: "Library" },
                    ]}
                    onChange={setPhoneTab}
                  />
                </div>
              )}

              {narrow && phoneTab === "library" && libraryPanel("tab")}

              {(!narrow || phoneTab === "layers") && (
              <Panel
                className="rd-layers"
                title="Layers"
                actions={
                  <span className="mute fig" style={{ fontSize: "var(--t14)" }}>
                    {count} / {MAX_LAYERS}
                  </span>
                }
              >
                {count === 0 ? (
                  <div className="rd-layers-empty">
                    <p>No layers yet. A stack is a list of effects drawn one over the other, bottom to top.</p>
                    <ul className="rd-how-list">
                      <li>The bottom layer is painted first, then each layer over it.</li>
                      <li>A layer paints on its canvas, a device, a zone or a rectangle.</li>
                      <li>Blend and opacity mix it over what is under it.</li>
                    </ul>
                    <p className="rd-layers-empty-hint">Pick an effect from the library to start.</p>
                  </div>
                ) : (
                  <ol className="layer-list" aria-label="Layers, top first">
                    {[...draft.layers].reverse().map((layer) => {
                      const status = statuses[layer.id];
                      const words = statusWords(status?.status, status?.last_ms);
                      const index = draft.layers.indexOf(layer);
                      return (
                        <Fragment key={layer.id}>
                        {insertAt === index + 1 && <li className="rd-insert-line" aria-hidden="true" />}
                        <li
                          className="layer-row"
                          data-layer-id={layer.id}
                          data-layer-index={index}
                          data-selected={layer.id === picked ? "true" : "false"}
                          data-hidden={layer.visible ? "false" : "true"}
                          data-dragging={drag.source?.kind === "layer" && drag.source.id === layer.id ? "true" : "false"}
                          data-mask-target={drag.target?.kind === "maskLayer" && drag.target.id === layer.id ? "true" : undefined}
                          tabIndex={0}
                          aria-current={layer.id === picked ? "true" : undefined}
                          onClick={() => pick(layer.id)}
                          onKeyDown={(event) => {
                            // A row is in the tab order so the picked layer can be changed from
                            // the keyboard; the grip's own keys reorder it. Delete removes the
                            // row's layer, as it does on the stage (#257).
                            if (event.target !== event.currentTarget) return;
                            if (event.key === "Enter" || event.key === " ") {
                              event.preventDefault();
                              pick(layer.id);
                              return;
                            }
                            if (event.key === "Delete" || event.key === "Backspace") {
                              event.preventDefault();
                              remove(layer.id);
                            }
                          }}
                        >
                          <button
                            type="button"
                            className="layer-grip"
                            aria-label={`Reorder ${layer.name}`}
                            title="Drag to reorder, or press Space; Alt+↑/↓ on the keyboard"
                            onPointerDown={(event) => onGripDown(layer, event)}
                            onKeyDown={(event) => {
                              if (dragRef.current.source) return;
                              if (event.altKey) {
                                if (event.key === "ArrowUp" && index < count - 1) {
                                  event.preventDefault();
                                  reorder(layer.id, index + 1);
                                } else if (event.key === "ArrowDown" && index > 0) {
                                  event.preventDefault();
                                  reorder(layer.id, index - 1);
                                }
                                return;
                              }
                              onGripKey(layer, event);
                            }}
                          >
                            <DotsSixVertical size={16} aria-hidden="true" />
                          </button>
                          <span className="layer-thumbs">
                            <Thumb
                              id={layer.effect}
                              size="sm"
                              label={layerLabel(schemas, effects, layer.effect) ?? "Effect"}
                            />
                            {layer.mask && (
                              <MaskChip
                                layerName={layer.name}
                                effect={layer.mask.effect}
                                label={layerLabel(schemas, effects, layer.mask.effect) ?? "Effect"}
                                onChange={(anchor) => {
                                  pick(layer.id);
                                  openMaskPicker(layer.id, anchor);
                                }}
                                onRemove={() => removeMask(layer.id)}
                              />
                            )}
                          </span>
                          <span className="layer-text">
                            {renaming === layer.id ? (
                              <div onClick={(event) => event.stopPropagation()}>
                                <NameField
                                  value={renameText}
                                  label={`Rename ${layer.name}`}
                                  maxLength={40}
                                  invalid={renameProblem !== null}
                                  describedBy="layer-name-error"
                                  onChange={(value) => {
                                    setRenameText(value);
                                    setRenameProblem(null);
                                  }}
                                  onEnter={commitRename}
                                  onEscape={() => setRenaming(null)}
                                />
                                {renameProblem && (
                                  <p className="error-line" id="layer-name-error" role="alert">
                                    {renameProblem}
                                  </p>
                                )}
                              </div>
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
                          {!layer.mask && (
                            <Button
                              size="sm"
                              variant="ghost"
                              icon={<CircleHalf size={15} />}
                              aria-label={`Add a mask to ${layer.name}`}
                              title={`Add a mask to ${layer.name}`}
                              onClick={(event) => {
                                event.stopPropagation();
                                pick(layer.id);
                                openMaskPicker(layer.id, event.currentTarget);
                              }}
                            >
                              <span className="rd-btn-text">Add mask</span>
                            </Button>
                          )}
                          <IconButton
                            label={layer.visible ? `Hide ${layer.name}` : `Show ${layer.name}`}
                            icon={layer.visible ? <Eye size={16} /> : <EyeSlash size={16} />}
                            pressed={!layer.visible}
                            onClick={() =>
                              setDraft((current) => ({ ...current, layers: toggleLayer(current.layers, layer.id) }))
                            }
                          />
                        </li>
                        </Fragment>
                      );
                    })}
                    {insertAt === 0 && <li className="rd-insert-line" aria-hidden="true" />}
                  </ol>
                )}
              </Panel>
              )}

              {narrow && pickedLayer && (
                <section className="panel rd-layer-summary" aria-label="The picked layer">
                  <div className="layer-edit-head">
                    <div className="layer-edit-title">
                      <Thumb id={pickedLayer.effect} size="md" label={layerLabel(schemas, effects, pickedLayer.effect) ?? "Effect"} />
                      <div>
                        <h2>{pickedLayer.name}</h2>
                        <span className="mute" aria-busy={layerLabel(schemas, effects, pickedLayer.effect) === null || undefined}>
                          {layerLabel(schemas, effects, pickedLayer.effect) ?? <InlineSkeleton width="10ch" />} · {areaName(pickedLayer.area, layout, devices)}
                        </span>
                      </div>
                    </div>
                    <span ref={tuneRef}>
                      <Button
                        size="sm"
                        icon={<SlidersHorizontal size={15} />}
                        aria-label={`Tune ${pickedLayer.name}`}
                        onClick={openSheet}
                      >
                        Tune
                      </Button>
                    </span>
                  </div>
                </section>
              )}
            </div>

            <div
              ref={narrow ? sheetRef : undefined}
              className={narrow ? "effect-sheet rd-layer-sheet" : "rd-stack-side"}
              data-open={narrow ? (sheetOpen ? "true" : "false") : undefined}
              role={narrow ? "dialog" : undefined}
              aria-modal={narrow ? true : undefined}
              aria-label={narrow && pickedLayer ? `Tune ${pickedLayer.name}` : undefined}
              tabIndex={narrow ? -1 : undefined}
              hidden={narrow ? !sheetOpen : undefined}
            >
              {narrow && (
                <div className="effect-sheet-head">
                  <IconButton label="Back to the stack" icon={<CaretLeft size={18} />} onClick={closeSheet} />
                  <span>{pickedLayer ? `Tune ${pickedLayer.name}` : "Tune"}</span>
                </div>
              )}
              {pickedLayer ? (
                <Panel className="rd-layer-edit">
                  <div className="layer-edit-head">
                    <div className="layer-edit-title">
                      <Thumb id={pickedLayer.effect} size="md" label={layerLabel(schemas, effects, pickedLayer.effect) ?? "Effect"} />
                      <div>
                        <h2>{pickedLayer.name}</h2>
                        <span className="mute" aria-busy={layerLabel(schemas, effects, pickedLayer.effect) === null || undefined}>
                          {layerLabel(schemas, effects, pickedLayer.effect) ?? <InlineSkeleton width="10ch" />} · layer {draft.layers.indexOf(pickedLayer) + 1} of{" "}
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
                        loading={
                          effects === null ||
                          (schemas[pickedLayer.effect] === undefined &&
                            !effects.some((effect) => effect.id === pickedLayer.effect))
                        }
                        value={pickedLayer.effect}
                        options={[
                          // A layer whose effect is gone (or is a stack, which the list never offers)
                          // keeps its own entry, so the select says what the layer has.
                          ...((effects ?? []).some((effect) => effect.id === pickedLayer.effect)
                            ? []
                            : [
                                {
                                  value: pickedLayer.effect,
                                  label: layerLabel(schemas, effects, pickedLayer.effect) ?? "Effect",
                                },
                              ]),
                          ...(effects ?? [])
                            .filter((effect) => !effect.id.startsWith("stack:"))
                            .map((effect) => ({
                              value: effect.id,
                              label: html && !isHtmlEffect(pickedLayer.effect) && isHtmlEffect(effect.id)
                                ? `${effect.name} (HTML layers full)`
                                : effect.name,
                            })),
                        ]}
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
                          onChange={(next) => patch(pickedLayer.id, { opacity: next }, `opacity:${pickedLayer.id}`)}
                          onCommit={() => setEdit((history) => endRun(history))}
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
                      <span className="layer-area-words">
                        <span>{areaName(pickedLayer.area, layout, devices)}</span>
                        {areaPosition(pickedLayer.area) && (
                          <span className="mute fig">{areaPosition(pickedLayer.area)}</span>
                        )}
                      </span>
                    </div>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={(event) => {
                        areaOpener.current = event.currentTarget;
                        setPickerFor(pickedLayer.id);
                      }}
                    >
                      Change area
                    </Button>
                  </div>

                  {schemas[pickedLayer.effect] === null || statuses[pickedLayer.id]?.status === "missing_effect" ? (
                    <div className="rd-gone">
                      <p className="error-line">This effect is gone. Choose another.</p>
                      <span ref={chooseRef}>
                        <Button
                          size="sm"
                          icon={<Plus size={15} />}
                          aria-label={`Choose effect for ${pickedLayer.name}`}
                          onClick={() => setChooseFor(pickedLayer.id)}
                        >
                          Choose effect
                        </Button>
                      </span>
                    </div>
                  ) : schemas[pickedLayer.effect] === undefined ? (
                    <LoadFrame
                      state="loading"
                      label="Loading the effect's properties"
                      shape={
                        // The shape of the property rows that come: a label on the left, its control on
                        // the right, one row each (a colour, a slider, a choice).
                        <div className="rd-props-shape">
                          {[0.32, 0.26, 0.36].map((label, i) => (
                            <div className="rd-props-row" key={i}>
                              <Skeleton width={`${label * 100}%`} height={14} />
                              <Skeleton width={i === 0 ? "34%" : "52%"} height={i === 0 ? 24 : 18} radius={i === 0 ? 6 : 9} />
                            </div>
                          ))}
                        </div>
                      }
                    />
                  ) : (
                    <PropertyControls
                      schema={schemas[pickedLayer.effect] as PropertySchema}
                      values={layerValues(schemas[pickedLayer.effect], pickedLayer.props)}
                      onChange={(key, value) =>
                        setDraft(
                          (current) => ({
                            ...current,
                            layers: current.layers.map((layer) =>
                              layer.id === pickedLayer.id ? { ...layer, props: { ...layer.props, [key]: value } } : layer,
                            ),
                          }),
                          `props:${pickedLayer.id}:${key}`,
                        )
                      }
                    />
                  )}

                  {/* The mask (#256): another effect, run over this layer's area, whose picture
                      decides where the layer shows. */}
                  <div className="rd-mask" aria-label="Mask">
                    <div className="rd-mask-head">
                      <span className="rd-mask-title">
                        <CircleHalf size={15} aria-hidden="true" /> Mask
                      </span>
                      {pickedLayer.mask ? (
                        <Button
                          size="sm"
                          variant="ghost"
                          icon={<X size={14} />}
                          onClick={() => removeMask(pickedLayer.id)}
                        >
                          Remove
                        </Button>
                      ) : (
                        <Button
                          size="sm"
                          icon={<Plus size={15} />}
                          onClick={(event) => openMaskPicker(pickedLayer.id, event.currentTarget)}
                        >
                          Add mask
                        </Button>
                      )}
                    </div>
                    {pickedLayer.mask ? (
                      <>
                        <p className="rd-mask-note">
                          The mask runs over this layer&apos;s area every frame; its picture decides how much of the
                          layer shows.
                        </p>
                        <div className="layer-fields">
                          <div className="layer-field wide">
                            <span>Mask effect</span>
                            <Select
                              label="Mask effect"
                              value={pickedLayer.mask.effect}
                              options={[
                                ...((effects ?? []).some((effect) => effect.id === pickedLayer.mask?.effect)
                                  ? []
                                  : [
                                      {
                                        value: pickedLayer.mask.effect,
                                        label:
                                          schemas[pickedLayer.mask.effect] === null
                                            ? "Missing effect"
                                            : layerLabel(schemas, effects, pickedLayer.mask.effect) ?? "Effect",
                                      },
                                    ]),
                                ...(effects ?? [])
                                  .filter((effect) => !effect.id.startsWith("stack:"))
                                  .map((effect) => ({
                                    value: effect.id,
                                    label:
                                      html > 0 &&
                                      !isHtmlEffect(pickedLayer.mask?.effect ?? "") &&
                                      isHtmlEffect(effect.id) &&
                                      htmlLayers(draft.layers) >= MAX_HTML_LAYERS
                                        ? `${effect.name} (HTML layers full)`
                                        : effect.name,
                                  })),
                              ]}
                              onChange={(next) => void chooseMaskEffect(pickedLayer.id, next)}
                            />
                            {maskProblem && (
                              <p className="error-line" role="alert">
                                {maskProblem}
                              </p>
                            )}
                          </div>
                          <div className="layer-field wide">
                            <span>Reads</span>
                            <Segmented<MaskSource>
                              label="What the mask reads"
                              value={pickedLayer.mask.source}
                              options={[
                                { value: "luma", label: "Brightness" },
                                { value: "alpha", label: "Strongest colour" },
                              ]}
                              onChange={(source) => patchMaskDraft(pickedLayer.id, { source })}
                            />
                            <span className="rd-mask-hint">
                              Strongest colour: each pixel shows as much of the layer as its strongest colour
                              channel is bright.
                            </span>
                          </div>
                          <div className="layer-field wide layer-field-row">
                            <span>Invert</span>
                            <Toggle
                              label="Invert the mask"
                              checked={pickedLayer.mask.invert}
                              onChange={(invert) => patchMaskDraft(pickedLayer.id, { invert })}
                            />
                          </div>
                          <div className="layer-field wide">
                            <span>Amount</span>
                            <div className="field-slider">
                              <Slider
                                label="Mask amount"
                                min={0}
                                max={100}
                                value={pickedLayer.mask.amount}
                                onChange={(amount) =>
                                  patchMaskDraft(pickedLayer.id, { amount }, `mask-amount:${pickedLayer.id}`)
                                }
                                onCommit={() => setEdit((history) => endRun(history))}
                              />
                              <NumberField
                                label="Mask amount value"
                                min={0}
                                max={100}
                                value={pickedLayer.mask.amount}
                                onCommit={(amount) => patchMaskDraft(pickedLayer.id, { amount })}
                              />
                            </div>
                          </div>
                        </div>
                        {schemas[pickedLayer.mask.effect] === null ? (
                          <p className="error-line">
                            This mask effect is gone. Choose another, or remove the mask.
                          </p>
                        ) : schemas[pickedLayer.mask.effect] === undefined ? (
                          <LoadFrame
                            state="loading"
                            label="Loading the mask's properties"
                            shape={
                              <div className="rd-props-shape">
                                {[0.32, 0.26, 0.36].map((label, i) => (
                                  <div className="rd-props-row" key={i}>
                                    <Skeleton width={`${label * 100}%`} height={14} />
                                    <Skeleton
                                      width={i === 0 ? "34%" : "52%"}
                                      height={i === 0 ? 24 : 18}
                                      radius={i === 0 ? 6 : 9}
                                    />
                                  </div>
                                ))}
                              </div>
                            }
                          />
                        ) : (
                          <>
                            <h3 className="rd-mask-sub">
                              {layerLabel(schemas, effects, pickedLayer.mask.effect)} settings
                            </h3>
                            <PropertyControls
                              schema={schemas[pickedLayer.mask.effect] as PropertySchema}
                              values={maskValues(schemas[pickedLayer.mask.effect], pickedLayer.mask)}
                              onChange={(key, value) => patchMaskProps(pickedLayer.id, key, value)}
                            />
                          </>
                        )}
                      </>
                    ) : (
                      <p className="rd-mask-note">
                        A mask shapes where this layer shows: another effect runs over the layer&apos;s area and its
                        picture decides the coverage. A flame over warm white is the classic.
                      </p>
                    )}
                  </div>
                  {(statuses[pickedLayer.id]?.status === "held" || statuses[pickedLayer.id]?.status === "faulted") && (
                    <p className="layer-warn" role="status">
                      This layer is slower than the frame; the rest of the stack keeps its rate.
                    </p>
                  )}
                  {statuses[pickedLayer.id]?.status === "nested_stack" && (
                    <p className="layer-warn" role="status">
                      A stack cannot contain a stack. This layer does not run; the rest of the stack does.
                    </p>
                  )}
                  {layerProblem && (
                    <p className="error-line" role="alert">
                      {layerProblem}
                    </p>
                  )}
                </Panel>
              ) : (
                <Panel className="rd-layer-edit rd-layer-none" title="No layer picked">
                  <p className="dim">Add a layer to tune it.</p>
                </Panel>
              )}

              {!narrow && (
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
                        setPresetProblem("Save the stack before saving a preset");
                        return;
                      }
                      setPresetProblem(null);
                      setPresetName("");
                    }}
                  >
                    + Save current
                  </Chip>
                </div>
                {presetProblem && (
                  <p className="error-line" role="alert">
                    {presetProblem}
                  </p>
                )}
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
              )}
            </div>
          </div>
        </LoadFrame>
      )}

      <Popover
        anchor={chooseRef}
        open={chooseFor !== null}
        onClose={closeChoose}
        label="Choose effect"
        className="rd-add-layer rd-choose-effect"
      >
        <div className="stack rd-add-layer-body">
          <ul className="rd-add-list" role="listbox" aria-label="Effects to choose">
            {chooseList.map((row) => effectRow(row))}
          </ul>
        </div>
      </Popover>

      <Popover
        anchor={maskAnchor}
        open={maskFor !== null}
        onClose={closeMaskPicker}
        label="Choose a mask"
        className="rd-add-layer rd-mask-picker"
      >
        <div className="stack rd-add-layer-body">
          <p className="rd-mask-picker-note">
            A mask&apos;s picture decides where the layer shows. These six read best as masks.
          </p>
          <ul
            ref={maskListRef}
            className="rd-add-list"
            role="listbox"
            aria-label="Effects to use as a mask"
            tabIndex={-1}
            aria-activedescendant={
              maskChoices[maskActive] ? `mask-option-${maskChoices[maskActive].id}` : undefined
            }
            onKeyDown={onMaskKey}
          >
            <li className="rd-add-group" aria-hidden="true">
              Good as a mask
            </li>
            {maskList.leads.map((effect, index) => maskRow(effect, index))}
            <li className="rd-add-group" aria-hidden="true">
              More
            </li>
            {maskList.rest.map((effect, index) => maskRow(effect, maskList.leads.length + index))}
          </ul>
        </div>
      </Popover>

      {pickerFor && (
        <StackAreaPicker
          layer={draft.layers.find((layer) => layer.id === pickerFor) ?? draft.layers[0]!}
          layout={layout}
          devices={devices}
          onUse={(area) => applyArea(area)}
          onClose={closePicker}
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

      {drag.source && drag.point && (
        <div
          className="rd-drag-ghost"
          data-kind={drag.source.kind}
          style={{ left: `${drag.point.x + 12}px`, top: `${drag.point.y + 12}px` }}
          aria-hidden="true"
        >
          {drag.source.kind === "layer" ? (
            <span className="rd-drag-ghost-grip">
              <DotsSixVertical size={16} />
            </span>
          ) : (
            <Thumb id={drag.source.effect} size="sm" label={drag.source.name} live />
          )}
          <span className="rd-drag-ghost-name">{drag.source.name}</span>
          {drag.source.kind === "mask" && <span className="rd-drag-ghost-tag">mask</span>}
        </div>
      )}

      {/* Why a mask cannot land here: the words follow the pointer, and the live region says
          them too. */}
      {drag.source && drag.point && drag.hint && (
        <p className="rd-drag-hint" style={{ left: `${drag.point.x + 12}px`, top: `${drag.point.y + 48}px` }} aria-hidden="true">
          {drag.hint}
        </p>
      )}

      <p className="sr-only rd-live" role="status" aria-live="polite" aria-atomic="true">
        {announce}
      </p>
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

function blendOf(blend: BlendMode): string {
  return BLENDS.find((entry) => entry.value === blend)?.label ?? "Normal";
}
