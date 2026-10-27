"use client";

import Link from "next/link";
import { useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import type { Icon } from "@phosphor-icons/react";
import { Cpu, Fan, Gear, SquaresFour, Sparkle } from "@phosphor-icons/react";

interface NavItem {
  href: string;
  label: string;
  icon: Icon;
}

export const NAV: NavItem[] = [
  { href: "/", label: "Layout", icon: SquaresFour },
  { href: "/effects/", label: "Effects", icon: Sparkle },
  { href: "/devices/", label: "Devices", icon: Cpu },
  { href: "/fans/", label: "Fans", icon: Fan },
  { href: "/settings/", label: "Settings", icon: Gear },
];

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
 * Left rail on wide screens; a bottom bar below 720 px. The active page carries the live
 * hue, and the marker that shows it slides from one link to the next.
 */
export function Rail({ pathname }: { pathname: string }) {
  const navRef = useRef<HTMLElement>(null);
  const [marker, setMarker] = useState<MarkerBox | null>(null);

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
      const next: MarkerBox = {
        mx: ar.left - 6 - nr.left - nav.clientLeft,
        my: ar.top + 8 - nr.top - nav.clientTop,
        mh: Math.max(10, ar.height - 16),
        mxn: ar.left - nr.left - nav.clientLeft,
        myn: ar.top - 5 - nr.top - nav.clientTop,
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

  return (
    <nav className="rail" aria-label="Pages" ref={navRef}>
      <span className="rail-marker" data-on={marker ? "true" : "false"} style={markerStyle} aria-hidden="true" />
      <div className="rail-brand">
        <span className="rail-brand-dot" aria-hidden="true" />
        canvas-rgb
      </div>
      {NAV.map((item) => {
        const Glyph = item.icon;
        const current = item.href === pathname;
        return (
          <Link
            key={item.href}
            href={item.href}
            prefetch={false}
            className="rail-link"
            aria-current={current ? "page" : undefined}
          >
            <span className="rail-icon" aria-hidden="true">
              <Glyph size={20} weight={current ? "fill" : "regular"} />
            </span>
            {item.label}
          </Link>
        );
      })}
    </nav>
  );
}
