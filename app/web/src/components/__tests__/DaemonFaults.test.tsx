import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import { useState } from "react";
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

  it("replaces the page after Try again when the daemon answers again", () => {
    // The shell swaps this page for the real one when its next status read succeeds (#576 item 3).
    const Harness = () => {
      const [up, setUp] = useState(false);
      return up ? (
        <div data-testid="live-page">the editor is live</div>
      ) : (
        <DaemonDown onRetry={async () => setUp(true)} />
      );
    };
    const { container } = render(<Harness />);
    expect(container.textContent).toMatch(/canvas-rgb is not answering/);
    // The recovery keys are on this page before the retry (#576 item 3): they failed to exist
    // on the tree before this change.
    expect(screen.getByRole("button", { name: /Save diagnostics/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Report issue/i })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Try again/i }));
    expect(screen.getByTestId("live-page")).toBeTruthy();
    expect(container.textContent).not.toMatch(/canvas-rgb is not answering/);
    expect(container.textContent).not.toMatch(/unknown error/i);
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
