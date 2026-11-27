// "See it without hardware" (#579): the whole editor on the demo backend, in the tray too.
//
// The flag is the URL's `?preview=1`, read once when the page loads and kept for the page's whole
// life. Nothing is stored, entering and leaving are full page loads (so no sample data survives in
// a store), and a reload keeps the preview because the address still says so.

const PARAM = "preview";

function asked(): boolean {
  return typeof window !== "undefined" && new URLSearchParams(window.location.search).get(PARAM) === "1";
}

// Latched: an in-app link drops the query, and that must not hand the page back to the real daemon.
const active = false;

/** True for a page that was loaded as a preview. */
export function previewActive(): boolean {
  return active;
}

/** The current address with the preview flag put on or taken off. */
export function previewAddress(on: boolean, href: string): string {
  const url = new URL(href);
  if (on) url.searchParams.set(PARAM, "1");
  else url.searchParams.delete(PARAM);
  return `${url.pathname}${url.search}${url.hash}`;
}

/** Puts the flag back on the address after an in-app navigation, so a reload stays in preview. */
export function keepPreviewInAddress(): void {
  if (!active || asked()) return;
  window.history.replaceState(window.history.state, "", previewAddress(true, window.location.href));
}

/** Loads Scenes as a preview. */
export function enterPreview(): void {
  window.location.assign("/?preview=1");
}

/** Loads the same page without the flag: the real PC again. */
export function leavePreview(): void {
  window.location.assign(previewAddress(false, window.location.href));
}

/**
 * Runs before the first paint (layout.tsx): the marker's row is there from the first frame, so
 * nothing moves when React arrives.
 */
export const PREVIEW_EARLY_SCRIPT = `try{if(new URLSearchParams(location.search).get("preview")==="1"){document.documentElement.setAttribute("data-preview","on")}}catch(e){}`;
