"use client";

// Step 3: does each one have its own colour?
//
// One row per part, in the colour its light shows. A row's LED count moves the boundary into its
// neighbour: the pair hold the same LEDs between them, the neighbour keeps at least one, and the
// lights follow at once. The last row is the end of the chain, so it may grow to the header's own
// maximum and no further.

import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainPartRow } from "@/views/chain/useChainSetup";
import { PartRow } from "@/views/chain/PartRow";
import s from "@/views/chain/chain.module.css";

export function ConfirmStep({
  parts,
  length,
  max,
  onName,
  onKind,
  onLeds,
}: {
  parts: ChainPartRow[];
  /** The chain's length now: the parts add up to it, except after the last row is grown. */
  length: number;
  /** The header's own longest length. */
  max: number;
  onName: (i: number, name: string) => void;
  onKind: (i: number, kind: ChainPartKind) => void;
  onLeds: (i: number, leds: number) => void;
}) {
  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>Does each one have its own colour?</h3>
      <p className={s.second}>Change a part&rsquo;s LEDs to move where its colour ends.</p>
      <ul className={s.list} aria-label="Parts on this chain">
        {parts.map((part, i) => {
          const next = parts[i + 1];
          const most = next
            ? part.leds + next.leds - 1
            : Math.max(1, max - (length - part.leds));
          return (
            <PartRow
              key={i}
              index={i}
              part={part}
              most={most}
              onName={(name) => onName(i, name)}
              onKind={(kind) => onKind(i, kind)}
              onLeds={(leds) => onLeds(i, leds)}
            />
          );
        })}
      </ul>
    </div>
  );
}
