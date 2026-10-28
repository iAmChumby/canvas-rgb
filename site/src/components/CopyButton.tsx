"use client";

import { useState } from "react";
import { Copy } from "@phosphor-icons/react";
import { Button, Swap } from "@editor/components/Button";

/**
 * A button that copies the commands. The label swaps in place, so the button never changes
 * width under the pointer.
 */
export function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // The clipboard is not always there: fall back to the old way.
      const area = document.createElement("textarea");
      area.value = text;
      area.setAttribute("readonly", "");
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      try {
        document.execCommand("copy");
      } catch {
        document.body.removeChild(area);
        return;
      }
      document.body.removeChild(area);
    }
    setDone(true);
    window.setTimeout(() => setDone(false), 2000);
  };

  return (
    <Button size="sm" icon={<Copy size={16} />} onClick={() => void copy()}>
      <Swap labels={["Copy", "Copied"]} at={done ? 1 : 0} />
    </Button>
  );
}
