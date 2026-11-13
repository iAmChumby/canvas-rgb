"use client";

import type { CSSProperties } from "react";
import { useRef, useState } from "react";
import { ArrowsHorizontal, CopySimple, Moon, Sparkle, WarningCircle } from "@phosphor-icons/react";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { Button } from "@/components/Button";
import { InlineSkeleton } from "@/components/Skeleton";
import { Popover } from "@/components/Popover";
import { Slider } from "@/components/Slider";
import { EffectPicker } from "@/views/scenes/EffectPicker";
import { LedRun, runLabel } from "@/views/scenes/LedRun";
import type { Group } from "@/proto/Group";
import type { LightView } from "@/proto/LightView";
import type { UseScene } from "@/views/scenes/useScene";
import s from "@/views/scenes/scenes.module.css";

const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

/** The mode in the owner's words (spec "Language"). */
export const MODE_WORDS = { flow_across: "flow across", each_its_own: "each its own" } as const;

/** Whether the daemon's list of lights has been read: a row cannot count what it has not seen. */
export type LightsState = "ready" | "loading" | "failed";

/** What a row says in place of its devices while the lights are unknown. */
export const FAILURE_GONE = "This effect is no longer installed.";

/** "3 devices" or the one device's own name; the LED total goes beside it. */
export function memberSummary(lights: LightView[]): { who: string; leds: number; away: number } {
  const leds = lights.reduce((n, l) => n + l.led_count, 0);
  const away = lights.filter((l) => !l.connected).length;
  const who =
    lights.length === 1 ? (lights[0]?.label ?? "1 device") : `${lights.length} devices`;
  return { who, leds, away };
}

/**
 * One line under a name: who is in the row, how many LEDs, and how many are away. While the
 * lights are being read, or could not be, it says that: never "0 devices".
 */
function Summary({ lights, state }: { lights: LightView[]; state: LightsState }) {
  if (state === "loading") {
    return (
      <span className={`${s.summary} fig`} aria-busy="true">
        <InlineSkeleton width="9ch" />
        <InlineSkeleton width="7ch" />
      </span>
    );
  }
  if (state === "failed") {
    return (
      <span className={`${s.summary} fig`}>
        <span className={s.away}>Devices not read</span>
      </span>
    );
  }
  const { who, leds, away } = memberSummary(lights);
  return (
    <span className={`${s.summary} fig`}>
      <span className="one-line" title={who}>{who}</span>
      <span className={s.leds}>
        <AnimatedNumber value={leds} /> {leds === 1 ? "LED" : "LEDs"}
      </span>
      {away > 0 && <span className={s.away}>{away} not connected</span>}
    </span>
  );
}

export interface GroupRowProps {
  group: Group;
  /** The group's lights, in member order. A member the daemon no longer lists is left out. */
  lights: LightView[];
  /** The name of the effect it plays, or "Off". Never the effect's raw id. */
  effectName: string;
  /** The effect's id, for the key's tooltip only. */
  effectId?: string;
  /** The effect is no longer installed: the failure line says so in words. */
  missing?: boolean;
  /** Whether the lights are read: `lights` is only meaningful when it is "ready". */
  lightsState?: LightsState;
  /** Why the group is dark, from the daemon; null while it runs. */
  failed: string | null;
  /** Its place in the scene, for its accent. */
  index: number;
  open?: boolean;
  /** The scene's data: the row's own changes go through it. */
  use: UseScene;
  onOpen: (groupId: string) => void;
  onBrightness: (groupId: string, n: number) => void;
}

/**
 * One group: its live LEDs, the largest thing in the row, then what it plays, how its lights
 * share it, and its brightness. From 1180 px the row is one line (name, LEDs, effect, mode,
 * fader); below that it is two (name and LEDs, then effect and controls). A failed group says so
 * on a line of its own under the row. The effect's name opens the picker: it is the way to
 * change what the group plays.
 */
export function GroupRow({
  group,
  lights,
  effectName,
  effectId,
  missing = false,
  lightsState = "ready",
  failed,
  index,
  open,
  use,
  onOpen,
  onBrightness,
}: GroupRowProps) {
  const off = group.plays.type === "off";
  const ModeIcon = group.mode === "flow_across" ? ArrowsHorizontal : CopySimple;
  const [picking, setPicking] = useState(false);
  const pickerAnchor = useRef<HTMLButtonElement>(null);
  return (
    <article
      className={s.row}
      data-testid="group-row"
      data-failed={failed ? "true" : undefined}
      data-off={off ? "true" : undefined}
      data-open={open ? "true" : undefined}
      style={{ "--tone": `var(${ACCENTS[index % ACCENTS.length]})` } as CSSProperties}
      aria-label={group.name}
    >
      <Button
        variant="ghost"
        className={s.who}
        aria-label={`Open ${group.name}`}
        onClick={() => onOpen(group.id)}
      >
        <span className={s.name} data-testid="group-name">
          {group.name}
        </span>
        <Summary lights={lights} state={lightsState} />
      </Button>
      <div className={s.runWell}>
        <LedRun lights={lights} label={runLabel(group.name, lights)} />
      </div>
      <div className={s.controls}>
        <Button
          ref={pickerAnchor}
          className={s.effect}
          icon={off ? <Moon size={16} /> : <Sparkle size={16} />}
          aria-label={`${group.name} plays ${effectName}`}
          aria-haspopup="true"
          aria-expanded={picking}
          title={effectId && !off ? `${effectName} (${effectId})` : undefined}
          onClick={() => setPicking((v) => !v)}
        >
          <span className="one-line">{effectName}</span>
        </Button>
        <span className={`${s.mode} fig`} title={MODE_WORDS[group.mode]}>
          <ModeIcon size={16} aria-hidden="true" />
          <span className={`one-line ${s.modeText}`}>{MODE_WORDS[group.mode]}</span>
        </span>
        <div className={s.fader}>
          <Slider
            label={`${group.name} brightness`}
            min={0}
            max={100}
            value={group.brightness}
            onChange={(n) => onBrightness(group.id, n)}
          />
          <span className={`${s.pct} fig`}>
            <AnimatedNumber value={group.brightness} />%
          </span>
        </div>
      </div>
      <Popover
        anchor={pickerAnchor}
        open={picking}
        onClose={() => setPicking(false)}
        label={`What ${group.name} plays`}
        className={s.pickerPopover}
      >
        <EffectPicker groupId={group.id} use={use} onClose={() => setPicking(false)} />
      </Popover>
      {failed && (
        <p className={s.failure} role="alert">
          <WarningCircle size={16} weight="fill" aria-hidden="true" />
          {/* A missing effect is said in the owner's words; the daemon's own sentence, which can
              carry the effect's id, is the tooltip. */}
          <span className={s.failureText} title={failed}>
            {missing ? FAILURE_GONE : failed}
          </span>
        </p>
      )}
    </article>
  );
}

/** The lights no group holds: dark by definition, so the row has no effect and no controls. */
export function UnassignedRow({ lights, lightsState = "ready" }: { lights: LightView[]; lightsState?: LightsState }) {
  return (
    <article className={s.row} data-testid="unassigned-row" data-unassigned="true" aria-label="Unassigned">
      <div className={`${s.who} ${s.whoStatic}`}>
        <span className={s.name}>Unassigned</span>
        <Summary lights={lights} state={lightsState} />
      </div>
      <div className={s.runWell}>
        <LedRun lights={lights} label={runLabel("Unassigned", lights)} />
      </div>
      <span className={`${s.dark} fig`}>
        <Moon size={16} aria-hidden="true" />
        dark
      </span>
    </article>
  );
}
