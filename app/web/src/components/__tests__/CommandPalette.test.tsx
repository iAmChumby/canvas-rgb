import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { useState } from "react";
import { CommandPalette, type Command } from "@/components/CommandPalette";

// The command palette's keyboard contract (#232), against the WAI-ARIA APG "combobox with a
// listbox popup": the search field is the combobox, the list is a listbox, each row is an
// option, and aria-activedescendant names the row Enter would run. Tab stays inside the palette
// while it is open and focus goes back to the control that opened it on close. The `short` list
// is the phone's list: a desktop window shows every command.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** The phone query answers from one flag, and the palette is told when the window changes. */
let phoneWidth = false;
const phoneListeners = new Set<() => void>();
function setPhoneWidth(on: boolean) {
  phoneWidth = on;
  for (const listener of phoneListeners) listener();
}

// The palette's beforeAll replaces these. The originals are put back afterwards, so this file
// does not leak a half-working matchMedia or a no-op scrollIntoView into the rest of the suite.
const originalMatchMedia = window.matchMedia;
const originalScrollIntoView = Element.prototype.scrollIntoView;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    get matches() {
      return query.includes("max-width: 720px") ? phoneWidth : false;
    },
    media: query,
    addEventListener: (_event: string, listener: () => void) => {
      if (query.includes("max-width: 720px")) phoneListeners.add(listener);
    },
    removeEventListener: (_event: string, listener: () => void) => {
      if (query.includes("max-width: 720px")) phoneListeners.delete(listener);
    },
  })) as unknown as typeof window.matchMedia;
  // jsdom has no layout, so the palette's "keep the active row in view" call has no method.
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
});

afterAll(() => {
  window.matchMedia = originalMatchMedia;
  Element.prototype.scrollIntoView = originalScrollIntoView;
});

let ran: string[] = [];
beforeEach(() => {
  ran = [];
  phoneWidth = false;
});

afterEach(() => {
  cleanup();
  phoneWidth = false;
});

function commands(): Command[] {
  return [
    // A status line is read, not run: the arrow keys never land on it.
    { id: "status-now", group: "Status", label: "Now playing", hint: "idle", status: true, run: () => {} },
    { id: "status-details", group: "Status", label: "Open the status details", run: () => ran.push("details") },
    { id: "page-home", group: "Pages", label: "Go to Layout", hint: "/", run: () => ran.push("layout") },
    { id: "theme", group: "Appearance", label: "Switch the theme", run: () => ran.push("theme") },
  ];
}

/** What the shell passes on a phone: the status lines, the pages and the lights. */
function shortCommands(): Command[] {
  return commands().filter((c) => c.group !== "Appearance");
}

