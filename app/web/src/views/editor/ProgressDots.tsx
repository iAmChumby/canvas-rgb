"use client";

import { useMemo } from "react";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { progressParts } from "@/views/editor/setup";
import { zoneKey } from "@/lib/zones";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";

/**
 * The progress, from the third screen on: one LED dot per part with lights. A dot is lit when
 * its part is mapped and hollow when the owner skipped it ("Can't see it"), and beside them a
 * mono "04 / 15". The hello shows no count at all.
 */
export function ProgressDots({
  layout,
  devices,
  skipped,
  celebrate = false,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  /** The keys (`zoneKey`) of the parts the owner could not see. */
  skipped: ReadonlySet<string>;
  /** The dots light one after the other: the finish, once. */
  celebrate?: boolean;
}) {
  const parts = useMemo(() => progressParts(layout, devices), [layout, devices]);
  const mapped = parts.filter((p) => p.mapped).length;
  return (
    <div className={s.dots} role="progressbar" aria-label="Parts mapped" aria-valuemin={0} aria-valuemax={parts.length} aria-valuenow={mapped}>
      <span className={s.dotRow} data-celebrate={celebrate}>
        {parts.map((p, i) => {
          const z = layout.zones[p.index];
          const hollow = p.mapped && z !== undefined && skipped.has(zoneKey(z));
          return (
            <i
              key={p.index}
              className={s.dot}
              data-state={!p.mapped ? "todo" : hollow ? "skipped" : "lit"}
              style={{ ["--i" as string]: i }}
            />
          );
        })}
      </span>
      <span className={s.dotCount}>
        <AnimatedNumber value={mapped} className={s.pad} /> / {String(parts.length).padStart(2, "0")}
      </span>
    </div>
  );
}
