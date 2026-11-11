"use client";

// The phone layer sheet's focus (#257 phase 4), moved out of StackBuilder.tsx unchanged: Tab stays
// inside the sheet, Escape closes it, opening it takes the first key and closing it gives focus
// back to Tune (or the library), and a resize to desktop lands focus on the picked layer's row.

import { useCallback, useEffect, useRef, type RefObject } from "react";
import { useFocusTrap } from "@/components/useFocusTrap";

/** What Tab reaches inside a dialog, in document order. */
const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function useSheetFocus({
  sheetOpen,
  pickerFor,
  leaving,
  setSheetOpen,
  phoneTab,
  setPhoneTab,
  narrow,
  picked,
  sheetRef,
  tuneRef,
  libraryRef,
}: {
  sheetOpen: boolean;
  pickerFor: string | null;
  leaving: boolean | string;
  setSheetOpen: (open: boolean) => void;
  phoneTab: "layers" | "library";
  setPhoneTab: (tab: "layers" | "library") => void;
  narrow: boolean;
  picked: string | null;
  sheetRef: RefObject<HTMLDivElement | null>;
  tuneRef: RefObject<HTMLSpanElement | null>;
  libraryRef: RefObject<HTMLDivElement | null>;
}) {
  // The sheet's own keys: Tab stays inside it while it is open, and Escape closes it. While the
  // area picker or the leave prompt is over the sheet, that dialog owns the keys and the sheet's
  // trap steps aside. Tab from outside the sheet comes back in rather than leaving the keyboard
  // on the page behind it. The trap is the palette's own, at the sheet's legacy placement: a
  // bubble listener on document.
  const closeSheet = useCallback(() => setSheetOpen(false), [setSheetOpen]);
  useFocusTrap({ panelRef: sheetRef, onEscape: closeSheet, capture: false, enabled: sheetOpen && !pickerFor && !leaving });

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
  }, [sheetOpen, setPhoneTab, sheetRef, tuneRef, libraryRef]);

  // The library tab opened for a fallback focus: its first tile takes the keyboard.
  useEffect(() => {
    if (!wantLibraryFocus.current || phoneTab !== "library") return;
    wantLibraryFocus.current = false;
    libraryRef.current?.querySelector<HTMLElement>("[data-lib-tile]")?.focus();
  }, [phoneTab, libraryRef]);

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
}
