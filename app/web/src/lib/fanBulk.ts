// Setting several fan headers at once (#110). The daemon has one route per header, so a bulk change
// is the per-header request sent once for each header in the selection, one after the other, behind
// one busy state. Each header answers for itself: the ones that took the change stay applied, the
// ones that did not are named with the reason. The daemon still enforces every floor; the floor
// applied here only keeps a request from being refused for being under it.

import { baseFloor } from "@/lib/fan-rules";
import { messageOf } from "@/lib/errors";
import type { CurvePoint } from "@/proto/CurvePoint";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FanMode } from "@/proto/FanMode";
import type { FansStatus } from "@/proto/FansStatus";
import type { WriteResult } from "@/lib/fanWrites";

/** Why a tach-only header cannot be ticked. */
export const TACH_ONLY_WHY = "Tach only: canvas-rgb reads its speed and cannot set it.";

/** Why a header with no duty reading cannot be ticked. */
export const UNREAD_DUTY_WHY = "canvas-rgb has not read this header's duty, so it will not set it.";

/** Why a header the board does not route cannot be ticked (issue #128). */
export const UNAVAILABLE_WHY = "Not on this board: this header is not fitted on your board.";

export type BulkAction =
  | { kind: "bios" }
  | { kind: "fixed"; percent: number }
  | { kind: "curve"; from: string };

export type BulkState = "applied" | "skipped" | "failed";

export interface BulkOutcome {
  id: string;
  label: string;
  state: BulkState;
  /** What happened to this header, in words. */
  detail: string;
}

/** A header's label without the "(tach only)" the daemon appends: the row's chip says it. */
export function plainLabel(label: string): string {
  return label.replace(/ \(tach only\)$/, "");
}

/** A header the chip can drive. A header the board does not route (issue #128: the chip has
 *  the channel, the board has no connector) and a tach-only header cannot be driven. */
export function canDrive(h: FanHeaderStatus): boolean {
  return h.available && h.pwm_percent !== null;
}

/** A header this board does not route (issue #128). */
export function isUnavailable(h: FanHeaderStatus): boolean {
  return !h.available;
}

/** The daemon handed this header back to BIOS although a mode was asked for. */
export function isHandedBack(h: FanHeaderStatus): boolean {
  return h.on_bios && h.mode.mode !== "bios";
}

export function actionWords(a: BulkAction, from?: string): string {
  if (a.kind === "bios") return "BIOS";
  if (a.kind === "fixed") return `Fixed ${a.percent}%`;
  return from ? `the curve of ${from}` : "a copied curve";
}

/** The mode one header is asked for. Null when the action has nothing for it. */
export function modeFor(a: BulkAction, h: FanHeaderStatus, fans: FansStatus): FanMode | null {
  if (a.kind === "bios") return { mode: "bios" };
  if (a.kind === "fixed") {
    const floor = baseFloor(h.id, fans.floor_percent);
    return { mode: "fixed", percent: Math.min(100, Math.max(floor, Math.round(a.percent))) };
  }
  const src = fans.headers.find((x) => x.id === a.from);
  if (!src || src.mode.mode !== "curve") return null;
  // The points are copied exactly, so the new curve is the source's curve.
  const points: CurvePoint[] = src.mode.points.map((p) => ({ ...p }));
  return { mode: "curve", source: { ...src.mode.source }, points };
}

export type Write = (header: string, mode: FanMode, onError: (e: unknown) => void) => Promise<WriteResult>;

/**
 * Sends the action to every header in `targets`, in order. A header the daemon handed back to
 * BIOS keeps that state: it is skipped and says why. A write that a newer edit of the same header
 * replaced before it went out is reported skipped, never applied. When `signal` aborts, no further
 * header is written and the outcomes so far come back. The refresh after the writes is the caller's.
 */
export async function applyBulk(
  targets: FanHeaderStatus[],
  action: BulkAction,
  fans: FansStatus,
  write: Write,
  signal?: AbortSignal,
): Promise<BulkOutcome[]> {
  const out: BulkOutcome[] = [];
  for (const h of targets) {
    if (signal?.aborted) break;
    const base = { id: h.id, label: h.label };
    // Issue #128: a header the board does not route is skipped like a handed-back header:
    // it is never sent.
    if (isUnavailable(h)) {
      out.push({ ...base, state: "skipped", detail: UNAVAILABLE_WHY });
      continue;
    }
    if (!canDrive(h)) {
      out.push({ ...base, state: "skipped", detail: TACH_ONLY_WHY });
      continue;
    }
    if (isHandedBack(h)) {
      const why = h.override_reason ? `: ${h.override_reason}` : "";
      out.push({ ...base, state: "skipped", detail: `Handed back to BIOS by the daemon${why}. Left as it is.` });
      continue;
    }
    if (action.kind === "curve" && action.from === h.id) {
      out.push({ ...base, state: "skipped", detail: "This is the curve being copied." });
      continue;
    }
    const mode = modeFor(action, h, fans);
    if (!mode) {
      out.push({ ...base, state: "failed", detail: "The header to copy from is no longer on a curve." });
      continue;
    }
    const failure: { text: string | null } = { text: null };
    const sent = await write(h.id, mode, (e) => {
      failure.text = messageOf(e);
    });
    if (sent === "superseded") {
      out.push({ ...base, state: "skipped", detail: "Superseded by your edit of this header: that edit went out instead." });
    } else if (failure.text !== null) {
      out.push({ ...base, state: "failed", detail: failure.text });
    } else if (action.kind === "fixed" && mode.mode === "fixed" && mode.percent !== Math.round(action.percent)) {
      out.push({ ...base, state: "applied", detail: `Raised to its floor, Fixed ${mode.percent}%.` });
    } else {
      out.push({ ...base, state: "applied", detail: mode.mode === "bios" ? "BIOS." : mode.mode === "fixed" ? `Fixed ${mode.percent}%.` : "Curve copied." });
    }
  }
  return out;
}

/** The one line that sums an apply up: how many took it, how many did not. */
export function summaryOf(outcomes: BulkOutcome[], words: string, total = outcomes.length): string {
  const n = (s: BulkState) => outcomes.filter((o) => o.state === s).length;
  const applied = n("applied");
  const failed = n("failed");
  const skipped = n("skipped");
  const parts = [`${applied} set to ${words}`];
  if (failed) parts.push(`${failed} failed`);
  if (skipped) parts.push(`${skipped} skipped`);
  if (total > outcomes.length) parts.push(`${total - outcomes.length} not sent`);
  return parts.join(", ");
}
