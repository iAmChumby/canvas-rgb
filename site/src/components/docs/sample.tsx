"use client";

import { useEffect, useState } from "react";
import { Segmented } from "@editor/components/Segmented";

/**
 * The cURL/JS and Shell/JSON switches. Each page has its own scope, so a choice on one route
 * never leaks into another. The code well listens for the choice with `useSample`.
 */
const EVENT = "canvas-rgb:docs-sample";

type Detail = { scope: string; value: string };

export function useSample(scope: string, initial: string, keys: string[]) {
  const [value, setValue] = useState(initial);
  useEffect(() => {
    const onSample = (event: Event) => {
      const detail = (event as CustomEvent<Detail>).detail;
      if (detail?.scope === scope && keys.includes(detail.value)) setValue(detail.value);
    };
    window.addEventListener(EVENT, onSample);
    return () => window.removeEventListener(EVENT, onSample);
  }, [scope, keys.join("|")]);
  return value;
}

export function SampleSwitch({
  scope,
  labels,
  value,
}: {
  scope: string;
  labels: Array<{ value: string; label: string }>;
  value: string;
}) {
  return (
    <Segmented
      label="Code sample"
      value={value}
      options={labels}
      onChange={(next) => window.dispatchEvent(new CustomEvent<Detail>(EVENT, { detail: { scope, value: next } }))}
    />
  );
}