describe("the command palette", () => {
  it("is a combobox over a listbox of options, and the arrows move the active descendant", async () => {
    render(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);
    const field = screen.getByRole("combobox", { name: "Search commands" });
    const list = screen.getByRole("listbox");

    // The field owns the popup and says the active row's id.
    expect(field.getAttribute("aria-expanded")).toBe("true");
    expect(field.getAttribute("aria-controls")).toBe(list.id);
    const options = within(list).getAllByRole("option");
    expect(options).toHaveLength(4);
    // One option per row, each with a stable id, and only the active one is selected. The first
    // actionable row is the second row: the status line above it is not one.
    expect(new Set(options.map((o) => o.id)).size).toBe(4);
    expect(options.map((o) => o.getAttribute("aria-selected"))).toEqual(["false", "true", "false", "false"]);
    expect(field.getAttribute("aria-activedescendant")).toBe(options[1]!.id);

    // ArrowDown moves the highlight to the next action row, and the descendant follows it.
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    await waitFor(() =>
      expect(options.map((o) => o.getAttribute("aria-selected"))).toEqual(["false", "false", "true", "false"]),
    );
    expect(field.getAttribute("aria-activedescendant")).toBe(options[2]!.id);

    // ArrowUp comes back.
    fireEvent.keyDown(document.body, { key: "ArrowUp" });
    await waitFor(() => expect(field.getAttribute("aria-activedescendant")).toBe(options[1]!.id));

    // Enter runs the row the descendant names.
    fireEvent.keyDown(document.body, { key: "Enter" });
    expect(ran).toEqual(["details"]);
  });

  it("traps Tab and Shift+Tab inside the palette while it is open", async () => {
    render(
      <>
        <button type="button">A key behind the palette</button>
        <CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />
      </>,
    );
    const field = screen.getByRole("combobox", { name: "Search commands" });
    await waitFor(() => expect(document.activeElement).toBe(field));
    const panel = screen.getByRole("dialog", { name: "Command palette" });
    // The tab stops: the field and the palette's own keys. The options are not tab stops (APG).
    const items = Array.from(
      panel.querySelectorAll<HTMLElement>('button:not([disabled]):not([tabindex="-1"]), input:not([disabled]):not([tabindex="-1"])'),
    );
    const last = items[items.length - 1]!;

    // Tab from the last key wraps to the first.
    last.focus();
    fireEvent.keyDown(document.body, { key: "Tab" });
    expect(document.activeElement).toBe(items[0]);

    // Shift+Tab from the first key wraps to the last.
    items[0]!.focus();
    fireEvent.keyDown(document.body, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);

    // Tab from a key on the page behind the palette comes into the palette, not out of it.
    const behind = screen.getByRole("button", { name: "A key behind the palette" });
    behind.focus();
    fireEvent.keyDown(behind, { key: "Tab" });
    expect(panel.contains(document.activeElement)).toBe(true);
  });

  it("gives focus back to the control that opened it when it closes", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Open the palette
          </button>
          {open && <CommandPalette commands={commands()} short={shortCommands()} onClose={() => setOpen(false)} />}
        </>
      );
    }
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Open the palette" });
    opener.focus();
    fireEvent.click(opener);
    await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });

  it("shows the short list below the phone breakpoint and the full list above it", async () => {
    render(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);

    // A desktop window: every command, including the Appearance rows the short list leaves out.
    expect(screen.getByRole("option", { name: "Switch the theme" })).toBeTruthy();
    expect(screen.getAllByRole("option")).toHaveLength(4);

    // Narrowed to a phone: the short list, and only the short list.
    act(() => setPhoneWidth(true));
    await waitFor(() => expect(screen.queryByRole("option", { name: "Switch the theme" })).toBeNull());
    expect(screen.getAllByRole("option")).toHaveLength(3);
    expect(screen.getByRole("option", { name: /Go to Layout/ })).toBeTruthy();

    // Widened again: the full list comes back.
    act(() => setPhoneWidth(false));
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(4));
  });

  it("resets the highlight to the first row when the filtered list changes (a resize), so Enter always has a row to run", async () => {
    render(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);
    const field = screen.getByRole("combobox", { name: "Search commands" });

    // Highlight the last row on a desktop window: two ArrowDowns land on "Switch the theme".
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    await waitFor(() => expect(field.getAttribute("aria-activedescendant")).toBe(within(screen.getByRole("listbox")).getAllByRole("option")[3]!.id));

    // Narrowed to a phone, the filtered list shrinks under the highlight. The highlight resets to
    // the first actionable row, so Enter has a row to run.
    act(() => setPhoneWidth(true));
    const options = within(screen.getByRole("listbox")).getAllByRole("option");
    await waitFor(() => expect(field.getAttribute("aria-activedescendant")).toBe(options[1]!.id));
    fireEvent.keyDown(document.body, { key: "Enter" });
    expect(ran).toEqual(["details"]);
  });

  it("keeps the highlight when the shell rebuilds the same commands (the once-a-second status poll)", async () => {
    const view = render(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);
    const field = screen.getByRole("combobox", { name: "Search commands" });
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    const last = within(screen.getByRole("listbox")).getAllByRole("option")[3]!.id;
    await waitFor(() => expect(field.getAttribute("aria-activedescendant")).toBe(last));

    // A new array with the same rows, as every status poll gives: the highlight stays.
    view.rerender(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);
    view.rerender(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);
    expect(field.getAttribute("aria-activedescendant")).toBe(last);
    fireEvent.keyDown(document.body, { key: "Enter" });
    expect(ran).toEqual(["theme"]);
  });

  it("keeps the options out of the tab order, so Tab never moves focus off the field onto a row", async () => {
    render(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);
    for (const option of within(screen.getByRole("listbox")).getAllByRole("option")) {
      if (option.tagName === "BUTTON") expect(option.getAttribute("tabindex")).toBe("-1");
    }
  });

  it("renders the empty line outside the listbox and closes aria-expanded when nothing matches", async () => {
    render(<CommandPalette commands={commands()} short={shortCommands()} onClose={() => {}} />);
    const field = screen.getByRole("combobox", { name: "Search commands" });
    const list = screen.getByRole("listbox");

    fireEvent.change(field, { target: { value: "zzzz" } });

    // The empty line is not a child of the listbox (only options are there); it is its own status.
    expect(within(list).queryByText("Nothing matches that.")).toBeNull();
    expect(screen.getByText("Nothing matches that.")).toBeTruthy();
    // The popup says it is closed while there is nothing in it.
    expect(field.getAttribute("aria-expanded")).toBe("false");
  });

  it("gives every row a unique id even when two command ids sanitise to the same string", () => {
    const clash: Command[] = [
      { id: "a b", group: "Pages", label: "First", run: () => {} },
      { id: "a/b", group: "Pages", label: "Second", run: () => {} },
    ];
    render(<CommandPalette commands={clash} onClose={() => {}} />);
    const ids = within(screen.getByRole("listbox")).getAllByRole("option").map((o) => o.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("leaves focus where a command put it instead of stealing it back to the opener", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Open the palette
          </button>
          <button type="button">Elsewhere</button>
          {open && (
            <CommandPalette
              commands={[
                { id: "focus-moving", group: "Pages", label: "Move the focus", run: () => screen.getByRole("button", { name: "Elsewhere" }).focus() },
              ]}
              onClose={() => setOpen(false)}
            />
          )}
        </>
      );
    }
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Open the palette" });
    opener.focus();
    fireEvent.click(opener);
    await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.keyDown(document.body, { key: "Enter" });
    // The command moved focus itself; the palette's unmount must not pull it back to the opener.
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "Elsewhere" })));
  });

  it("returns focus to the opener when the command did not move it (a navigating command)", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Open the palette
          </button>
          {open && (
            // A navigating command moves the page, not the focus: it touches no element.
            <CommandPalette
              commands={[{ id: "go", group: "Pages", label: "Go to Layout", run: () => {} }]}
              onClose={() => setOpen(false)}
            />
          )}
        </>
      );
    }
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Open the palette" });
    opener.focus();
    fireEvent.click(opener);
    await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.keyDown(document.body, { key: "Enter" });
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });
});
