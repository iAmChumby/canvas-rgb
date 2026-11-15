import { describe, expect, it } from "vitest";
import { firstNote, NOTE_LINE_MAX, parseInline, parseNotes } from "@/lib/releaseNotes";

// The one line a toast has room for, taken from a release's own notes (#366). It replaced
// "You have 0.16.0." under a title that already said "Updated to 0.16.0".

const NOTES = [
  "### Added",
  "- Chains: tell the app what is on a header in three steps: drag, type a number, confirm.",
  "- Patterns: paint the colour of every LED of a device.",
  "",
  "### Fixed",
  "- The lights API answers its edges the way the rest of the API does.",
].join("\n");

describe("firstNote", () => {
  it("is the release's first note, in plain words", () => {
    expect(firstNote(NOTES)).toBe(
      "Chains: tell the app what is on a header in three steps: drag, type a number, confirm.",
    );
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
});

describe("the notes reader it stands on", () => {
  it("keeps one block per heading, with its items", () => {
    const blocks = parseNotes(NOTES);
    expect(blocks.map((b) => b.title)).toEqual(["Added", "Fixed"]);
    expect(blocks[0]?.items).toHaveLength(2);
  });

  it("splits a line into text, code and bold, and a link keeps its words", () => {
    expect(parseInline("press **Update now** in `Settings` ([notes](https://example.test))")).toEqual([
      { kind: "text", text: "press " },
      { kind: "strong", text: "Update now" },
      { kind: "text", text: " in " },
      { kind: "code", text: "Settings" },
      { kind: "text", text: " (" },
      { kind: "text", text: "notes" },
      { kind: "text", text: ")" },
    ]);
  });
});
