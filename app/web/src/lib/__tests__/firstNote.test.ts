import { describe, expect, it } from "vitest";
import { firstNote, NOTE_LINE_MAX } from "@/lib/releaseNotes";

// The one line a toast has room for, taken from a release's own notes (#366). It replaced
// "You have 0.16.0." under a title that already said "Updated to 0.16.0".

const NOTES = [
  "### Added",
  "- Chains: set up a header in three steps.",
  "- Patterns: paint the colour of every LED of a device.",
  "",
  "### Fixed",
  "- The lights API answers its edges the way the rest of the API does.",
].join("\n");

describe("firstNote", () => {
  it("is the release's first note, in plain words", () => {
    expect(firstNote(NOTES)).toBe("Chains: set up a header in three steps.");
  });

  it("cuts the same note when it is longer than the toast's line", () => {
    expect(firstNote(NOTES.split("\n").slice(0, 2).join("\n"), 20)).toBe("Chains: set up a…");
  });

  it("loses the markup a toast cannot draw", () => {
    expect(firstNote("### Added\n- Updates: press **Update now** to install `canvas-rgb`.")).toBe(
      "Updates: press Update now to install canvas-rgb.",
    );
  });

  it("cuts a long note at a word, inside the line's width", () => {
    const long = `### Added\n- ${"word ".repeat(60).trim()}`;
    const line = firstNote(long)!;
    expect(line.length).toBeLessThanOrEqual(NOTE_LINE_MAX + 1);
    expect(line.endsWith("…")).toBe(true);
    expect(line.endsWith(" …")).toBe(false);
  });

  it("is null when the notes hold nothing to read", () => {
    expect(firstNote("")).toBeNull();
    expect(firstNote("### Added\n\n### Fixed\n")).toBeNull();
  });

  // #389: the line used to cut at 64 characters whatever was there, so it could stop in the middle
  // of a word and before the sentence's object.

  it("cuts a long note at the last clause mark that leaves a phrase", () => {
    const note =
      "### Fixed\n- Every message looks the same, and the update notice no longer repeats the version number in its second line";
    expect(firstNote(note)).toBe("Every message looks the same…");
  });

  it("reads the 0.16.0 release's own first note as its second clause", () => {
    const note =
      "### Added\n- Chains: tell the app what is on a header in three steps: drag, type a number, confirm. (#338)";
    expect(firstNote(note)).toBe("Chains: tell the app what is on a header in three steps: drag…");
  });

  it("prefers the word 'and' to a cut inside a sentence", () => {
    const note =
      "### Changed\n- The update notice looks for a newer release at start and once every six hours after that";
    expect(firstNote(note)).toBe("The update notice looks for a newer release at start…");
  });

  it("cuts a single word longer than the line at the line, as the last resort", () => {
    const note =
      "### Added\n- Controllers.bindings.reconnect_backoff_exponential_jitter_for_all_daemons_now and more";
    const line = firstNote(note)!;
    expect(line.endsWith("…")).toBe(true);
    expect(line.length).toBeLessThanOrEqual(NOTE_LINE_MAX + 1);
    // The first space in the note is past the line, so the cut has nowhere else to land.
    expect(line).not.toContain(" ");
  });

  it("shows a note that fits whole, with no ellipsis", () => {
    expect(firstNote("### Fixed\n- The preset's Undo stays for eight seconds.")).toBe(
      "The preset's Undo stays for eight seconds.",
    );
  });

  it("is the first item under a heading that opens the notes", () => {
    expect(firstNote("### Added\n- Patterns: paint every LED.")).toBe("Patterns: paint every LED.");
  });

  it("is null for notes that are not there at all (undefined at runtime)", () => {
    expect(firstNote(undefined as unknown as string)).toBeNull();
    expect(firstNote(null as unknown as string)).toBeNull();
  });
});
