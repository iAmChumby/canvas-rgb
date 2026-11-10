import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Check, FloppyDisk } from "@phosphor-icons/react";
import { BusyIcon, Button, IconButton } from "@/components/Button";
import { LOAD_DELAY_MS } from "@/components/Loading";
import { LoopPreview } from "@/components/LoopPreview";
import { Select } from "@/components/Select";
import { CanvasSkeleton, InlineSkeleton } from "@/components/Skeleton";

// React needs this flag to run effects inside act() under a test runner.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  vi.useFakeTimers();
  window.IntersectionObserver ??= class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
    takeRecords(): IntersectionObserverEntry[] {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  // jsdom has no matchMedia, which the motion gate behind the previews reads.
  window.matchMedia ??= ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const advance = (ms: number) => act(() => void vi.advanceTimersByTime(ms));
const spinner = (c: ParentNode) => c.querySelector(".spin");

describe("the busy Button", () => {
  it("is aria-disabled and aria-busy at once, and shows no spinner before the delay", () => {
    const { container } = render(<Button busy>Save</Button>);
    const button = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(button.getAttribute("aria-disabled")).toBe("true");
    expect(button.getAttribute("aria-busy")).toBe("true");
    expect(spinner(container)).toBeNull();
    advance(LOAD_DELAY_MS - 1);
    expect(spinner(container)).toBeNull();
  });

  it("stays focusable: it is not disabled, and keeps keyboard focus when it turns busy", () => {
    const { rerender } = render(<Button>Save</Button>);
    const button = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    button.focus();
    expect(document.activeElement).toBe(button);
    rerender(<Button busy>Save</Button>);
    expect(button.disabled).toBe(false);
    expect(document.activeElement).toBe(button);
    advance(LOAD_DELAY_MS);
    expect(document.activeElement).toBe(button);
    rerender(<Button>Save</Button>);
    expect(document.activeElement).toBe(button);
  });

  it("shows the spinner at the delay and drops it when the request ends", () => {
    const { container, rerender } = render(<Button busy>Save</Button>);
    advance(LOAD_DELAY_MS);
    expect(spinner(container)).not.toBeNull();
    rerender(<Button busy={false}>Save</Button>);
    expect(spinner(container)).toBeNull();
    const button = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(button.getAttribute("aria-disabled")).toBeNull();
    expect(button.getAttribute("aria-busy")).toBeNull();
  });

  it("a request that ends inside the delay never shows a spinner", () => {
    const { container, rerender } = render(<Button busy>Save</Button>);
    advance(LOAD_DELAY_MS - 10);
    rerender(<Button busy={false}>Save</Button>);
    advance(LOAD_DELAY_MS * 2);
    expect(spinner(container)).toBeNull();
  });

  it("with an icon the spinner shares the icon cell and the label stays", () => {
    const { container } = render(
      <Button busy icon={<Check data-testid="icon" />}>
        Apply
      </Button>,
    );
    advance(LOAD_DELAY_MS);
    const slot = container.querySelector(".btn-slot") as HTMLElement;
    expect(slot.dataset.spinning).toBe("true");
    // The icon stays in the slot (hidden by CSS, still taking its room) beside the spinner.
    expect(slot.querySelector('[data-testid="icon"]')).not.toBeNull();
    expect(slot.querySelector(".spin")).not.toBeNull();
    expect(screen.getByRole("button").textContent).toBe("Apply");
  });

  it("with no icon the spinner sits beside the label and the label text stays", () => {
    const { container } = render(<Button busy>Apply</Button>);
    advance(LOAD_DELAY_MS);
    const button = screen.getByRole("button") as HTMLButtonElement;
    expect(button.textContent).toBe("Apply");
    expect(button.dataset.spin).toBe("lead");
    const kids = [...button.childNodes];
    // The spinner is in the flow before the label: nothing is laid over the words.
    expect((kids[0] as Element).classList.contains("spin")).toBe(true);
    expect(kids[1]?.textContent).toBe("Apply");
    expect(container.querySelector(".btn-over")).toBeNull();
  });

  it("locks its width while busy: the idle width is set when busy starts and released when it ends", () => {
    const measure = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ width: 123.5, height: 30 } as DOMRect);
    const { rerender } = render(<Button>Apply</Button>);
    const button = screen.getByRole("button") as HTMLButtonElement;
    expect(button.style.width).toBe("");
    rerender(<Button busy>Apply</Button>);
    expect(button.style.width).toBe("123.5px");
    advance(LOAD_DELAY_MS);
    // The spinner appears; the width the button was measured at is still the width it is held to.
    expect(button.style.width).toBe("123.5px");
    expect(button.textContent).toBe("Apply");
    rerender(<Button>Apply</Button>);
    expect(button.style.width).toBe("");
    measure.mockRestore();
  });

  it("does not fire onClick while busy, and fires it again after", () => {
    const onClick = vi.fn();
    const { rerender } = render(
      <Button busy onClick={onClick}>
        Save
      </Button>,
    );
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).not.toHaveBeenCalled();
    rerender(<Button onClick={onClick}>Save</Button>);
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("leaves a plain disabled button without aria-busy or a spinner", () => {
    const { container } = render(<Button disabled>Save</Button>);
    advance(LOAD_DELAY_MS * 2);
    expect(screen.getByRole("button").getAttribute("aria-busy")).toBeNull();
    expect(spinner(container)).toBeNull();
  });
});

