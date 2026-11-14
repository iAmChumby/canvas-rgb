// Task 11: the command palette offers one "Set scene: <name>" per scene, and no layout command.

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { CommandPalette } from "@/components/CommandPalette";
import { sceneCommands } from "@/shell/commands";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";

vi.mock("@/components/MediaQuery", () => ({ useMediaQuery: () => false }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
});

afterEach(cleanup);

const scene = (id: string, name: string): Scene => ({ id, name, rev: 1, groups: [] });

const answer: ScenesResponse = {
  active: "gaming",
  scenes: [scene("gaming", "Gaming"), scene("focus", "Focus"), scene("evening", "Evening")],
  group_states: [],
};

describe("the scene commands", () => {
  it("lists a Set scene command per scene", () => {
    const set = vi.fn();
    const commands = sceneCommands(answer, set);
    expect(commands.map((c) => c.label)).toEqual([
      "Set scene: Gaming",
      "Set scene: Focus",
      "Set scene: Evening",
    ]);
    expect(commands.every((c) => c.group === "Scenes")).toBe(true);
  });

  it("sets the scene it names, and offers no layout command", () => {
    const set = vi.fn();
    const commands = sceneCommands(answer, set);
    const palette = render(<CommandPalette commands={commands} onClose={() => {}} />);
    expect(within(palette.container).queryByText(/layout/i)).toBeNull();
    fireEvent.click(screen.getByRole("option", { name: "Set scene: Focus" }));
    expect(set).toHaveBeenCalledWith("focus");
  });
});
