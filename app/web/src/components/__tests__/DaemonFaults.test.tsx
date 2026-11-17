import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { DaemonDown } from "@/components/DaemonDown";
import { EngineDown } from "@/components/EngineDown";

// The daemon's two failures are two pages (#241). The daemon that is gone and the daemon
// that is up with its render engine not answering need different words: one says start
// the daemon, the other says the daemon is running and shows what it said.

afterEach(() => {
  cleanup();
});

describe("the daemon's two failures", () => {
  it("says the daemon is not there when it cannot be reached", () => {
    const { container } = render(<DaemonDown onRetry={async () => undefined} />);
    const page = container.textContent ?? "";
    expect(page).toMatch(/canvas-rgb is not answering/);
    expect(page).not.toMatch(/render engine/);
  });

  it("says the daemon is up and shows the words it sent", () => {
    const { container } = render(
      <EngineDown
        onRetry={async () => undefined}
        message="the render engine is not answering"
      />,
    );
    const page = container.textContent ?? "";
    // The daemon is up: the page does not say it is not answering.
    expect(page).toMatch(/canvas-rgb is up/);
    expect(page).not.toMatch(/canvas-rgb is not answering/);
    // The daemon's own words are on the page, as they are.
    expect(page).toContain("the render engine is not answering");
  });

  it("says canvas-rgb, not the daemon, on both pages", () => {
    const down = render(<DaemonDown onRetry={async () => undefined} />);
    expect(down.container.textContent ?? "").not.toMatch(/daemon/i);
    cleanup();
    const engine = render(
      <EngineDown onRetry={async () => undefined} message="the render engine is not answering" />,
    );
    expect(engine.container.textContent ?? "").not.toMatch(/daemon/i);
  });
});