describe("the busy IconButton and BusyIcon", () => {
  it("keeps its name, is aria-busy and aria-disabled (still focusable), and spins after the delay", () => {
    const { container } = render(<IconButton label="Rescan" icon={<FloppyDisk />} tip={false} busy onClick={() => undefined} />);
    const button = screen.getByRole("button", { name: "Rescan" }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    expect(button.getAttribute("aria-disabled")).toBe("true");
    expect(button.getAttribute("aria-busy")).toBe("true");
    expect(spinner(container)).toBeNull();
    advance(LOAD_DELAY_MS);
    expect(spinner(container)).not.toBeNull();
  });

  it("BusyIcon swaps in the spinner only after the delay", () => {
    const { container } = render(<BusyIcon icon={<Check />} busy />);
    expect(spinner(container)).toBeNull();
    advance(LOAD_DELAY_MS);
    expect(spinner(container)).not.toBeNull();
  });
});

describe("InlineSkeleton", () => {
  it("holds its width unseen until the delay, then shows; it is a drawing, aria-hidden, not a status", () => {
    const { container } = render(<InlineSkeleton width="12ch" />);
    const line = container.querySelector(".inline-skel") as HTMLElement;
    expect(line.style.width).toBe("12ch");
    expect(line.dataset.shown).toBe("false");
    expect(line.getAttribute("aria-hidden")).toBe("true");
    expect(line.getAttribute("role")).toBeNull();
    expect(line.getAttribute("aria-busy")).toBeNull();
    advance(LOAD_DELAY_MS);
    expect(line.dataset.shown).toBe("true");
    expect(line.style.width).toBe("12ch");
  });

  it("draws nothing when inactive", () => {
    const { container } = render(<InlineSkeleton active={false} />);
    expect(container.querySelector(".inline-skel")).toBeNull();
  });
});

describe("Select loading", () => {
  const options = [
    { value: "a", label: "Alpha" },
    { value: "b", label: "Beta" },
  ];

  it("is disabled and aria-busy, and keeps its value until the delay: a fast load blanks nothing", () => {
    const { container } = render(<Select label="Source" value="a" options={options} onChange={() => undefined} loading />);
    const trigger = screen.getByRole("combobox", { name: "Source" }) as HTMLButtonElement;
    expect(trigger.disabled).toBe(true);
    expect(trigger.getAttribute("aria-busy")).toBe("true");
    const visible = () => [...container.querySelectorAll(".select-value > .one-line")].filter((n) => n.getAttribute("data-on") === "true");
    // Before the delay the value is still drawn and there is no skeleton.
    expect(visible().map((n) => n.textContent)).toEqual(["Alpha"]);
    expect(container.querySelector(".inline-skel")).toBeNull();
    advance(LOAD_DELAY_MS - 1);
    expect(visible()).toHaveLength(1);
    advance(1);
    // After it the skeleton stands in for the value.
    expect(container.querySelector(".inline-skel")).not.toBeNull();
    expect(visible()).toHaveLength(0);
  });

  it("keeps the options' room so the trigger does not resize when they arrive", () => {
    const { container } = render(<Select label="Source" value="a" options={options} onChange={() => undefined} loading />);
    const texts = [...container.querySelectorAll(".select-value .one-line")].map((n) => n.textContent);
    expect(texts).toEqual(["Alpha", "Beta"]);
  });

  it("does not open while loading, and is a normal select after", () => {
    const { rerender } = render(<Select label="Source" value="a" options={options} onChange={() => undefined} loading />);
    fireEvent.click(screen.getByRole("combobox"));
    expect(screen.queryByRole("listbox")).toBeNull();
    rerender(<Select label="Source" value="a" options={options} onChange={() => undefined} />);
    const trigger = screen.getByRole("combobox") as HTMLButtonElement;
    expect(trigger.disabled).toBe(false);
    expect(trigger.getAttribute("aria-busy")).toBeNull();
  });
});

describe("a thumbnail whose fetch fails", () => {
  it("ends the wait: a still no-picture tile, no skeleton, aria-busy off", async () => {
    vi.useRealTimers();
    vi.resetModules();
    vi.doMock("@/lib/thumbs", () => ({ loadThumb: () => Promise.resolve(null), blit: () => undefined }));
    const { Thumb } = await import("@/components/Thumb");
    const { container } = render(<Thumb id="x" label="Comet" />);
    await waitFor(() => expect(container.querySelector(".thumb-none")).not.toBeNull());
    const tile = container.querySelector(".thumb") as HTMLElement;
    expect(tile.getAttribute("aria-busy")).toBeNull();
    expect(tile.dataset.failed).toBe("true");
    expect(container.querySelector(".canvas-skel")).toBeNull();
    vi.doUnmock("@/lib/thumbs");
    vi.useFakeTimers();
  });

  it("EffectPreview of an effect whose thumbnail fetch fails is a still box", async () => {
    vi.useRealTimers();
    vi.resetModules();
    vi.doMock("@/lib/thumbs", () => ({ loadThumb: () => Promise.resolve(null), blit: () => undefined }));
    const { EffectPreview } = await import("@/components/EffectPreview");
    const { container } = render(<EffectPreview id="x" />);
    await waitFor(() => expect((container.querySelector("canvas") as HTMLCanvasElement).dataset.wait).toBe("failed"));
    vi.doUnmock("@/lib/thumbs");
    vi.useFakeTimers();
  });
});

describe("canvas placeholders", () => {
  it("CanvasSkeleton fills its tile and shows after the delay", () => {
    const { container } = render(<CanvasSkeleton />);
    const skel = container.querySelector(".canvas-skel") as HTMLElement;
    expect(skel.dataset.shown).toBe("false");
    advance(LOAD_DELAY_MS);
    expect(skel.dataset.shown).toBe("true");
  });

  it("LoopPreview with a failed preview is a still box: no skeleton, no aria-busy", () => {
    const { container } = render(<LoopPreview thumb={null} label="Preview of Comet" failed />);
    advance(LOAD_DELAY_MS * 3);
    const canvas = container.querySelector("canvas") as HTMLCanvasElement;
    expect(canvas.dataset.wait).toBe("failed");
    expect(canvas.getAttribute("aria-busy")).toBeNull();
  });

  it("LoopPreview waits as a held box, then a skeleton, then nothing once it has a loop", () => {
    const { container, rerender } = render(<LoopPreview thumb={null} label="Preview of Comet" />);
    const canvas = container.querySelector("canvas") as HTMLCanvasElement;
    expect(canvas.dataset.wait).toBe("held");
    expect(canvas.getAttribute("aria-busy")).toBe("true");
    advance(LOAD_DELAY_MS);
    expect(canvas.dataset.wait).toBe("shown");
    rerender(<LoopPreview thumb={{ w: 1, h: 1, frames: 1, data: new Uint8Array(3) }} label="Preview of Comet" />);
    expect(canvas.dataset.wait).toBeUndefined();
    expect(canvas.getAttribute("aria-busy")).toBeNull();
  });
});

describe("the kit's styles", () => {
  const css = (name: string) => readFileSync(resolve(process.cwd(), "src/styles", name), "utf8");

  it("reduced motion zeroes every animation duration, the spinner's included", () => {
    const app = css("app.css");
    expect(app).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*\*,[^}]*animation-duration: 0ms !important/);
    expect(app).toMatch(/html\[data-motion="reduced"\] \*,[^}]*animation-duration: 0ms !important/);
    // The spinner is the one infinite animation, and it is the shared `.spin` rule.
    expect(css("motion.css")).toMatch(/\.spin \{\s*animation: canvas-spin 900ms linear infinite;/);
  });

  it("the busy button keeps its cell: icon and spinner share one grid cell", () => {
    const loading = css("loading.css");
    expect(loading).toMatch(/\.btn-slot > \* \{\s*grid-area: 1 \/ 1;/);
    expect(loading).toMatch(/\.btn\[data-spin="lead"\] \{[^}]*justify-content: center;/);
  });

  it("every colour in the kit is a token", () => {
    // Comments mention issue numbers ("#258"), which are not colours.
    const loading = css("loading.css").replace(/\/\*[\s\S]*?\*\//g, "");
    expect(loading).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(loading).not.toMatch(/\brgba?\(/);
  });
});
