// The writes to one fan header: one request at a time, and while one is out only the newest waits.
// Shared by the header editor and the bulk bar (#110), so a single-header edit and a bulk run on
// the same header queue behind each other and the one that loses says so.

import { api } from "@/lib/endpoints";
import type { FanMode } from "@/proto/FanMode";

/**
 * What became of one write: "sent" means its request went out (a refusal arrives through its
 * onError), "superseded" means a newer write for the same header took its place and it never went.
 */
export type WriteResult = "sent" | "superseded";

interface Pending {
  mode: FanMode;
  onError: (e: unknown) => void;
  settle: (r: WriteResult) => void;
}

const slots = new Map<string, { running: boolean; queued: Pending | null }>();

async function pump(header: string, slot: { running: boolean; queued: Pending | null }) {
  slot.running = true;
  try {
    while (slot.queued) {
      const mine = slot.queued;
      slot.queued = null;
      try {
        await api.patchFanHeader(header, mine.mode);
      } catch (e) {
        mine.onError(e);
      }
      mine.settle("sent");
    }
  } finally {
    slot.running = false;
  }
}

/**
 * Sends a mode to a header so the latest choice is the one the chip ends on. Requests that
 * overlap can land out of order, so a new request waits for the one in flight, and a request
 * still waiting when a newer one arrives is dropped: its promise resolves "superseded" and its
 * onError never fires. A write's refusal goes to that write's own onError.
 */
export function writeFanMode(header: string, mode: FanMode, onError: (e: unknown) => void): Promise<WriteResult> {
  let slot = slots.get(header);
  if (!slot) {
    slot = { running: false, queued: null };
    slots.set(header, slot);
  }
  const s = slot;
  return new Promise<WriteResult>((settle) => {
    s.queued?.settle("superseded");
    s.queued = { mode, onError, settle };
    if (!s.running) void pump(header, s);
  });
}
