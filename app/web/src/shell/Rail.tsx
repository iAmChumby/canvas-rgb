"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import type { Icon } from "@phosphor-icons/react";
import { Fan, FilmSlate, GameController, Gear, Monitor, Circuitry, Question, Sparkle } from "@phosphor-icons/react";
import { BrandMark, Wordmark } from "@/components/BrandMark";
import { useSnap } from "@/lib/store";
import { useRailData, type RailValues } from "@/shell/RailData";
import { hasNavigationGuard, requestNavigation } from "@/lib/navigationGuard";
import { boundsOf, placeTooltip } from "@/lib/tooltipPlacement";
import { RailDeck } from "@/shell/RailDeck";

export interface NavItem {
  href: string;
  label: string;
  icon: Icon;
}

/** The pages, in the order the rail shows them. */
const PAGES: NavItem[] = [
  // What the lights are doing, grouped the way the owner grouped them: the app opens here.
  { href: "/", label: "Scenes", icon: FilmSlate },
  { href: "/effects/", label: "Effects", icon: Sparkle },
  // What the daemon found on this machine: the board, the sticks, the card, the keyboard.
  // #57: the upright circuitry glyph, not the diagonal Plugs: the page is the wiring, not
  // whatever happens to be plugged in.
  { href: "/devices/", label: "Devices", icon: Circuitry },
  // The panels that show numbers: a pump head, an AIO screen. A monitor says "a display", where
  // a thermometer said "a temperature".
  { href: "/displays/", label: "Displays", icon: Monitor },
  { href: "/peripherals/", label: "Peripherals", icon: GameController },
  { href: "/fans/", label: "Fans", icon: Fan },
];

/** The settings row, which sits under the deck. */
const SETTINGS: NavItem = { href: "/settings/", label: "Settings", icon: Gear };

/** The help row, beside Settings: one click from any page, and in the palette for every width. */
export const HELP: NavItem = { href: "/help/", label: "Help & About", icon: Question };

/** Every page, in the order the command palette offers them. */
export const NAV: NavItem[] = [...PAGES, SETTINGS, HELP];

interface MarkerBox {
  /** Wide layout: the bar sits on the rail's left edge, level with the active link. */
  mx: number;
  my: number;
  mh: number;
  /** Narrow layout: the bar sits above the active link in the bottom bar. */
  mxn: number;
  myn: number;
  mw: number;
}

/**
 * The left rail: the name and the version, one row per page with what that page is doing now,
 * the deck (the lights and the machine), the settings row, and the engine's own line.
 *
 * With labels from 1280 px, icons only from 721 px, a bottom bar at 720 px and below. The
 * active page carries the live hue, and the marker that shows it slides from one link to the
 * next.
 */
export function Rail({ pathname }: { pathname: string }) {
  const navRef = useRef<HTMLElement>(null);
  const [marker, setMarker] = useState<MarkerBox | null>(null);
  const values = useRailData();
  const status = useSnap((s) => s.status);
  const statusFault = useSnap((s) => s.statusFault);

  useLayoutEffect(() => {
    const nav = navRef.current;
    if (!nav) return;
    const measure = () => {
      const active = nav.querySelector<HTMLElement>('[aria-current="page"]');
      if (!active) {
        setMarker(null);
        return;
      }
      const nr = nav.getBoundingClientRect();
      const ar = active.getBoundingClientRect();
      // On a phone the mark sits just above the active item's icon: the tinted tile hugs the
      // icon and the name under it, so the mark has to follow the icon, not the whole link.
      const icon = active.querySelector<HTMLElement>(".rail-icon");
      const ir = icon ? icon.getBoundingClientRect() : ar;
      const next: MarkerBox = {
        mx: ar.left - 6 - nr.left - nav.clientLeft,
        my: ar.top + 8 - nr.top - nav.clientTop,
        mh: Math.max(10, ar.height - 16),
        mxn: ar.left - nr.left - nav.clientLeft,
        myn: ir.top - 5 - nr.top - nav.clientTop,
        mw: ar.width,
      };
      setMarker((prev) => {
        if (
          prev &&
          Math.abs(prev.mx - next.mx) < 0.5 &&
          Math.abs(prev.my - next.my) < 0.5 &&
          Math.abs(prev.mh - next.mh) < 0.5 &&
          Math.abs(prev.mxn - next.mxn) < 0.5 &&
          Math.abs(prev.myn - next.myn) < 0.5 &&
          Math.abs(prev.mw - next.mw) < 0.5
        ) {
          return prev;
        }
        return next;
      });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(nav);
    const links = Array.from(nav.querySelectorAll<HTMLElement>(".rail-link"));
    for (const el of links) ro.observe(el);

    // The icon rail keeps each link's name as a ::after label instead of drawing the label.
    // That label is fixed to the window, so it stays out of the rail's scroll box (#467); put
    // its window coordinates on the link when it is about to show, on hover or focus.
    const placeTip = (link: HTMLElement) => {
      const s = getComputedStyle(link, "::after");
      const width = Number.parseFloat(s.width);
      const height = Number.parseFloat(s.height);
      if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) return;
      const at = placeTooltip({
        trigger: link.getBoundingClientRect(),
        bubble: { width, height },
        bounds: boundsOf({
          width: document.documentElement.clientWidth,
          height: document.documentElement.clientHeight,
        }),
        side: "right",
        align: "center",
        gap: 10,
      });
      link.style.setProperty("--tip-x", `${at.left}px`);
      link.style.setProperty("--tip-y", `${at.top}px`);
      // #478: mark the label as placed only now. The CSS shows the ::after label on hover/focus
      // only with this mark, so the fixed label is invisible at the window-corner fallback
      // (`left: var(--tip-x, 0px)`) until its window coordinates are written. Both happen in this
      // one task, so no painted frame shows the unplaced corner position.
      link.dataset.tipPlaced = "true";
    };
    const showTip = new Map<HTMLElement, () => void>();
    for (const el of links) {
      const on = () => placeTip(el);
      showTip.set(el, on);
      el.addEventListener("pointerenter", on);
      el.addEventListener("focus", on);
    }
    return () => {
      ro.disconnect();
      for (const [el, on] of showTip) {
        el.removeEventListener("pointerenter", on);
        el.removeEventListener("focus", on);
      }
    };
  }, [pathname]);

  const markerStyle = marker
    ? ({
        "--mx": `${marker.mx}px`,
        "--my": `${marker.my}px`,
        "--mh": `${marker.mh}px`,
        "--mxn": `${marker.mxn}px`,
        "--myn": `${marker.myn}px`,
        "--mw": `${marker.mw}px`,
      } as CSSProperties)
    : undefined;

  // The engine's own line: the daemon's word, and the rate it is running at.
  // The daemon that answered and said its render engine is not answering (#241) is up:
  // its engine is the part that is not answering.
  const engine = status
    ? "engine"
    : statusFault === "engine"
      ? "engine down"
      : statusFault === "down"
        ? "offline"
        : "connecting";
  const engineTone = status
    ? "ok"
    : statusFault === "engine"
      ? "warn"
      : statusFault === "down"
        ? "down"
        : "work";
  const fps = status ? Math.round(status.fps_measured) : null;

  return (
    <nav className="rail" aria-label="Pages" ref={navRef}>
      <span className="rail-marker" data-on={marker ? "true" : "false"} style={markerStyle} aria-hidden="true" />

      <div className="rail-brand">
        <BrandMark />
        <span className="rail-label">
          <Wordmark />
        </span>
        <span className="rail-version fig">{status?.version ?? "—"}</span>
      </div>

      {PAGES.map((item) => (
        <RailLink key={item.href} item={item} current={item.href === pathname}>
          <PageValue href={item.href} values={values} />
        </RailLink>
      ))}

      {/* The deck sits at the bottom of the rail, above the settings row. */}
      <div className="rail-spacer" />

      <RailDeck />

      <RailLink item={SETTINGS} current={SETTINGS.href === pathname} />
      {/* The eighth dock item. The dock gate (R5) may hide it at phone widths; NAV keeps the
          palette door at every size, and Settings carries a row there as well. */}
      <RailLink item={HELP} current={HELP.href === pathname} className="rail-help" />

      <div className="rail-status">
        <span className="rail-status-dot" data-tone={engineTone} aria-hidden="true" />
        <span className="rail-status-word">{engine}</span>
        <span className="rail-status-fps">{fps === null ? "—" : `${fps} fps`}</span>
      </div>
    </nav>
  );
}

