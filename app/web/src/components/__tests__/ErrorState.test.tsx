import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ErrorState } from "@/components/ErrorState";
import { ApiFailure, caught, inContext, messageOf, presetFailure } from "@/lib/errors";

afterEach(cleanup);

const refusal = new ApiFailure(500, "internal", "the daemon said no", undefined, "Try again in a moment.");

describe("the error anatomy takes the caught value (#576, F8/F9)", () => {
  it("puts a view's title over the error's own words and the daemon's next step", () => {
    render(<ErrorState message="The list may be old." error={refusal} />);
    const card = screen.getByRole("alert");
    expect([...card.querySelectorAll("p")].map((p) => p.textContent)).toEqual([
      "The list may be old.",
      "Try again in a moment.",
      "the daemon said no",
    ]);
  });

  it("never says the title twice", () => {
    render(<ErrorState message="the daemon said no" error={new Error("the daemon said no")} />);
    expect(screen.getAllByText("the daemon said no")).toHaveLength(1);
  });

  it("says the context in front of the words in the compact form", () => {
    render(<ErrorState compact context="Rescan failed:" error="no reply" />);
    expect(screen.getByRole("alert").textContent).toBe("Rescan failed: no reply");
  });

  it("keeps an empty line, with nothing invented, when a reserved row has no error", () => {
    render(<ErrorState compact context="Rescan failed:" error={null} />);
    expect(screen.getByRole("alert").textContent).toBe("");
  });

  it("repeats a cut line in the tooltip", () => {
    render(<ErrorState compact tooltip error={refusal} />);
    expect(screen.getByRole("alert").getAttribute("title")).toBe("the daemon said no");
  });

  it("says the next step after the words of a compact line that has no key row (K2)", () => {
    render(<ErrorState compact tooltip error="unknown header" next="Press the key again to retry." />);
    const line = screen.getByRole("alert");
    expect(line.textContent).toBe("unknown header. Press the key again to retry.");
    expect(line.getAttribute("title")).toBe("unknown header. Press the key again to retry.");
    expect(screen.getByText("unknown header")).toBeTruthy();
  });

  it("says the error's own hint over the view's next step", () => {
    render(<ErrorState compact error={refusal} next="Set it again." />);
    expect(screen.getByRole("alert").textContent).toBe("the daemon said no. Try again in a moment.");
  });

  it("says no next step on an empty reserved line", () => {
    render(<ErrorState compact error={null} next="Set it again." />);
    expect(screen.getByRole("alert").textContent).toBe("");
  });

  it("offers no logs key outside the tray", () => {
    render(<ErrorState error={refusal} />);
    expect((screen.getByRole("button", { name: "Tray app only" }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("what a view stores (#576, F8/F9)", () => {
  it("keeps the caught error and its hint", () => {
    expect(caught(refusal)).toBe(refusal);
    expect(messageOf(caught(42))).toBe("unknown error");
  });

  it("keeps the hint when context is added", () => {
    const said = inContext("Saved, but it did not start:", refusal);
    expect(messageOf(said)).toBe("Saved, but it did not start: the daemon said no");
    expect((said as ApiFailure).hint).toBe("Try again in a moment.");
  });

  it("words a preset failure as the strip did", () => {
    expect(messageOf(presetFailure(new Error("could not save the effects: access is denied"), "save"))).toBe(
      "Not saved: Access is denied",
    );
    const clash = new ApiFailure(409, "conflict", "that name is taken");
    expect(presetFailure(clash, "save")).toBe(clash);
  });
});
