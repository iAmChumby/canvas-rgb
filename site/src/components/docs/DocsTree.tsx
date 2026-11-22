"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { CaretDown, MagnifyingGlass } from "@phosphor-icons/react";
import { Button } from "@editor/components/Button";
import { AREA_KEYS } from "../../lib/area-keys.mjs";
import type { AreaGroup, CliGroup } from "../../lib/docs-data.mjs";
import type { Heading } from "../../lib/markdown.mjs";

type TreeProps = {
  mode: "routes" | "commands" | "sections";
  currentUrl: string;
  areaGroups?: AreaGroup[];
  cliGroups?: CliGroup[];
  sections?: Heading[];
};

function methodTone(method: string) {
  return method.toUpperCase();
}

/** A route path, with a break opportunity after each slash so the tree wraps at `/` boundaries. */
function pathWithBreaks(routePath: string) {
  const segments = routePath.split("/");
  return segments.map((segment, index) => (
    <span key={index}>
      {index === 0 ? segment : `/${segment}`}
      {index < segments.length - 1 ? <wbr /> : null}
    </span>
  ));
}

export function DocsTree({ mode, currentUrl, areaGroups = [], cliGroups = [], sections = [] }: TreeProps) {
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState(false);
  const query = filter.trim().toLowerCase();
  const label = mode === "routes" ? "Routes" : mode === "commands" ? "Commands" : "Sections";

  const shownAreas = useMemo(
    () =>
      areaGroups
        .map((group) => ({
          ...group,
          routes: group.routes.filter((route) =>
            `${route.methods.join(" ")} ${route.path} ${route.area}`.toLowerCase().includes(query),
          ),
        }))
        .filter((group) => group.routes.length > 0),
    [areaGroups, query],
  );
  const shownCommands = useMemo(
    () =>
      cliGroups
        .map((group) => ({
          ...group,
          commands: group.commands.filter((command) =>
            `${command.title} ${command.description}`.toLowerCase().includes(query),
          ),
        }))
        .filter((group) => group.commands.length > 0),
    [cliGroups, query],
  );
  const shownSections = sections.filter((section) => section.text.toLowerCase().includes(query));

  return (
    <nav className="a-tree" aria-label={`${label} tree`}>
      <Button
        className="a-tree-summary"
        size="sm"
        aria-expanded={open}
        icon={<CaretDown size={12} className={open ? "is-open" : ""} aria-hidden="true" />}
        onClick={() => setOpen((was) => !was)}
      >
        {label}
      </Button>
      <div className={`a-tree-body ${open ? "is-open" : ""}`}>
        <div className="a-tree-head">
          <span className="eyebrow">{label}</span>
          <span className="led" data-on={mode === "commands" ? "a5" : "a1"} />
        </div>
        {mode !== "sections" ? (
          <label className="a-tree-filter">
            <MagnifyingGlass size={14} aria-hidden="true" />
            <span className="sr-only">Filter {label.toLowerCase()}</span>
            <input
              type="search"
              value={filter}
              placeholder={`Filter ${label.toLowerCase()}`}
              onChange={(event) => setFilter(event.target.value)}
            />
            <kbd className="kbd">/</kbd>
          </label>
        ) : null}

        {mode === "routes" ? (
          <>
            <div className="ac-areas">
              <div className="a-tree-head">
                <span className="eyebrow">Areas</span>
                <span className="led" data-on="a1" />
              </div>
              <div className="c-board-keys ac-area-keys" aria-label="Area shortcuts">
                {AREA_KEYS.map((key) => {
                  const first = areaGroups.find((group) => key.areas.includes(group.area));
                  return (
                    <a className="c-key" key={key.label} href={first ? `#area-${first.key.toLowerCase()}` : "#"}>
                      {key.label}
                    </a>
                  );
                })}
              </div>
            </div>
            {shownAreas.map((group) => (
              <div className="a-tree-group" key={group.area} id={`area-${group.key.toLowerCase()}`}>
                <div className="a-tree-group-title">{group.area}</div>
                <ul className="a-tree-list">
                  {group.routes.flatMap((route) =>
                    route.methods.map((method) => {
                      const href = `/docs/api/${route.slug}`;
                      return (
                        <li key={`${route.slug}-${method}`}>
                          <Link
                            className="a-tree-link"
                            href={href}
                            aria-current={currentUrl === href ? "page" : undefined}
                          >
                            <span className="a-method-mini" data-m={methodTone(method)}>
                              {method}
                            </span>
                            <span>{pathWithBreaks(route.path.replace(/^\/api\/v1/, "") || "/")}</span>
                          </Link>
                        </li>
                      );
                    }),
                  )}
                </ul>
              </div>
            ))}
            {shownAreas.length === 0 ? <p className="a-tree-note">No routes match that filter.</p> : null}
            <div className="a-tree-note">
              <strong>Auth:</strong> bearer on every route except <span className="mono">GET /status</span>.
            </div>
          </>
        ) : null}

        {mode === "commands" ? (
          <>
            {shownCommands.map((group) => (
              <div className="a-tree-group" key={group.slug} id={`command-${group.slug}`}>
                <div className="a-tree-group-title">{group.title}</div>
                <ul className="a-tree-list">
                  {group.commands.map((command) => {
                    const href = `/docs/cli/${command.slug}`;
                    return (
                      <li key={command.slug}>
                        <Link
                          className="a-tree-link"
                          href={href}
                          aria-current={currentUrl === href ? "page" : undefined}
                        >
                          <span className="a-cmd-chip">{group.title}</span>
                          <span>{command.label}</span>
                        </Link>
                      </li>
                    );
                  })}
                </ul>
              </div>
            ))}
            {shownCommands.length === 0 ? <p className="a-tree-note">No commands match that filter.</p> : null}
            <div className="a-tree-note">
              Run <span className="mono">canvas-rgb --help</span> for the whole list.
            </div>
          </>
        ) : null}

        {mode === "sections" ? (
          <div className="a-tree-group">
            <div className="a-tree-group-title">On this guide</div>
            <ul className="a-tree-list">
              {shownSections.map((section) => (
                <li key={section.id}>
                  <a className="a-tree-link" href={`#${section.id}`}>
                    {section.text}
                  </a>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>
    </nav>
  );
}