function RailLink({ item, current, children, className }: { item: NavItem; current: boolean; children?: ReactNode; className?: string }) {
  const Glyph = item.icon;
  const router = useRouter();
  return (
    <Link
      href={item.href}
      prefetch={false}
      className={className ? `rail-link ${className}` : "rail-link"}
      data-label={item.label}
      aria-current={current ? "page" : undefined}
      onClick={(e) => {
        // An editor with unsaved code asks before the page goes. Without a guard the link
        // navigates as it always did.
        if (!hasNavigationGuard()) return;
        e.preventDefault();
        requestNavigation(() => router.push(item.href));
      }}
    >
      <span className="rail-icon" aria-hidden="true">
        <Glyph size={18} weight={current ? "fill" : "regular"} />
      </span>
      <span className="rail-label">{item.label}</span>
      {children}
    </Link>
  );
}

/**
 * A value is shown whole or not at all (#586 R5). Names are user-chosen and of any length, so no
 * CSS width can decide. The observer measures the value against its own box; while it is hidden
 * the full words stay in `title` and in the accessible name. `visibility: hidden` keeps the box
 * and its measurement stable, so the observer never oscillates.
 */
function RailValue({ value, className, children }: { value: string; className?: string; children?: ReactNode }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [fits, setFits] = useState(true);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setFits(el.scrollWidth <= el.clientWidth);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [value]);
  return (
    <span
      ref={ref}
      className={className ? `rail-value ${className}` : "rail-value"}
      data-fits={fits ? "true" : "false"}
      title={value}
      aria-label={value}
    >
      {children ?? value}
    </span>
  );
}

/**
 * The value at the end of a row: what that page is doing now. A value the daemon has not
 * given is left out, so the row says nothing rather than something made up.
 */
function PageValue({ href, values }: { href: string; values: RailValues }) {
  if (href === "/") {
    return values.scene ? <RailValue value={values.scene} /> : null;
  }
  if (href === "/effects/") {
    if (!values.effect) return null;
    return (
      <RailValue value={values.effect} className="rail-value-name">
        {/* The light's own dot: it is lit while the lights run, and dim while they are paused. */}
        <span className="rail-led" data-on={values.paused ? "off" : "live"} aria-hidden="true" />
        {values.effect}
      </RailValue>
    );
  }
  if (href === "/devices/") {
    if (!values.devices) return null;
    const value = `${values.devices.connected}/${values.devices.total}`;
    return (
      <RailValue value={value}>
        <span className="rail-value-ok">{values.devices.connected}</span>/{values.devices.total}
      </RailValue>
    );
  }
  if (href === "/displays/") {
    return values.displays === null ? null : <RailValue value={String(values.displays)} />;
  }
  if (href === "/peripherals/") {
    return values.peripherals === null ? null : <RailValue value={String(values.peripherals)} />;
  }
  if (href === "/fans/") {
    return values.fans ? <RailValue value={values.fans} /> : null;
  }
  return null;
}
