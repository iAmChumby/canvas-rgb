"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { MagnifyingGlass } from "@phosphor-icons/react";
import { Button } from "@editor/components/Button";
import { CommandPalette, type Command } from "@editor/components/CommandPalette";

const BASE = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

interface Entry {
  title: string;
  hint: string;
  href: string;
  keywords: string;
}

function groupLabel(group: string) {
  return group
    .split(/\s+/)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

/** The one docs search, in the site header. Ctrl K opens the editor's command palette. */
export function DocsSearch() {
  const [entries, setEntries] = useState<Entry[]>([]);
  const [open, setOpen] = useState(false);
  const router = useRouter();

  useEffect(() => {
    let live = true;
    fetch(`${BASE}/docs/search-index.json`)
      .then((response) => (response.ok ? response.json() : []))
      .then((data: Entry[]) => {
        if (live) setEntries(Array.isArray(data) ? data : []);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setOpen((was) => !was);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const commands = useMemo<Command[]>(
    () =>
      entries.map((entry) => ({
        id: entry.href,
        label: entry.title,
        hint: groupLabel(entry.hint),
        group: "Documentation",
        run: () => {
          setOpen(false);
          router.push(`${BASE}${entry.href}`);
        },
      })),
    [entries, router],
  );

  const close = useCallback(() => setOpen(false), []);

  return (
    <>
      <Button
        aria-label="Search the docs"
        icon={<MagnifyingGlass size={16} aria-hidden="true" />}
        className="site-search"
        onClick={() => setOpen(true)}
      >
        <span className="site-search-label">Search the docs</span>
        <kbd className="kbd">Ctrl K</kbd>
      </Button>
      {open ? <CommandPalette commands={commands} onClose={close} /> : null}
    </>
  );
}
