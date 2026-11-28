"use client";

import { useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { useMediaQuery } from "@/components/MediaQuery";
import { useFocusTrap } from "@/components/useFocusTrap";
import { PHONE_QUERY } from "@/lib/MediaQuery";
import { useSnap } from "@/lib/store";
import { useSetupVisible } from "@/views/editor/flow";
import { Led, SetupChecklist, SetupCount, useSetupRows } from "@/views/setup/SetupChecklist";
import s from "@/views/setup/setup.module.css";

/** The checklist over the page, on a phone. Escape and the scrim shut it. */
function SetupSheet({ onClose, onTroubleshoot }: { onClose: () => void; onTroubleshoot: () => void }) {
  const panel = useRef<HTMLElement>(null);
  useFocusTrap({ panelRef: panel, capture: false, onEscape: onClose });
  return (
    <div className={s.sheetRoot}>
      <div className={s.scrim} data-testid="setup-scrim" onClick={onClose} aria-hidden="true" />
      <section className={s.sheet} role="dialog" aria-label="Setup checklist" ref={panel}>
        <IconButton label="Close" icon={<X size={18} />} onClick={onClose} />
        <SetupChecklist onTroubleshoot={onTroubleshoot} onLeave={onClose} />
      </section>
    </div>
  );
}

/** The phone's one line: four lamps and the count. */
function SetupPill({ onOpen }: { onOpen: () => void }) {
  const rows = useSetupRows();
  return (
    <Button className={s.pill} onClick={onOpen} data-testid="setup-pill" aria-haspopup="dialog">
      <span>Setup</span>
      <span className={s.pillLeds}>
        {rows.map((r) => (
          <Led key={r.id} state={r.state} />
        ))}
      </span>
      <SetupCount rows={rows} />
    </Button>
  );
}

function SetupSurface({ onTroubleshoot }: { onTroubleshoot: () => void }) {
  const phone = useMediaQuery(PHONE_QUERY);
  const [open, setOpen] = useState(false);
  if (!phone) {
    return (
      <aside className={s.panel} aria-label="Setup checklist">
        <SetupChecklist onTroubleshoot={onTroubleshoot} />
      </aside>
    );
  }
  // On the body: the page's box is a size container, which would hold a fixed element inside it.
  return createPortal(
    <>
      <SetupPill onOpen={() => setOpen(true)} />
      {open && <SetupSheet onClose={() => setOpen(false)} onTroubleshoot={onTroubleshoot} />}
    </>,
    document.body,
  );
}

/**
 * The Scenes page with the checklist beside it: a right column on a wide window, a band above the
 * page in between, a pill on a phone. It never takes the window, so the page stays usable. The
 * wrapper is always there, so showing or hiding the checklist never remounts the page.
 */
export function SetupLayout({ children, onTroubleshoot }: { children: ReactNode; onTroubleshoot: () => void }) {
  const status = useSnap((st) => st.status);
  const visible = useSetupVisible(status);
  return (
    <div className={s.frame} data-setup={visible ? "on" : "off"}>
      <div className={s.page}>{children}</div>
      {visible && <SetupSurface onTroubleshoot={onTroubleshoot} />}
    </div>
  );
}
