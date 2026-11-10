"use client";

// The builder's save and its unsaved-changes guard (#257 phase 4): Save writes the stack and
// activates it, a taken name asks to replace, Revert restores the loaded stack, and leaving with
// changes goes through the prompt and the history guard. Moved out of StackBuilder.tsx unchanged.

import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import {
  cancelNavigation,
  continueNavigation,
  hasPendingNavigation,
  setNavigationGuard,
} from "@/lib/navigationGuard";
import { refreshAll } from "@/lib/store";
import { layersForServer, stackNameProblem } from "@/views/effects/stackDraft";
import { nameOf } from "@/views/effects/layerText";
import type { Stack } from "@/proto/Stack";

type LeaveReason = "back" | "navigation" | "close";

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function useSaveAndLeave({
  isNew,
  name,
  nameDraft,
  setNameDraft,
  nameRef,
  otherStacks,
  setOtherStacks,
  draftRef,
  base,
  setBase,
  resetDraft,
  pick,
  count,
  dirty,
  setSheetOpen,
  onClose,
  onName,
  onWriteCode,
  endDraft,
  loadPresets,
  backRef,
  tuneRef,
  libraryRef,
  promptRef,
}: {
  isNew: boolean;
  name: string;
  nameDraft: string;
  setNameDraft: (value: string) => void;
  nameRef: { current: string };
  otherStacks: { id: string; name: string }[];
  setOtherStacks: (update: (current: { id: string; name: string }[]) => { id: string; name: string }[]) => void;
  draftRef: { current: Stack };
  base: Stack | null;
  setBase: (stack: Stack) => void;
  resetDraft: (next: Stack) => void;
  pick: (id: string | null) => void;
  count: number;
  dirty: boolean;
  setSheetOpen: (open: boolean) => void;
  onClose: () => void;
  onName: (name: string) => void;
  onWriteCode?: () => void;
  endDraft: () => void;
  loadPresets: (id: string) => void;
  backRef: RefObject<HTMLSpanElement | null>;
  tuneRef: RefObject<HTMLSpanElement | null>;
  libraryRef: RefObject<HTMLDivElement | null>;
  promptRef: RefObject<HTMLDivElement | null>;
}) {
  const [saveState, setSaveState] = useState<"idle" | "saving">("idle");
  const [saveError, setSaveError] = useState<string | null>(null);
  /** The stack is saved but would not start: the id that was saved and the daemon's reason. Not a
   *  save error, so the banner says "Saved" and offers Start again, not a second write. */
  const [startFailure, setStartFailure] = useState<{ id: string; message: string } | null>(null);
  const [starting, setStarting] = useState(false);
  const [clash, setClash] = useState<string | null>(null);
  const [naming, setNaming] = useState(false);
  const [namingError, setNamingError] = useState<string | null>(null);
  const [leaving, setLeaving] = useState<LeaveReason | false>(false);
  /** A leave the unsaved prompt asked for, carried out once the new stack has a name. */
  const leaveAfterName = useRef<(() => void) | null>(null);
  /** The name a new stack was saved under: the guard entry the builder lands on takes it. */
  const pendingName = useRef<string | null>(null);
  /** The code flow was asked for from the library: it opens once a dirty builder has left. */
  const pendingCode = useRef(false);
  /** A Save or Discard that will leave the builder: the guard entry is not popped separately. */
  const pendingLeave = useRef(false);
  const allowBack = useRef(false);
  const sentinelPushed = useRef(false);
  /** True when this builder was opened from another page, so there are two entries behind the
   *  prompt's own sentinel: the page and the guard. A direct load has none. */
  const openedInApp = useRef(
    typeof window !== "undefined" && window.history.state?.canvasRgbStack !== undefined,
  );

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

  /** Keep editing: the leave prompt closes, and focus goes back where it was. */
  const keepEditing = useCallback(() => {
    pendingCode.current = false;
    setLeaving(false);
    cancelNavigation();
  }, []);

  const askLeave = () => {
    if (dirty) setLeaving("close");
    else onClose();
  };

  /** The library's "New from code" tile: the code flow, once a dirty builder has left. */
  const writeCode = () => {
    if (dirty) {
      pendingCode.current = true;
      setLeaving("close");
      return;
    }
    onWriteCode?.();
  };

  // The leave prompt is a modal over the builder and over the sheet: it takes focus, Tab stays
  // inside it, and Escape is Keep editing. Focus goes back where it was when it closes.
  const leaveWasOpen = useRef(false);
  const leaveOpener = useRef<HTMLElement | null>(null);
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
  }, [leaving, backRef, tuneRef, libraryRef, promptRef]);

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
  }, [leaving, keepEditing, promptRef]);

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
      const problem = stackNameProblem(wanted, otherStacks.filter((entry) => !base || entry.id !== base.id).map((entry) => entry.name));
      if (problem) {
        setSaveError(problem);
        return;
      }
    }
    // A save that will leave the builder must not also pop its guard entry.
    pendingLeave.current = then !== undefined;
    setSaveState("saving");
    setSaveError(null);
    setStartFailure(null);
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
    endDraft();
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
      setStartFailure({ id: saved.id, message: messageOf(e) });
      // Save and leave: the builder closes, and its banner with it, so the note says what the banner
      // would have said. It stays until it is dismissed.
      if (then) notify(`${nameOf(saved.id)} is saved, but it did not start: ${messageOf(e)}`, "warn");
    }
    then?.();
  };

  /** Start again: the stack is already saved, so only the activation is sent once more. */
  const startAgain = async () => {
    const failed = startFailure;
    // An unsaved edit: the saved stack is no longer what is on screen, so there is nothing to start.
    if (!failed || starting || dirty || failed.id !== base?.id) return;
    setStarting(true);
    try {
      await api.activateEffect(failed.id, { props: {}, preset: null });
      await refreshAll();
      setStartFailure(null);
      notify(`${nameOf(failed.id)} is running`);
    } catch (e) {
      setStartFailure({ id: failed.id, message: messageOf(e) });
    } finally {
      setStarting(false);
    }
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
    const problem = stackNameProblem(wanted, otherStacks.filter((entry) => !base || entry.id !== base.id).map((entry) => entry.name));
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

  /** The Close and Cancel keys: the prompt closes without a name, and a leave that waited for one is dropped. */
  const cancelNaming = () => {
    leaveAfterName.current = null;
    setNaming(false);
  };

  /** Escape in the prompt only closes it, as it did before #257's move: an owed leave stays owed. */
  const escapeNaming = () => setNaming(false);

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
    setStartFailure(null);
    setClash(null);
  };

  /** The leave prompt's "Discard": carry out the way out it was asked about. */
  const discardLeave = () => {
    const reason: LeaveReason = leaving || "close";
    setLeaving(false);
    goAfterLeave(reason);
  };

  /** The leave prompt's "Save": save, then carry out the way out it was asked about. */
  const saveLeave = () => {
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

  return {
    save,
    replaceOther,
    nameTheStack,
    cancelNaming,
    escapeNaming,
    revert,
    askLeave,
    writeCode,
    keepEditing,
    discardLeave,
    saveLeave,
    saveState,
    saveError,
    setSaveError,
    // Hidden, not dropped, while the draft has unsaved edits: Start again would run the saved stack,
    // not what is on screen. Undo back to the saved stack and it is offered again; Save starts the edit.
    // And only for the stack it was about: the builder can load another stack while mounted.
    startFailure: dirty || startFailure?.id !== base?.id ? null : startFailure,
    starting,
    startAgain,
    clash,
    setClash,
    naming,
    namingError,
    setNamingError,
    setNameDraft,
    leaving,
    setLeaving,
    pendingName,
  };
}
