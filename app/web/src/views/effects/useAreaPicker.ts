"use client";

// The area picker's open/close state (#257 phase 4), moved out of StackBuilder.tsx: the dialog
// hangs over the builder, and closing it puts focus back on the key that opened it.

import { useRef, useState } from "react";
import type { LayerArea } from "@/proto/LayerArea";

export function useAreaPicker(patch: (id: string, change: Partial<{ area: LayerArea }>) => void) {
  const [pickerFor, setPickerFor] = useState<string | null>(null);
  /** The key that opened the area picker: focus returns to it on close. */
  const opener = useRef<HTMLElement | null>(null);

  const open = (id: string, element: HTMLElement) => {
    opener.current = element;
    setPickerFor(id);
  };

  const close = () => {
    setPickerFor(null);
    const element = opener.current;
    if (element) setTimeout(() => element.focus(), 0);
  };

  const apply = (area: LayerArea) => {
    if (pickerFor) patch(pickerFor, { area });
    close();
  };

  return { pickerFor, open, close, apply };
}
