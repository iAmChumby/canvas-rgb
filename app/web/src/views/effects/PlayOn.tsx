"use client";

// The Effects page's play action, now "Play on …": the groups of the active scene are the places
// an effect can play. One group plays there directly; more offers the groups, with every group as
// one choice. A group that already plays the effect is marked and left as it is: playing the
// effect on a second group never changes the first. An HTML effect (or a stack with an HTML
// layer) may play in one group only (`SCENE_HTML_MAX` is 1), so with two or more groups it offers
// single groups and says why. A daemon without scenes has no groups: Play falls back to its
// legacy activation, which plays on everything.

import { useRef, useState, type Ref } from "react";
import { CaretDown, Play } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Checkbox } from "@/components/Checkbox";
import { Popover } from "@/components/Popover";
import type { Group } from "@/proto/Group";

/** The key's words. `playing` is how many groups of the scene play the effect now. */
export function playLabel(
  count: number,
  playing: number,
  known: boolean,
  canPlay: boolean,
): string {
  if (!known) return "Play";
  if (count === 0) return "Play";
  if (!canPlay) return count === 1 ? "Playing" : `Playing on all ${count} groups`;
  if (playing > 0) return `Playing on ${playing} of ${count} · Play on …`;
  return count > 1 ? "Play on …" : "Play";
}

export function PlayOn({
  effect,
  groups,
  playing,
  groupsKnown,
  html,
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
  /** The names of the groups that already play this effect. */
  playing: string[];
  /** The scene routes answered: false while the scene is still being read. */
  groupsKnown: boolean;
  /** An HTML effect, or a stack with an HTML layer: one group at a time. */
  html: boolean;
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

  const playingSet = new Set(playing);
  const addable = groups.filter((g) => !playingSet.has(g.name));
  const allPlay = groups.length > 0 && addable.length === 0;
  const canPlay = groupsKnown && (groups.length === 0 || !allPlay);
  const label = playLabel(groups.length, playing.length, groupsKnown, canPlay);
  const single = groups.length <= 1;

  const start = (names: string[]) => {
    setOpen(false);
    onPlay(names);
  };

  // Each opening starts from every group it can still be added to, except an HTML effect,
  // which starts on the first of them.
  const toggleOpen = () => {
    if (open) {
      setOpen(false);
      return;
    }
    setEvery(!html);
    setChosen(html && addable[0] ? [addable[0].name] : []);
    setOpen(true);
  };

  if (!groupsKnown) {
    return (
      <Button
        ref={pulseRef}
        variant="default"
        size={size}
        busy
        disabled
        title="Reading the scene"
        icon={<Play size={14} weight="fill" />}
      >
        <span className="play-on-label">{label}</span>
      </Button>
    );
  }

  if (single) {
    const alone = groups[0];
    const plays = alone !== undefined && playingSet.has(alone.name);
    return (
      <Button
        ref={pulseRef}
        variant={plays ? "default" : variant}
        size={size}
        busy={busy}
        disabled={disabled || plays}
        title={groups.length === 0 ? "No scene: this plays on every light" : undefined}
        onClick={() => start(alone ? [alone.name] : [])}
        icon={<Play size={14} weight="fill" />}
      >
        <span className="play-on-label">{label}</span>
      </Button>
    );
  }

  const names = every ? addable.map((g) => g.name) : chosen;
  const toggleGroup = (name: string) => {
    // A group that already plays it keeps its own values: it cannot be unticked here.
    if (playingSet.has(name)) return;
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
        variant={allPlay ? "default" : variant}
        size={size}
        busy={busy}
        disabled={disabled || !canPlay}
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={toggleOpen}
        icon={<Play size={14} weight="fill" />}
      >
        <span className="play-on-label">{label}</span>
        {canPlay && <CaretDown size={12} aria-hidden="true" />}
      </Button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Play on">
        <div className="stack play-on" data-effect={effect}>
          <p className="play-on-head">Play on</p>
          {html && (
            <p className="play-on-note" role="note">
              One HTML effect at a time
            </p>
          )}
          {playing.length > 0 && (
            <p className="play-on-note" role="note">
              {playing.length === 1
                ? `${playing[0]} already plays it and keeps its own values.`
                : `${playing.length} groups already play it and keep their own values.`}
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
                <span className="mute play-on-count">{addable.length}</span>
              </div>
            )}
            {groups.map((g) => {
              const plays = playingSet.has(g.name);
              return (
                <div className="play-on-row" key={g.id}>
                  <Checkbox
                    checked={plays || every || chosen.includes(g.name)}
                    blocked={plays}
                    label={g.name}
                    title={plays ? "Already playing it here; it keeps its own values" : undefined}
                    onToggle={() => toggleGroup(g.name)}
                  />
                  <span className="one-line">{g.name}</span>
                  {plays && <span className="play-on-playing">Playing</span>}
                  <span className="mute play-on-count">{g.members.length}</span>
                </div>
              );
            })}
          </div>
          <Button
            variant="primary"
            size="sm"
            disabled={names.length === 0}
            onClick={() => start(names)}
          >
            Play
          </Button>
        </div>
      </Popover>
    </span>
  );
}
