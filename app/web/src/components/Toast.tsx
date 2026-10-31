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
import { Button } from "@/components/Button";
import { CheckCircle, Info, Warning, WarningOctagon } from "@phosphor-icons/react";

export type ToastTone = "ok" | "warn" | "down" | "info";

/** A single answer a toast offers, "Undo": it runs once and the toast goes. */
export interface ToastAction {
  label: string;
  onClick: () => void;
}

interface ToastItem {
  id: number;
  tone: ToastTone;
  text: string;
  action?: ToastAction;
}

type Push = (text: string, tone?: ToastTone, action?: ToastAction) => void;

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
/** A toast that offers an answer stays long enough to read it and reach it. */
const ACTION_LIFETIME_MS = 7000;

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
  const push = useCallback((text: string, tone: ToastTone = "ok", action?: ToastAction) => {
    const id = nextId.current++;
    setItems([{ id, tone, text, action }]);
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
        <Toast key={t.id} item={t} remove={remove} />
      ))}
    </div>
  );
}

function Toast({ item, remove }: { item: ToastItem; remove: (id: number) => void }) {
  const [leaving, setLeaving] = useState(false);
  // The timers depend on the id alone. A parent that re-renders (the shell does, every second)
  // must never restart them, so the latest `remove` is read from a ref, not from the deps.
  const removeRef = useRef(remove);
  useEffect(() => {
    removeRef.current = remove;
  });

  useEffect(() => {
    const life = item.action ? ACTION_LIFETIME_MS : LIFETIME_MS;
    const hold = setTimeout(() => setLeaving(true), life);
    const drop = setTimeout(() => removeRef.current(item.id), life + 200);
    return () => {
      clearTimeout(hold);
      clearTimeout(drop);
    };
    // The action is fixed for the life of the item
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item.id]);

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
      {item.action && (
        <Button
          variant="ghost"
          size="sm"
          className="toast-action"
          onClick={() => {
            item.action?.onClick();
            removeRef.current(item.id);
          }}
        >
          {item.action.label}
        </Button>
      )}
    </div>
  );
}
