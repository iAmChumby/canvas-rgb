"use client";

// The stage (#378): the picture the group itself draws, at the top of the group sheet.
//
// The picture is the group's own 80x50 canvas as the LEDs get it (the group's brightness on
// it), drawn into one canvas and scaled by the browser. A frame never causes a render: the
// picture is painted outside React, and only the stage's own state (connecting, live, paused,
// pattern, off, stopped) is React's.
//
// The box is the same size in every state, so nothing moves under the pointer when the state
// changes. The words of each state are the design's own.

import { useEffect, useRef, type ReactNode } from "react";
import { DotsNine, Moon, WarningCircle } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { GROUP_PREVIEW_H, GROUP_PREVIEW_W } from "@/lib/groupPreview";
import { useSnap } from "@/lib/store";
import { blit, type Surface } from "@/lib/thumbs";
import { PatternTile } from "@/views/effects/PatternTile";
import { useGroupPreview } from "@/views/scenes/useGroupPreview";
import type { Group } from "@/proto/Group";
import s from "@/views/scenes/scenes.module.css";

/** What the stage shows. The box is the same size in every one of them. */
export type GroupStageState =
  | "live"
  | "connecting"
  | "paused"
  | "pattern"
  | "off"
  | "stopped"
  | "failed";

/** The words of the states that have no picture. */
export const STAGE_WORDS = {
  connecting: "Connecting to the preview.",
  paused: "The lights are paused.",
  stopped: "The preview stopped.",
  pattern: "A pattern is drawn on the lights only. The LEDs show it.",
  off: "Off. Nothing is drawn.",
} as const;

/** What the caption's tooltip says: the picture is the group's, not the scene's. */
export const STAGE_CAPTION_TITLE =
  "Global brightness, warmth and white balance apply to the lights, not to this picture.";

export interface GroupStageProps {
  /** The group the sheet is open on. */
  group: Group;
  /** The name of the effect it plays, or "Off". */
  effectName: string;
  /** Why the group is dark, from the daemon; null while it runs. */
  failed: string | null;
  /** True when the group plays a pattern: a pattern has no picture. */
  pattern: boolean;
  /** True when the daemon holds this group in its saved scene. */
  saved: boolean;
  /** True when the lights are paused: the engine sends nothing. */
  paused: boolean;
  /** The devices of the group that are not connected. */
  away: number;
  /** The effect button and the mode control, beside the picture. */
  children?: ReactNode;
}

/**
 * The stage: the group's own picture, and the group's own controls beside it.
 *
 * The picture is drawn outside React: a frame goes into the canvas and nothing else. Only the
 * state of the stage is React's, and it changes when the state changes, not when a picture
 * arrives.
 */
export function GroupStage({
  group,
  effectName,
  failed,
  pattern,
  saved,
  paused,
  away,
  children,
}: GroupStageProps) {
  const off = group.plays.type === "off";
  // A pattern has no picture, so it opens no socket at all. A group set to Off does open one:
  // the daemon sends it black frames, and the stage says "Off" from the group's own state
  // rather than reading it out of the frames.
  const preview = useGroupPreview({ groupId: pattern ? null : group.id, saved });

  const state: GroupStageState = failed
    ? "failed"
    : off
      ? "off"
      : pattern
        ? "pattern"
        : paused
          ? "paused"
          : preview.state;

  const effectId = group.plays.type === "effect" ? group.plays.effect : null;
  // The saved pattern, so the stage draws its dots in their own colours.
  const patterns = useSnap((st) => st.patterns);
  const patternDoc = effectId ? patterns?.find((p) => p.id === effectId) : undefined;

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surface = useRef<Surface | null>(null);

  // The picture: one canvas, one draw call a frame, and no render.
  useEffect(
    () =>
      preview.onFrame((frame) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        if (!surface.current || surface.current.canvas !== canvas) {
          surface.current = { ctx, canvas, img: null };
        }
        blit(surface.current, frame.rgb, 0, frame.width, frame.height);
      }),
    [preview.onFrame],
  );

  const words =
    state === "failed"
      ? (failed ?? "")
      : state === "live"
        ? ""
        : STAGE_WORDS[state];

  return (
    <div className={s.stage} data-state={state} data-testid="group-stage">
      <div className={s.stageColumn}>
        <div className={s.stagePicture}>
          <canvas
            ref={canvasRef}
            className={s.stageCanvas}
            width={GROUP_PREVIEW_W}
            height={GROUP_PREVIEW_H}
            role="img"
            aria-label={`Preview of ${effectName}`}
          />
          {state !== "live" && (
            <div className={s.stageMark}>
              {state === "off" && <Moon size={22} aria-hidden="true" />}
              {(state === "stopped" || state === "failed") && (
                <WarningCircle size={22} weight="fill" aria-hidden="true" />
              )}
              {state === "pattern" &&
                (patternDoc ? (
                  <span className={s.stageDots}>
                    <PatternTile pattern={patternDoc} size="sm" />
                  </span>
                ) : (
                  <DotsNine size={22} aria-hidden="true" />
                ))}
              <span className={s.stageWords}>{words}</span>
              {state === "stopped" && (
                <Button size="sm" onClick={preview.retry}>
                  Try again
                </Button>
              )}
            </div>
          )}
        </div>
        <p className={s.stageCaption} title={STAGE_CAPTION_TITLE} aria-description={STAGE_CAPTION_TITLE}>
          Group brightness only.
        </p>
      </div>
      <div className={s.stageSide}>
        <div className={s.stageControls}>
          {children}
          {/* A pattern group has no mode control: its row is kept, so the band is the same
              height for an effect group and a pattern group and nothing moves under the
              pointer when the sheet is opened on another group. */}
          {pattern && (
            <div className={s.stageModeRow} data-testid="stage-mode-row" aria-hidden="true" />
          )}
        </div>
        {/* The note keeps its room whether there is a device away or not, so the stage's box
            is the same size in every state. */}
        <p className={s.stageAway}>
          {away > 0 ? `${away} not connected. The picture is what the group draws.` : ""}
        </p>
      </div>
    </div>
  );
}
