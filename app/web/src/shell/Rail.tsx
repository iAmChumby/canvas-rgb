"use client";

import Link from "next/link";
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

/** Left rail on wide screens; a bottom bar below 720 px. The active page carries the live hue. */
export function Rail({ pathname }: { pathname: string }) {
  return (
    <nav className="rail" aria-label="Pages">
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
