"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { CheckCircle, Info, Warning, WarningOctagon } from "@phosphor-icons/react";

export type ToastTone = "ok" | "warn" | "down" | "info";

interface ToastItem {
  id: number;
  tone: ToastTone;
  text: string;
}

type Push = (text: string, tone?: ToastTone) => void;

const ToastContext = createContext<Push>(() => {});

/** Shows a short confirmation. Returns nothing; the toast lives on its own. */
export function useToast(): Push {
  return useContext(ToastContext);
}

const LIFETIME_MS = 2600;

const ICONS: Record<ToastTone, ReactNode> = {
  ok: <CheckCircle size={16} weight="fill" />,
  warn: <Warning size={16} weight="fill" />,
  down: <WarningOctagon size={16} weight="fill" />,
  info: <Info size={16} weight="fill" />,
};

/**
 * The toast host. One stack, bottom right on desktop and above the bottom bar on
 * narrow screens. Each toast rises in, holds, and fades out on its own.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  const push = useCallback((text: string, tone: ToastTone = "ok") => {
    const id = nextId.current++;
    setItems((prev) => [...prev.slice(-2), { id, tone, text }]);
  }, []);

  const value = useMemo(() => push, [push]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-host" role="status" aria-live="polite">
        {items.map((t) => (
          <Toast key={t.id} item={t} onDone={() => setItems((prev) => prev.filter((x) => x.id !== t.id))} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

function Toast({ item, onDone }: { item: ToastItem; onDone: () => void }) {
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    const hold = setTimeout(() => setLeaving(true), LIFETIME_MS);
    const drop = setTimeout(onDone, LIFETIME_MS + 200);
    return () => {
      clearTimeout(hold);
      clearTimeout(drop);
    };
  }, [item.id, onDone]);

  return (
    <div
      className="toast"
      data-leaving={leaving ? "true" : "false"}
      style={{ "--tone": `var(--${item.tone})` } as CSSProperties}
    >
      <span className="toast-icon" aria-hidden="true">
        {ICONS[item.tone]}
      </span>
      <span className="toast-text">{item.text}</span>
    </div>
  );
}
