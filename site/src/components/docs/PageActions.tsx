"use client";

import { useState } from "react";
import { Copy, FileText } from "@phosphor-icons/react";
import { Button, Swap } from "@editor/components/Button";

const BASE = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

/** The opencode-style pair on every docs page: copy the Markdown, or open it. */
export function PageActions({ mdPath, className = "" }: { mdPath: string; className?: string }) {
  const [done, setDone] = useState(false);
  const copy = async () => {
    try {
      const response = await fetch(`${BASE}${mdPath}`);
      const text = await response.text();
      await navigator.clipboard.writeText(text);
    } catch {
      // Clipboard access is refused in some contexts; the Markdown link still opens the file.
    }
    setDone(true);
    window.setTimeout(() => setDone(false), 1800);
  };

  return (
    <div className={`page-actions ${className}`.trim()} aria-label="Page actions">
      <Button size="sm" icon={<Copy size={14} />} className="page-action" onClick={() => void copy()}>
        <Swap labels={["Copy page as Markdown", "Copied"]} at={done ? 1 : 0} />
      </Button>
      <a className="page-action" href={`${BASE}${mdPath}`}>
        <FileText size={14} aria-hidden="true" />
        <span>View as Markdown</span>
      </a>
    </div>
  );
}
