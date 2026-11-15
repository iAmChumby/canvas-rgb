"use client";

// New effect's first level: a stack, a pattern, or an HTML effect.
//
// The menu hangs under the New effect key in a popover, so opening it takes no room in the page
// and nothing moves under the pointer. It is three keys and their one line each: what the choice
// makes, in the words the rest of the editor uses.
//
// Plan call (Task 10): "New effect" used to open the stack builder directly. The menu puts the
// pattern beside the other two ways in, at the cost of one more click on the stack path.

import type { RefObject } from "react";
import { CirclesFour, Code, StackSimple, type Icon } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Popover } from "@/components/Popover";
import s from "@/views/effects/newEffect.module.css";

export function NewEffectMenu({
  anchor,
  open,
  disabled,
  onClose,
  onStack,
  onPattern,
  onCode,
}: {
  /** The New effect key it hangs under. */
  anchor: RefObject<HTMLElement | null>;
  open: boolean;
  /** The code editor is open: nothing new may start beside it. */
  disabled?: boolean;
  onClose: () => void;
  onStack: () => void;
  onPattern: () => void;
  onCode: () => void;
}) {
  const key = (
    label: string,
    note: string,
    Glyph: Icon,
    choose: () => void,
  ) => (
    <Button
      size="sm"
      className={s.menuKey}
      icon={<Glyph size={18} aria-hidden="true" />}
      aria-label={label}
      disabled={disabled}
      onClick={choose}
    >
      <span className={s.menuText}>
        <span className={s.menuName}>{label}</span>
        <span className={s.menuNote}>{note}</span>
      </span>
    </Button>
  );

  return (
    <Popover anchor={anchor} open={open} onClose={onClose} label="New effect">
      <div className={s.menu}>
        {key("Stack", "Effects layered and blended", StackSimple, onStack)}
        {key("Pattern", "A colour for every LED of one device", CirclesFour, onPattern)}
        {key("Code", "An HTML effect you write", Code, onCode)}
      </div>
    </Popover>
  );
}
