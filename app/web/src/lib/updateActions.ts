// Sends one of the person's update choices to the tray and takes the state it answers with. The
// tray also pushes every change as an event, so the answer only makes the screen quicker. Returns
// the error words when the command failed, so the caller can show them where the person looked.

import { messageOf } from "@/lib/errors";
import { patchSnap, refreshUpdate } from "@/lib/store";
import type { UpdateState } from "@/proto/UpdateState";

export async function sendUpdate(call: () => Promise<UpdateState | null>): Promise<string | null> {
  try {
    const next = await call();
    if (next) patchSnap({ update: next });
    else await refreshUpdate();
    return null;
  } catch (e) {
    return messageOf(e);
  }
}
