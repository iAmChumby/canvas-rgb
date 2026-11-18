// The history helper the builder's tests clean up with (#484 fix pass). It must leave nothing
// behind that can reach the next test: not the guard's state on the current entry, and not a Back
// that is still on its way to a guard entry further back.

import { afterEach, describe, expect, it } from "vitest";
import { historySettled, plainEntry } from "@/test/historyGuard";

const GUARD = { canvasRgbStackGuard: true };

afterEach(() => {
  plainEntry();
});

describe("plainEntry", () => {
  it("cancels a Back that is already on its way, so it cannot land on a guard's copy", async () => {
    const here = "/effects/?stack=Sunset";
    window.history.replaceState({}, "", here);
    // The builder goes dirty: a guard entry is pushed.
    window.history.pushState(GUARD, "", here);
    // It undoes to clean: the guard's entry is taken back, the Back is queued ...
    window.history.back();
    // ... and a new edit pushes a second guard entry before that Back has landed, on top of the first.
    window.history.pushState(GUARD, "", here);
    // Undo again: a second Back is queued. The test ends here, as the slider tests of the stack builder do.
    window.history.back();

    plainEntry();
    await historySettled();

    // Nothing landed on the first guard's copy: the document is on a plain entry.
    expect(window.history.state).toEqual({});
    expect(window.location.search).toBe("?stack=Sunset");
  });

  it("clears the guard's state on the entry the document is on", () => {
    window.history.pushState(GUARD, "", "/effects/?stack=Sunset");
    expect(window.history.state).toEqual(GUARD);
    plainEntry();
    expect(window.history.state).toEqual({});
  });
});
