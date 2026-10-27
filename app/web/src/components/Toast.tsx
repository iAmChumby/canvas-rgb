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

interface ToastState {
  push: Push;
  items: ToastItem[];
  remove: (id: number) => void;
}

const ToastContext = createContext<ToastState>({
  push: () => {},
  items: [],
  remove: () => {},
});

/** Shows a short confirmation. Returns nothing; the toast lives on its own. */
export function useToast(): Push {
  return useContext(ToastContext).push;
}

/** The toast list and the way to change it. For the host that draws the lane. */
export function useToasts(): ToastState {
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
 * The toast host. One toast at a time, in the lane the shell draws at the top of the page.
 * The lane is the page's own space: a toast never floats over a control.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  // One toast at a time: the newest replaces the one before it. A stack would grow the lane
  // and push the page down.
  const push = useCallback((text: string, tone: ToastTone = "ok") => {
    const id = nextId.current++;
    setItems([{ id, tone, text }]);
  }, []);

  const remove = useCallback((id: number) => {
    setItems((prev) => prev.filter((x) => x.id !== id));
  }, []);

  const value = useMemo(() => ({ push, items, remove }), [push, items, remove]);

  return <ToastContext.Provider value={value}>{children}</ToastContext.Provider>;
}

/**
 * The host the toast floats in. On a wide screen it hangs over the page head's subtitle row; on
 * a phone it takes a band of the bottom dock. It takes no room in the page, so the page's first
 * content does not move when a toast comes or goes.
 */
export function ToastHost() {
  const { items, remove } = useToasts();
  return (
    <div className="toast-host" role="status" aria-live="polite">
      {items.map((t) => (
        <Toast key={t.id} item={t} onDone={() => remove(t.id)} />
      ))}
    </div>
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
