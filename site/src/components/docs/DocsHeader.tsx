"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { GithubLogo } from "@phosphor-icons/react";
import { Wordmark } from "@editor/components/BrandMark";
import { Tooltip } from "@editor/components/Tooltip";
import { ThemeToggle } from "../ThemeToggle";
import { DocsSearch } from "./DocsSearch";

const REPO = "https://github.com/iAmChumby/canvas-rgb";

const LINKS = [
  { href: "/docs", label: "Overview" },
  { href: "/docs/getting-started", label: "Start here" },
  { href: "/docs/api", label: "API" },
  { href: "/docs/cli", label: "CLI" },
  { href: "/docs/effects", label: "Effects" },
];

/** The site's header, with the docs navigation and the one search field. */
export function DocsHeader() {
  const pathname = usePathname() ?? "";
  return (
    <header className="site-header">
      <div className="site-header-inner">
        <Link className="site-brand" href="/docs" aria-label="canvas-rgb docs home">
          <Wordmark />
        </Link>
        <nav className="site-nav" aria-label="Documentation">
          {LINKS.map((link) => {
            const active = link.href === "/docs" ? pathname === "/docs" : pathname.startsWith(link.href);
            return (
              <Link
                key={link.href}
                className="site-nav-link"
                href={link.href}
                aria-current={active ? "page" : undefined}
              >
                {link.label}
              </Link>
            );
          })}
        </nav>
        <div className="site-header-actions">
          <DocsSearch />
          <Tooltip label="canvas-rgb on GitHub">
            <a className="ibtn site-github" href={REPO} aria-label="canvas-rgb on GitHub">
              <GithubLogo size={18} />
            </a>
          </Tooltip>
          <ThemeToggle />
        </div>
      </div>
    </header>
  );
}
