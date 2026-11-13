"use client";

// The Effects page's play action, now "Play on …": the groups of the active scene are the places
// an effect can play. One group plays there directly; more offers a list, with every group as one
// choice. An HTML effect may play in one group only (`SCENE_HTML_MAX` is 1), so with two or more
// groups it offers single groups and says why.

import { useEffect, useRef, useState, type Ref } from "react";
import { CaretDown, Play } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Checkbox } from "@/components/Checkbox";
import { Popover } from "@/components/Popover";
import type { Group } from "@/proto/Group";

/** "Play", when the scene has one group or fewer; "Play on …", when the choice is real. */
export function playLabel(count: number, running: boolean): string {
  if (running) return "Running";
  return count > 1 ? "Play on …" : "Play";
}

export function PlayOn({
  effect,
  groups,
  html,
  running,
  busy,
  disabled,
  variant = "primary",
  size = "md",
  pulseRef,
  onPlay,
}: {
  /** The effect's id, for the button's name when two effects share a label. */
  effect: string;
  groups: Group[];
  /** An HTML effect, or a stack with an HTML layer: one group at a time. */
  html: boolean;
  running: boolean;
  busy: boolean;
  disabled?: boolean;
  variant?: "default" | "primary";
  size?: "sm" | "md";
  pulseRef?: Ref<HTMLButtonElement>;
  onPlay: (groups: string[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const [every, setEvery] = useState(true);
  const [chosen, setChosen] = useState<string[]>([]);

  // Each opening starts from every group, except an HTML effect, which starts on the first.
  useEffect(() => {
    if (!open) return;
    setEvery(!html);
    setChosen(html ? groups.slice(0, 1).map((g) => g.name) : []);
    // Only an opening resets the choice: the groups themselves can change while it is open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const single = groups.length <= 1;
  const label = playLabel(groups.length, running);

  const start = (names: string[]) => {
    setOpen(false);
    onPlay(names);
  };

  if (single) {
    return (
      <Button
        ref={pulseRef}
        variant={running ? "default" : variant}
        size={size}
        busy={busy}
        disabled={disabled || running || groups.length === 0}
        title={groups.length === 0 ? "The scene has no group yet" : undefined}
        onClick={() => start(groups.map((g) => g.name))}
        icon={<Play size={14} weight="fill" />}
      >
        {label}
      </Button>
    );
  }

  const names = every ? groups.map((g) => g.name) : chosen;
  const toggleGroup = (name: string) => {
    if (html) {
      setChosen([name]);
      return;
    }
    if (every) {
      setEvery(false);
      setChosen([name]);
      return;
    }
    setChosen((cur) => (cur.includes(name) ? cur.filter((n) => n !== name) : [...cur, name]));
  };

  return (
    <span ref={anchor} className="play-on-anchor">
      <Button
        ref={pulseRef}
        variant={running ? "default" : variant}
        size={size}
        busy={busy}
        disabled={disabled || running}
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((v) => !v)}
        icon={<Play size={14} weight="fill" />}
      >
        {label}
        <CaretDown size={12} aria-hidden="true" />
      </Button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Play on">
        <div className="stack play-on" data-effect={effect}>
          <p className="play-on-head">Play on</p>
          {html && (
            <p className="play-on-note" role="note">
              One HTML effect at a time
            </p>
          )}
          <div className="play-on-list" role="group" aria-label="Groups">
            {!html && (
              <div className="play-on-row">
                <Checkbox
                  checked={every}
                  label="Every group"
                  onToggle={() => {
                    setEvery(true);
                    setChosen([]);
                  }}
                />
                <span>Every group</span>
                <span className="mute play-on-count">{groups.length}</span>
              </div>
            )}
            {groups.map((g) => (
              <div className="play-on-row" key={g.id}>
                <Checkbox
                  checked={every || chosen.includes(g.name)}
                  label={g.name}
                  onToggle={() => toggleGroup(g.name)}
                />
                <span className="one-line">{g.name}</span>
                <span className="mute play-on-count">{g.members.length}</span>
              </div>
            ))}
          </div>
          <Button
            variant="primary"
            size="sm"
            disabled={!html && every ? false : names.length === 0}
            onClick={() => start(names)}
          >
            Play
          </Button>
        </div>
      </Popover>
    </span>
  );
}
