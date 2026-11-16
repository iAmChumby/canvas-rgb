"use client";

// One device of the header, as the owner confirms it: the colour its light shows, its name, what it
// is, and how many LEDs it holds.
//
// The colour chip is drawn from PART_COLOURS at the part's position, which is the same list and the
// same order the daemon is told to light, so the row's colour is the colour on the hardware. The
// LED count is bounded by its neighbour: the last LED of the next part cannot be taken, so the
// entry stops there and the boundary moves instead of a part skipping another.

import type { CSSProperties } from "react";

import { KindIcon } from "@/components/KindIcon";
import { NameField } from "@/components/NameField";
import { NumberField } from "@/components/NumberField";
import { Segmented, type SegOption } from "@/components/Segmented";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import { PART_COLOURS } from "@/views/chain/partColours";
import type { ChainPartRow } from "@/views/chain/useChainSetup";
import s from "@/views/chain/chain.module.css";

/** What a part is, in the words on the key and the mark beside them. */
const KINDS: SegOption<ChainPartKind>[] = [
  { value: "fan", label: "Fan", icon: <KindIcon kind={null} fan size={14} aria-hidden="true" /> },
  { value: "strip", label: "Strip", icon: <KindIcon kind="led_strip" size={14} aria-hidden="true" /> },
  { value: "other", label: "Other" },
];

/** The colour a part's light is, by position in the chain (a long chain starts the list again). */
export function partColour(index: number): string {
  return PART_COLOURS[index % PART_COLOURS.length] ?? PART_COLOURS[0] ?? "#ffffff";
}

/** The dot of colour that says which light this part is: the very colour the LEDs show. */
export function PartChip({ index }: { index: number }) {
  const colour = partColour(index);
  return (
    <span
      className={s.chip}
      style={{ "--c": colour } as CSSProperties}
      data-colour={colour}
      data-part-chip
      aria-hidden="true"
    />
  );
}

export function PartRow({
  index,
  part,
  most,
  onName,
  onKind,
  onLeds,
}: {
  index: number;
  part: ChainPartRow;
  /** The most LEDs this row can hold: its own plus its neighbour's, less the neighbour's last. */
  most: number;
  onName: (name: string) => void;
  onKind: (kind: ChainPartKind) => void;
  onLeds: (leds: number) => void;
}) {
  // The name is written as it is typed, so Enter and Escape only put the keyboard down.
  const endEditing = () => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  };

  return (
    <li className={s.row}>
      <span className={s.rowChip}>
        <PartChip index={index} />
      </span>
      <span className={s.rowName}>
        <NameField
          value={part.name}
          label={`Name of device ${index + 1}`}
          autoFocus={false}
          onChange={onName}
          onEnter={endEditing}
          onEscape={endEditing}
        />
      </span>
      <span className={s.rowKind}>
        <Segmented
          options={KINDS}
          value={part.kind}
          label={`What device ${index + 1} is`}
          onChange={onKind}
        />
      </span>
      <span className={s.rowLeds}>
        <NumberField
          value={part.leds}
          min={1}
          max={most}
          label={`LEDs in device ${index + 1}`}
          onCommit={onLeds}
        />
        <span className={s.unit}>LEDs</span>
      </span>
    </li>
  );
}
