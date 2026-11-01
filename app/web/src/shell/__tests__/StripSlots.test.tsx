import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PageActions, PageTitle, StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The strip's two slots (R2): a page draws its title and its own actions into the status strip.
// The page's title and actions leave with the page, never stand twice, and the buttons in the
// strip are the page's own buttons.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
});

/** A page that puts a title and one action into the strip. */
function PageA({ onAct }: { onAct: () => void }) {
  return (
    <div data-testid="page-a">
      <PageTitle title="Alpha" context="one line" />
      <PageActions>
        <button type="button" onClick={onAct}>
          Act on alpha
        </button>
      </PageActions>
    </div>
  );
}

function PageB() {
  return (
    <div data-testid="page-b">
      <PageTitle title="Beta" />
    </div>
  );
}

/** The strip and a router of two pages: the button "navigates" by swapping the page. */
function Shell({ onAct }: { onAct: () => void }) {
  const [page, setPage] = useState<"a" | "b">("a");
  return (
    <StripSlotsProvider>
      <header data-testid="strip">
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
      </header>
      <button type="button" onClick={() => setPage(page === "a" ? "b" : "a")}>
        Navigate
      </button>
      {page === "a" ? <PageA onAct={onAct} /> : <PageB />}
    </StripSlotsProvider>
  );
}

describe("the strip's slots", () => {
  it("draws the page's title and actions into the strip, once", async () => {
    render(<Shell onAct={() => undefined} />);
    const strip = screen.getByTestId("strip");
    // The title is in the strip (not in the page), and it is the only heading.
    expect(await screen.findByRole("heading", { level: 1, name: "Alpha" })).toBeTruthy();
    expect(strip.contains(screen.getByRole("heading", { level: 1, name: "Alpha" }))).toBe(true);
    expect(screen.getByTestId("page-a").querySelector("h1")).toBeNull();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Act on alpha" })).toHaveLength(1);
    expect(strip.contains(screen.getByRole("button", { name: "Act on alpha" }))).toBe(true);
  });

  it("calls the page's own handler from the strip's button", async () => {
    const onAct = vi.fn();
    render(<Shell onAct={onAct} />);
    fireEvent.click(await screen.findByRole("button", { name: "Act on alpha" }));
    expect(onAct).toHaveBeenCalledTimes(1);
  });

  it("takes the title and the actions away with the page, and puts the next page's in", async () => {
    render(<Shell onAct={() => undefined} />);
    await screen.findByRole("heading", { level: 1, name: "Alpha" });
    fireEvent.click(screen.getByRole("button", { name: "Navigate" }));
    // The next page's title is the only one; nothing of the last page stays in the strip.
    expect(await screen.findByRole("heading", { level: 1, name: "Beta" })).toBeTruthy();
    expect(screen.queryByRole("heading", { level: 1, name: "Alpha" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Act on alpha" })).toBeNull();
    expect(screen.queryByText("one line")).toBeNull();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    // And back: the title and the action come again, once.
    fireEvent.click(screen.getByRole("button", { name: "Navigate" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Alpha" })).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Act on alpha" })).toHaveLength(1);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });
});
