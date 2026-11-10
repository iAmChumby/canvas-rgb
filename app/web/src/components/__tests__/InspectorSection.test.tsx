import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { InspectorSection } from "@/components/InspectorSection";

// The inspector's disclosure (#257 phase 4, fix pass 1): the heading is a heading and the key is
// a key, side by side. A heading inside a button is invalid HTML; the key is named by the heading.

afterEach(cleanup);

describe("InspectorSection", () => {
  it("keeps the heading outside the key, and names the key by the heading", () => {
    render(
      <InspectorSection title="Area">
        <p>body</p>
      </InspectorSection>,
    );
    const heading = screen.getByRole("heading", { name: "Area" });
    const key = screen.getByRole("button", { name: "Area" });
    expect(key.contains(heading)).toBe(false);
    expect(heading.closest("button")).toBeNull();
    expect(key.querySelector("h1,h2,h3,h4,h5,h6")).toBeNull();
    expect(key.getAttribute("aria-labelledby")).toBe(heading.id);
    expect(heading.id).not.toBe("");
  });

  it("says whether it is open, and what it controls", () => {
    render(
      <InspectorSection title="Area">
        <p>body</p>
      </InspectorSection>,
    );
    const key = screen.getByRole("button", { name: "Area" });
    const body = document.getElementById(key.getAttribute("aria-controls") ?? "");
    expect(body).not.toBeNull();
    expect(key.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(key);
    expect(key.getAttribute("aria-expanded")).toBe("false");
    expect(body?.hasAttribute("hidden")).toBe(true);
  });
});
