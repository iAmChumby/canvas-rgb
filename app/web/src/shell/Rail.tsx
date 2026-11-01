"use client";

import Link from "next/link";
import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import type { Icon } from "@phosphor-icons/react";
import { Fan, GameController, Gear, Monitor, Plugs, SquaresFour, Sparkle } from "@phosphor-icons/react";
import { useToasts } from "@/components/Toast";
import { BrandMark, Wordmark } from "@/components/BrandMark";
import { useSnap } from "@/lib/store";
import { useRailData, type RailValues } from "@/shell/RailData";
import { RailDeck } from "@/shell/RailDeck";

export interface NavItem {
  href: string;
  label: string;
  icon: Icon;
}

/** The pages, in the order the rail shows them. */
const PAGES: NavItem[] = [
  { href: "/", label: "Layout", icon: SquaresFour },
  { href: "/effects/", label: "Effects", icon: Sparkle },
  // What the daemon found on this machine: the board, the sticks, the card, the keyboard. Plugs
  // says "the things plugged in", where a chip only said "the processor".
  { href: "/devices/", label: "Devices", icon: Plugs },
  // The panels that show numbers: a pump head, an AIO screen. A monitor says "a display", where
  // a thermometer said "a temperature".
  { href: "/displays/", label: "Displays", icon: Monitor },
  { href: "/peripherals/", label: "Peripherals", icon: GameController },
  { href: "/fans/", label: "Fans", icon: Fan },
];

/** The settings row, which sits under the deck. */
const SETTINGS: NavItem = { href: "/settings/", label: "Settings", icon: Gear };

/** Every page, in the order the command palette offers them. */
export const NAV: NavItem[] = [...PAGES, SETTINGS];

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
  // A toast on a phone takes a band of the bottom dock: the dock grows by exactly that much, so
  // the toast is over the app's own chrome and hides no control.
  const toast = useToasts();
  const values = useRailData();
  const status = useSnap((s) => s.status);
  const statusError = useSnap((s) => s.statusError);

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
    for (const el of Array.from(nav.querySelectorAll(".rail-link"))) ro.observe(el);
    return () => ro.disconnect();
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
  const engine = status ? "engine" : statusError ? "offline" : "connecting";
  const engineTone = status ? "ok" : statusError ? "down" : "work";
  const fps = status ? Math.round(status.fps_measured) : null;

  return (
    <nav className="rail" aria-label="Pages" ref={navRef} data-toast={toast.items.length > 0 ? "on" : "off"}>
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

      <div className="rail-status">
        <span className="rail-status-dot" data-tone={engineTone} aria-hidden="true" />
        <span className="rail-status-word">{engine}</span>
        <span className="rail-status-fps">{fps === null ? "—" : `${fps} fps`}</span>
      </div>
    </nav>
  );
}

function RailLink({ item, current, children }: { item: NavItem; current: boolean; children?: ReactNode }) {
  const Glyph = item.icon;
  return (
    <Link
      href={item.href}
      prefetch={false}
      className="rail-link"
      data-label={item.label}
      aria-current={current ? "page" : undefined}
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
 * The value at the end of a row: what that page is doing now. A value the daemon has not
 * given is left out, so the row says nothing rather than something made up.
 */
function PageValue({ href, values }: { href: string; values: RailValues }) {
  if (href === "/") {
    return values.layout ? <span className="rail-value">{values.layout}</span> : null;
  }
  if (href === "/effects/") {
    if (!values.effect) return null;
    return (
      <span className="rail-value rail-value-name">
        {/* The light's own dot: it is lit while the lights run, and dim while they are paused. */}
        <span className="rail-led" data-on={values.paused ? "off" : "live"} aria-hidden="true" />
        {values.effect}
      </span>
    );
  }
  if (href === "/devices/") {
    if (!values.devices) return null;
    return (
      <span className="rail-value">
        <span className="rail-value-ok">{values.devices.connected}</span>/{values.devices.total}
      </span>
    );
  }
  if (href === "/displays/") {
    return values.displays === null ? null : <span className="rail-value">{values.displays}</span>;
  }
  if (href === "/peripherals/") {
    return values.peripherals === null ? null : <span className="rail-value">{values.peripherals}</span>;
  }
  if (href === "/fans/") {
    return values.fans ? <span className="rail-value">{values.fans}</span> : null;
  }
  return null;
}
