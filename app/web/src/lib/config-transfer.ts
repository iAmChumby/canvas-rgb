// Settings > Export and Import. Both buttons go through this one module, and so does the
// Import control's file read, so a refusal shows the daemon's own 422 reason in one place.

import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import type { ExportBundle } from "@/proto/ExportBundle";

/** Downloads the daemon's config bundle as `canvas-rgb-config-<date>.json`. */
export async function exportConfig(): Promise<string> {
  const bundle = await api.configExport();
  const text = JSON.stringify(bundle, null, 2);
  const blob = new Blob([text], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const stamp = new Date().toISOString().slice(0, 10);
  const name = `canvas-rgb-config-${stamp}.json`;
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 4000);
  return name;
}

/**
 * Reads a config file and posts it to the daemon. A file that is not JSON is refused here;
 * anything the daemon refuses comes back as an ApiFailure whose message is shown inline.
 */
export async function importConfigFile(file: File): Promise<void> {
  let bundle: ExportBundle;
  try {
    bundle = JSON.parse(await file.text()) as ExportBundle;
  } catch {
    throw new ApiFailure(0, "parse", `${file.name} is not a canvas-rgb config file (it is not valid JSON).`);
  }
  await api.configImport(bundle);
}
