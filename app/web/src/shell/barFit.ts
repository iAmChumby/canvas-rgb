// What the status strip's row needs, measured (#586). The strip and a user-named title share these,
// so "does it fit" has one answer. Widths are `offsetWidth`: an entrance animation's transform
// must not change the decision.

/** The shortest a user-named title may be drawn before it gives way (#586 R1). */
export const USER_TITLE_FLOOR = 12;

/** A name cut in the middle to `keep` characters. Code points, so an emoji is never split. */
export function middleCut(title: string, keep: number): string {
  const chars = Array.from(title);
  if (keep >= chars.length) return title;
  const left = Math.ceil(keep / 2);
  return `${chars.slice(0, left).join("")}…${chars.slice(chars.length - (keep - left)).join("")}`;
}

/** A measurer for text drawn in `el`'s font, or null where there is no canvas. */
export function textMeasurer(el: Element): ((text: string) => number) | null {
  const ctx = document.createElement("canvas").getContext("2d");
  if (!ctx) return null;
  const cs = getComputedStyle(el);
  ctx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
  return (text) => ctx.measureText(text).width;
}

const px = (value: string) => parseFloat(value) || 0;

/** The row's items. The title's slot is `display: contents`, so its children stand in for it. */
export function rowItems(row: HTMLElement): HTMLElement[] {
  return ([...row.children] as HTMLElement[]).flatMap((el) =>
    el.classList.contains("strip-slot") ? ([...el.children] as HTMLElement[]) : [el],
  );
}

/** The width these children of `box` take side by side: widths, margins and the gaps between. */
export function widthOf(box: HTMLElement, items: HTMLElement[]): number {
  const drawn = items.filter((el) => el.offsetWidth > 0);
  const gap = px(getComputedStyle(box).columnGap);
  return drawn.reduce((sum, el) => {
    const cs = getComputedStyle(el);
    // The controls' left margin is `auto`: it is the row's spare room, not a need.
    const left = el.classList.contains("strip-controls") ? 0 : px(cs.marginLeft);
    return sum + el.offsetWidth + left + px(cs.marginRight);
  }, gap * Math.max(0, drawn.length - 1));
}

export type BarFit = {
  /** Where the page's one-line context is drawn. */
  context: "bar" | "body";
  /** Where the page's secondary keys are drawn. */
  secondary: "bar" | "menu";
  rows: "1" | "2";
};

/**
 * One rule for everything that can leave the row (#586 B9, B10, B12), in priority order: the
 * context leaves first, then the secondary keys go to the overflow, then a name a person typed is
 * cut, and only then does a primary key that still cannot fit take a second row. Each step is taken
 * only when the row does not fit without it, so a thing is in the bar whenever it fits there.
 *
 * It must run with `data-measure` on the strip (nothing shrinks), and it leaves the attributes it
 * tried in place; the caller publishes the answer.
 */
export function fitBar(strip: HTMLElement, row: HTMLElement): BarFit {
  const root = document.documentElement;
  const answer: BarFit = { context: "bar", secondary: "bar", rows: "1" };
  const apply = () => {
    root.dataset.barContext = answer.context;
    root.dataset.barSecondary = answer.secondary;
    strip.dataset.barRows = answer.rows;
  };
  apply();
  const name = row.querySelector<HTMLElement>("[data-bar-user-title]");
  const measure = name ? textMeasurer(name) : null;
  const full = name?.getAttribute("title") ?? "";
  const fits = (items: () => HTMLElement[], title = full) => {
    // The heading may be drawn cut right now: count the name the step asks about instead.
    const extra = name && measure ? Math.ceil(measure(title)) - name.offsetWidth : 0;
    return widthOf(row, items()) + extra <= row.clientWidth + 0.5;
  };
  const all = () => rowItems(row);
  if (!fits(all)) {
    answer.context = "body";
    apply();
  }
  if (!fits(all)) {
    answer.secondary = "menu";
    apply();
  }
  const primary = row.querySelector("[data-bar-primary]");
  if (!primary || fits(all, middleCut(full, USER_TITLE_FLOOR))) return answer;
  answer.rows = "2";
  apply();
  // Only a stylesheet that wraps the row gives it a second line (the phone's).
  if (getComputedStyle(row).getPropertyValue("--bar-wraps").trim() !== "1") {
    answer.rows = "1";
    apply();
    return answer;
  }
  // With the keys on their own line, both lines are asked again.
  answer.context = "bar";
  answer.secondary = "bar";
  apply();
  if (!fits(() => rowItems(row).filter((el) => !el.classList.contains("strip-page-actions")))) {
    answer.context = "body";
    apply();
  }
  const actions = row.querySelector<HTMLElement>(".rd-actions");
  if (actions && actions.offsetWidth > row.clientWidth + 0.5) {
    answer.secondary = "menu";
    apply();
  }
  return answer;
}
