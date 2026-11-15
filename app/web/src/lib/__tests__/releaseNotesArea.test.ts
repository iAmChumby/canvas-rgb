// The notes reader's area rule, its grouping and the promises it keeps (#365 fix pass): a line
// that starts with an area names one, the items of a section are grouped by the area they name,
// and the reader reads Windows line endings, issue 0 and wrapped lines the way it says it does.
//
// The module is imported as a namespace on purpose: a rule that is not there yet fails its own
// test, instead of breaking the whole file at the import.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import * as rn from "@/lib/releaseNotes";

/** The changelog as it is on disk, with whatever line endings it has. */
const changelog = readFileSync(resolve(process.cwd(), "../../CHANGELOG.md"), "utf8");

/** The words of an item, as one string. */
function words(item: { parts: { text: string }[] }): string {
  return item.parts.map((p) => p.text).join("");
}

describe("the area a line names", () => {
  it("reads a name of one word", () => {
    expect(rn.parseItem("Chains: tell the app what is on a header in three steps.").area).toBe("Chains");
  });

  it("reads a name of two words", () => {
    expect(rn.parseItem("Command line: `canvas-rgb status` names the active scene").area).toBe("Command line");
  });

  it("reads no area when five words come before the colon", () => {
    const item = rn.parseItem("The app opens on Scenes: your groups, their live lights and what each one plays.");
    expect(item.area).toBeNull();
    // The sentence stays whole: the colon is still in it.
    expect(words(item)).toContain("Scenes:");
  });

  it("reads no area when three words come before the colon", () => {
    expect(rn.parseItem("Lights and fans: a line that names no area.").area).toBeNull();
  });

  it("reads no area when the words before the colon hold code", () => {
    expect(rn.parseItem("`canvas-rgb`: a command that cannot reach the daemon.").area).toBeNull();
  });

  it("keeps the code that follows the area in the sentence", () => {
    const item = rn.parseItem("Command line: `canvas-rgb scenes list` shows the scenes.");
    expect(item.area).toBe("Command line");
    expect(words(item)).toContain("canvas-rgb scenes list");
  });

  it("reads no area in a line with no colon", () => {
    expect(rn.parseItem("The MonsGeek keyboard shows each key's own colour.").area).toBeNull();
  });

  it("starts the sentence with a capital once the area is gone", () => {
    expect(words(rn.parseItem("Chains: tell the app what is on a header."))).toBe(
      "Tell the app what is on a header.",
    );
  });
});

describe("the groups of a section", () => {
  it("puts the items that name no area first, then each area in the order it first appears", () => {
    const groups = rn.groupByArea([
      rn.parseItem("Chains: first"),
      rn.parseItem("Scenes: second"),
      rn.parseItem("An item that names no area"),
      rn.parseItem("Chains: third"),
    ]);
    expect(groups.map((g) => g.area)).toEqual([null, "Chains", "Scenes"]);
    expect(groups.map((g) => g.items.length)).toEqual([1, 2, 1]);
  });

  it("keeps a group of one item", () => {
    const groups = rn.groupByArea([rn.parseItem("Patterns: one item")]);
    expect(groups).toHaveLength(1);
    expect(groups[0]?.area).toBe("Patterns");
    expect(groups[0]?.items).toHaveLength(1);
  });
});

describe("the reader's promises", () => {
  it("reads a changelog with Windows line endings the same as one with Unix line endings", () => {
    const lf = changelog.replace(/\r\n/g, "\n");
    const crlf = lf.replace(/\n/g, "\r\n");
    expect(crlf).toContain("\r\n");
    // The CRLF text goes to the reader as it is: the reader does the normalising, not the test.
    expect(rn.parseRelease(crlf)).toEqual(rn.parseRelease(lf));
    const sample = "### Added\r\n\r\n- Chains: tell the app what is on a header. (#338)\r\n";
    expect(rn.parseRelease(sample)).toEqual(
      rn.parseRelease("### Added\n\n- Chains: tell the app what is on a header. (#338)\n"),
    );
    expect(rn.parseRelease(sample).sections[0]?.items[0]?.area).toBe("Chains");
  });

  it("reads the changelog as it is on disk the same as its Unix line endings", () => {
    expect(rn.parseRelease(changelog)).toEqual(rn.parseRelease(changelog.replace(/\r\n/g, "\n")));
  });

  it("links no issue for #0, and none for a bare #", () => {
    expect(rn.parseInline("#0")).toEqual([{ kind: "text", text: "#0" }]);
    expect(rn.parseInline("#")).toEqual([{ kind: "text", text: "#" }]);
    expect(rn.parseInline("see #0 and #12").filter((p) => p.kind === "issue")).toEqual([
      { kind: "issue", text: "#12", number: 12 },
    ]);
  });

  it("keeps a reference to issue 0 as words", () => {
    const item = rn.parseItem("A thing that names no issue (#0)");
    expect(item.refs).toEqual([]);
    expect(words(item)).toContain("(#0)");
  });

  it("joins a line that follows a bullet to the bullet above it", () => {
    const release = rn.parseRelease("### Added\n\n- A bullet that wrapped\nand continues here.\n");
    expect(release.sections[0]?.items).toHaveLength(1);
    expect(words(release.sections[0]?.items[0] ?? { parts: [] })).toBe(
      "A bullet that wrapped and continues here.",
    );
  });
});
