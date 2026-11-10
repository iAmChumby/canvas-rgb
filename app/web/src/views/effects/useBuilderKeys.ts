"use client";

// The builder's global keys (#257 phase 4), moved out of StackBuilder.tsx unchanged: Ctrl+Z,
// Ctrl+Shift+Z and Ctrl+Y take a step back and again, and Ctrl+D copies the picked layer. A field,
// a select, a popover and any open surface keeps its own keys.

import { useEffect } from "react";

export function useBuilderKeys({
  draft,
  sheetOpen,
  pickerFor,
  leaving,
  naming,
  maskFor,
  dragSource,
  pickedRef,
  undoNow,
  redoNow,
  duplicate,
}: {
  draft: unknown;
  sheetOpen: boolean;
  pickerFor: string | null;
  leaving: boolean | string;
  naming: boolean;
  maskFor: string | null;
  dragSource: unknown;
  pickedRef: { current: string | null };
  undoNow: () => void;
  redoNow: () => void;
  duplicate: (id: string) => void;
}) {
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
      if (sheetOpen || pickerFor || leaving || naming || maskFor || dragSource) return;
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
  }, [draft, sheetOpen, pickerFor, leaving, naming, maskFor, dragSource]);
}
