// The release notes reader (#365). The real 0.16.0 section of CHANGELOG.md is the sample: the
// page used to draw it as one flat list of bullets with the issue references as plain words.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { issueUrl, noteDate, parseInline, parseItem, parseRelease } from "@/lib/releaseNotes";

const changelog = readFileSync(resolve(process.cwd(), "../../CHANGELOG.md"), "utf8").replace(/\r\n/g, "\n");

/** One version's section of the changelog, heading included. */
function section(version: string): string {
  const lines = changelog.split("\n");
  const start = lines.findIndex((l) => l.startsWith(`## [${version}]`));
  expect(start).toBeGreaterThanOrEqual(0);
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (lines[i]?.startsWith("## [")) {
      end = i;
      break;
    }
  }
  return lines.slice(start, end).join("\n").trim();
}

const notes = section("0.16.0");

describe("a release section of the changelog", () => {
  it("keeps the release's own heading as its version and date, not as a section", () => {
    const release = parseRelease(notes);
    expect(release.version).toBe("0.16.0");
    expect(release.date).toBe("2026-11-15");
    expect(release.sections.map((s) => s.title)).toEqual(["Added", "Changed", "Fixed", "Removed"]);
  });

  it("holds every bullet of the real section, in its own section", () => {
    const release = parseRelease(notes);
    const counts = Object.fromEntries(release.sections.map((s) => [s.title, s.items.length]));
    // The section's bullets, counted in CHANGELOG.md: 12 added, 6 changed, 2 fixed, 2 removed.
    expect(counts).toEqual({ Added: 12, Changed: 6, Fixed: 2, Removed: 2 });
    const total = release.sections.reduce((n, s) => n + s.items.length, 0);
    expect(total).toBe(notes.split("\n").filter((l) => /^- /.test(l)).length);
  });

  it("takes the issue reference off the sentence and keeps it as a number", () => {
    const [first] = parseRelease(notes).sections[0]?.items ?? [];
    expect(first?.refs).toEqual([338]);
    const text = (first?.parts ?? []).map((p) => p.text).join("");
    // The area the line names is written once for its group, so the sentence itself starts at
    // "Tell the app".
    expect(text).toBe("Tell the app what is on a header in three steps: drag, type a number, confirm.");
    expect(text).not.toContain("#338");
  });

  it("reads every item's issue reference: none is left as words in the sentence", () => {
    const items = parseRelease(notes).sections.flatMap((s) => s.items);
    expect(items.length).toBeGreaterThan(20);
    for (const item of items) {
      expect(item.refs.length).toBeGreaterThanOrEqual(1);
      const text = item.parts.map((p) => (p.kind === "text" || p.kind === "code" || p.kind === "strong" || p.kind === "link" ? p.text : "")).join("");
      expect(text).not.toMatch(/#\d+/);
    }
  });

  it("has no notes heading of its own when the tray strips it, and then reads no version", () => {
    const body = notes.split("\n").slice(1).join("\n");
    const release = parseRelease(body);
    expect(release.version).toBeNull();
    expect(release.sections[0]?.title).toBe("Added");
  });
});

describe("one item's inline words", () => {
  it("reads a list of issues after the sentence", () => {
    const item = parseItem("A refusal now names the scenes it is in (#233, #230)");
    expect(item.refs).toEqual([233, 230]);
    expect(item.parts).toEqual([{ kind: "text", text: "A refusal now names the scenes it is in" }]);
  });

  it("reads an issue named in the middle of a sentence as a link", () => {
    const item = parseItem("The same fix as #310, and the light stays on.");
    expect(item.refs).toEqual([]);
    expect(item.parts).toEqual([
      { kind: "text", text: "The same fix as " },
      { kind: "issue", text: "#310", number: 310 },
      { kind: "text", text: ", and the light stays on." },
    ]);
  });

  it("reads code, bold and a link, and leaves a link that is not https as words", () => {
    expect(parseInline("Run `canvas-rgb status` **now** [docs](https://example.com/x)")).toEqual([
      { kind: "text", text: "Run " },
      { kind: "code", text: "canvas-rgb status" },
      { kind: "text", text: " " },
      { kind: "strong", text: "now" },
      { kind: "text", text: " " },
      { kind: "link", text: "docs", href: "https://example.com/x" },
    ]);
    expect(parseInline("[x](http://example.com)")).toEqual([{ kind: "text", text: "[x](http://example.com)" }]);
  });

  it("does not read an issue reference inside code", () => {
    expect(parseInline("`#310`")).toEqual([{ kind: "code", text: "#310" }]);
  });

  it("keeps HTML in a line as words, not as markup", () => {
    expect(parseItem("<b>bold</b> and <img src=x>")).toEqual({
      area: null,
      parts: [{ kind: "text", text: "<b>bold</b> and <img src=x>" }],
      refs: [],
    });
  });
});

describe("the addresses and the date", () => {
  it("points every issue at the repository's issue page", () => {
    expect(issueUrl(365)).toBe("https://github.com/iAmChumby/canvas-rgb/issues/365");
  });

  it("writes a release date as a day, a month and a year", () => {
    expect(noteDate("2026-11-15")).toEqual({ text: "15 November 2026", iso: "2026-11-15" });
    expect(noteDate("2026-11-15T09:14:00Z")).toEqual({ text: "15 November 2026", iso: "2026-11-15" });
  });

  it("draws no date for anything that is not one", () => {
    expect(noteDate(null)).toBeNull();
    expect(noteDate("")).toBeNull();
    expect(noteDate("2026-13-01")).toBeNull();
    expect(noteDate("last Tuesday")).toBeNull();
  });
});
