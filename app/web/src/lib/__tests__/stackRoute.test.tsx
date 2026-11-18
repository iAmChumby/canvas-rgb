import { useInsertionEffect } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { STACK_GUARD, useStackParam } from "@/lib/stackRoute";
import { plainEntry } from "@/test/historyGuard";

// Build mode's URL: an entry the builder's guard made opens no builder, and the route reads that
// once the page has committed, not while it renders (#484 fix pass).

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function Probe() {
  const [name] = useStackParam();
  return <p data-testid="stack">{name ?? "closed"}</p>;
}

/** What Next does when it pushes the entry of the page it is showing: in the commit, before the effects. */
function NextCommit({ to, state }: { to: string; state: unknown }) {
  useInsertionEffect(() => {
    window.history.pushState(state, "", to);
  }, [to, state]);
  return null;
}

beforeEach(() => {
  window.history.replaceState({}, "", "/effects/");
});

afterEach(() => {
  plainEntry();
  cleanup();
});

describe("useStackParam", () => {
  it("opens the builder the entry names", () => {
    window.history.replaceState({}, "", "/effects/?stack=Sunset");
    render(<Probe />);
    expect(screen.getByTestId("stack").textContent).toBe("Sunset");
  });

  it("opens no builder on a guard entry the document is on, and makes it the effects page", () => {
    window.history.replaceState({ [STACK_GUARD]: true }, "", "/effects/?stack=Sunset");
    render(<Probe />);
    expect(screen.getByTestId("stack").textContent).toBe("closed");
    expect(window.location.pathname + window.location.search).toBe("/effects/");
  });

  it("reads the entry the page opened on, not the one a client-side push is leaving", () => {
    // The entry being left names a builder and is plain; the page renders on it, and Next then pushes
    // the entry of the page itself in the commit. That entry is a guard's copy: no builder opens.
    window.history.replaceState({}, "", "/effects/?stack=Sunset");
    render(
      <>
        <NextCommit to="/effects/?stack=Sunset" state={{ [STACK_GUARD]: true }} />
        <Probe />
      </>,
    );
    expect(screen.getByTestId("stack").textContent).toBe("closed");
    expect(window.location.pathname + window.location.search).toBe("/effects/");
  });
});
