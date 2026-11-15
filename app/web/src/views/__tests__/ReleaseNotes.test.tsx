// The What's new view (#365): the notes drawn as a page, not a wall of bullets. The sample is
// the real 0.16.0 section of CHANGELOG.md, which is what the tray hands the page.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { WhatsNewPanel } from "@/views/updates/ReleaseNotes";

const changelog = readFileSync(resolve(process.cwd(), "../../CHANGELOG.md"), "utf8").replace(/\r\n/g, "\n");

/** The 0.16.0 section as the tray sends it: the heading stripped, the bullets kept. */
function notesOf(version: string): string {
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
  return lines
    .slice(start + 1, end)
    .join("\n")
    .trim();
}

afterEach(() => {
  cleanup();
  delete window.__TAURI__;
});

const NOTES = notesOf("0.16.0");
const PUB_DATE = "2026-11-15T00:00:00Z";

describe("the What's new notes", () => {
  it("heads the page with the release's version and date", () => {
    render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    expect(screen.getByRole("heading", { level: 3, name: "0.16.0" })).toBeTruthy();
    expect(screen.getByText("15 November 2026").getAttribute("datetime")).toBe("2026-11-15");
  });

  it("draws the changelog's headings as sections, in the changelog's order", () => {
    render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    const sections = screen.getAllByRole("heading", { level: 4 }).map((h) => h.textContent);
    expect(sections).toEqual(["Added", "Changed", "Fixed", "Removed"]);
  });

  it("links every issue in the notes to its issue page, and sets it apart from the sentence", () => {
    render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    const links = screen.getAllByRole("link", { name: /^Issue #\d+ on GitHub$/ });
    // One per bullet of the real section: 22.
    expect(links.length).toBe(22);
    const first = screen.getAllByRole("link", { name: "Issue #338 on GitHub" })[0];
    expect(first?.getAttribute("href")).toBe("https://github.com/iAmChumby/canvas-rgb/issues/338");
    expect(first?.className).toContain("notes-ref");
    // The sentence keeps its words and no longer carries the reference.
    expect(screen.queryByText(/confirm\. \(#338\)/)).toBeNull();
  });

  it("opens an issue in the person's browser through the tray's opener, not in the window", async () => {
    const invoke = vi.fn(async () => null);
    window.__TAURI__ = { core: { invoke }, event: { listen: vi.fn(async () => () => undefined) } };
    render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    fireEvent.click(screen.getAllByRole("link", { name: "Issue #338 on GitHub" })[0] as HTMLElement);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("plugin:opener|open_url", {
        url: "https://github.com/iAmChumby/canvas-rgb/issues/338",
      }),
    );
  });

  it("shows HTML in a changelog line as words", () => {
    const notes = "### Fixed\n\n- <b>not bold</b> and <img src=x onerror=alert(1)> stays a sentence. (#9)";
    const { container } = render(<WhatsNewPanel version="9.9.9" date={null} notes={notes} />);
    expect(screen.getByText("<b>not bold</b> and <img src=x onerror=alert(1)> stays a sentence.")).toBeTruthy();
    expect(container.querySelector("b")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
  });

  it("says so when a release has no notes, as before", () => {
    render(<WhatsNewPanel version="0.17.0" date={null} notes="" />);
    expect(screen.getByText("No notes were published for 0.17.0.")).toBeTruthy();
  });
});

describe("the notes grouped by area", () => {
  it("writes each area of a section once, in the order the areas first appear", () => {
    const { container } = render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    const first = container.querySelector(".notes-sec");
    const labels = [...(first?.querySelectorAll(".notes-area") ?? [])].map((el) => el.textContent);
    expect(labels).toEqual(["Chains", "Scenes", "Effects", "Command line", "Patterns"]);
  });

  it("puts the items that name no area in one group at the top of the section", () => {
    const { container } = render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    const first = container.querySelector(".notes-sec");
    const groups = [...(first?.querySelectorAll(".notes-group") ?? [])];
    expect(groups).toHaveLength(6);
    expect(groups[0]?.querySelector(".notes-area")).toBeNull();
    expect(groups[0]?.querySelectorAll(".notes-item")).toHaveLength(2);
    expect(groups.slice(1).map((g) => g.querySelector(".notes-area")?.textContent)).toEqual([
      "Chains",
      "Scenes",
      "Effects",
      "Command line",
      "Patterns",
    ]);
  });

  it("writes the area of a group that holds one item as well", () => {
    const { container } = render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    const patterns = [...container.querySelectorAll(".notes-group")].find(
      (g) => g.querySelector(".notes-area")?.textContent === "Patterns",
    );
    expect(patterns?.querySelectorAll(".notes-item")).toHaveLength(1);
  });

  it("takes the area's name off the sentence", () => {
    const { container } = render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
    const chains = [...container.querySelectorAll(".notes-group")].find(
      (g) => g.querySelector(".notes-area")?.textContent === "Chains",
    );
    const sentence = chains?.querySelector(".notes-text")?.textContent ?? "";
    expect(sentence).toBe("Tell the app what is on a header in three steps: drag, type a number, confirm.");
  });
});

describe("the notes' links", () => {
  it("draws two issue links in one sentence without a duplicate key warning", () => {
    const errors: string[] = [];
    const spy = vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
      errors.push(args.map((a) => String(a)).join(" "));
    });
    try {
      render(<WhatsNewPanel version="1.0.0" date={null} notes={"### Fixed\n\n- The same fix as #1 and #2 now. (#3)"} />);
    } finally {
      spy.mockRestore();
    }
    expect(errors).toEqual([]);
  });

  it("catches a refused opener instead of leaving a rejected promise behind", async () => {
    const seen: unknown[] = [];
    const onNode = (reason: unknown) => {
      seen.push(reason);
    };
    const onWindow = (e: Event) => {
      seen.push((e as PromiseRejectionEvent).reason);
      e.preventDefault();
    };
    process.on("unhandledRejection", onNode);
    window.addEventListener("unhandledrejection", onWindow);
    const invoke = vi.fn(async () => {
      throw new Error("the opener refuses");
    });
    window.__TAURI__ = { core: { invoke }, event: { listen: vi.fn(async () => () => undefined) } };
    try {
      render(<WhatsNewPanel version="0.16.0" date={PUB_DATE} notes={NOTES} />);
      fireEvent.click(screen.getAllByRole("link", { name: "Issue #338 on GitHub" })[0] as HTMLElement);
      await waitFor(() => expect(invoke).toHaveBeenCalled());
      await new Promise((r) => setTimeout(r, 20));
    } finally {
      process.off("unhandledRejection", onNode);
      window.removeEventListener("unhandledrejection", onWindow);
    }
    expect(seen).toEqual([]);
  });
});
