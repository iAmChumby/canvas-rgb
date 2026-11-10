"use client";

// The editor's notifications (#168).
//
// The library (sonner 2.0.8) owns the lane: the queue, the stacking, the enter and leave
// motion, the swipe away and the dismissal. This file owns the policy: which tone a message
// carries, how long it stays, when it is held while the reader is on it, and which messages
// are the same message.
//
// Call sites import `notify` from here and nothing else: they never touch the library, so the
// policy can change in one place.

import { useEffect, useRef } from "react";
import { Toaster, toast as sonnerToast } from "sonner";
import { CheckCircle, Info, Warning, WarningOctagon, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";

/** The four tones a notification comes in. */
export type NotifyTone = "ok" | "warn" | "down" | "info";

/** An answer a notification offers: the key beside the message. */
export interface NotifyAction {
  label: string;
  /**
   * Runs when the reader presses the key. The message goes with the answer, unless the answer
   * returns `false`: it did not happen yet (a navigation the unsaved-changes prompt is holding),
   * so the message is pinned: no clock, not even after the pointer leaves, until the owner takes
   * it away with `dismissNotification` or the reader closes it.
   */
  onClick: () => void | boolean;
}

/** Extra policy for one message: a second line, a life of its own, and what the close key means. */
export interface NotifyOptions {
  /** A second line under the message. */
  detail?: string;
  /** How long the message stays before it goes on its own. It replaces the tone's rule. */
  lifeMs?: number;
  /**
   * Runs when the reader takes the message away by hand: the close key, or a swipe. A timeout, an
   * answer and `dismissNotification` do not run it.
   */
  onClose?: () => void;
}

/** How long a plain note stays up before it goes. */
const NOTE_MS = 3000;

const TONE_ICON = {
  ok: CheckCircle,
  warn: Warning,
  down: WarningOctagon,
  info: Info,
} as const;

/**
 * The identity of a notification: the same tone and the same words are the same message, so a
 * second one updates the first instead of stacking on it.
 */
function notificationId(tone: NotifyTone, text: string): string {
  return `${tone}:${text}`;
}

/** What is left of a notification's life, and what is holding it. */
interface Hold {
  /** The pointer is on the notification. */
  hover: boolean;
  /** The keyboard is on the notification. */
  focus: boolean;
  /** How many milliseconds are left before it goes. */
  left: number;
  /** When it goes; null while it is held. */
  due: number | null;
  /** The timer that spends the life. */
  timer: ReturnType<typeof setTimeout> | null;
}

const holds = new Map<string, Hold>();

/** Forgets a notification's clock. The library calls it however the message went (swipe too). */
function forget(id: string): void {
  const hold = holds.get(id);
  if (hold?.timer !== null && hold?.timer !== undefined) clearTimeout(hold.timer);
  holds.delete(id);
}

/**
 * The messages the app took away (a timeout, an answer, `dismissNotification`). The library says
 * "dismissed" for those too; only a message it dismissed that is not in this set was swiped away.
 */
const takenAway = new Set<string>();

/** The notification goes: the library plays the leave motion. */
function go(id: string): void {
  forget(id);
  takenAway.add(id);
  sonnerToast.dismiss(id);
}

/**
 * The message waits for its owner: the answer it offers has not happened yet (a navigation the
 * unsaved-changes prompt holds). The clock stops for good, so the pointer leaving does not start
 * it again; the owner takes the message away when the answer happens, or the reader does.
 */
function pin(id: string): void {
  const hold = holds.get(id);
  if (!hold) return;
  if (hold.timer !== null) clearTimeout(hold.timer);
  hold.timer = null;
  hold.due = null;
  hold.left = Infinity;
}

/**
 * The clock keeps time only while nobody is reading. A notification the pointer is on, or that
 * has the keyboard's focus, waits: the reader is still with it.
 */
function sync(id: string): void {
  const hold = holds.get(id);
  if (!hold) return;
  if (hold.hover || hold.focus) {
    if (hold.timer === null) return;
    clearTimeout(hold.timer);
    hold.timer = null;
    hold.left = Math.max(0, (hold.due ?? Date.now()) - Date.now());
    hold.due = null;
    return;
  }
  if (hold.timer !== null) return;
  // A message that waits for the reader has no clock to restart. setTimeout turns Infinity into
  // 0, so restarting one would take an error away the moment the pointer left it.
  if (!Number.isFinite(hold.left)) return;
  if (hold.left <= 0) {
    go(id);
    return;
  }
  hold.due = Date.now() + hold.left;
  hold.timer = setTimeout(() => go(id), hold.left);
}

/** The reader's hand on the message: hovering it or focusing it holds it. */
function touch(id: string, patch: Partial<Hold>): void {
  const hold = holds.get(id);
  if (!hold) return;
  Object.assign(hold, patch);
  sync(id);
}

/**
 * Says something.
 *
 * A note (a success, a piece of information) goes after three seconds. An error, a warning and
 * anything that offers an answer stays until it is dismissed, because a message nobody can
 * finish reading is the problem this replaces. `options.lifeMs` gives a message a life of its own:
 * it still carries its close key, and it goes on its own when the life is spent. The return value
 * is the message's identity, for `dismissNotification`.
 */
export function notify(
  text: string,
  tone: NotifyTone = "ok",
  action?: NotifyAction,
  options?: NotifyOptions,
): string {
  const id = notificationId(tone, text);
  const keeps = tone === "down" || tone === "warn" || action !== undefined;
  const life = options?.lifeMs ?? (keeps ? Infinity : NOTE_MS);
  sonnerToast.custom(
    () => (
      <NotifyBody
        id={id}
        text={text}
        tone={tone}
        action={action}
        keeps={keeps}
        detail={options?.detail}
        onClose={options?.onClose}
      />
    ),
    // The library's own timer never runs: this file's clock owns the life of every message.
    {
      id,
      duration: Infinity,
      testId: id,
      onDismiss: () => {
        forget(id);
        // A swipe is as deliberate as the close key: it means what the close key means.
        if (!takenAway.delete(id)) options?.onClose?.();
      },
    },
  );
  takenAway.delete(id);
  const before = holds.get(id);
  if (before?.timer !== null && before?.timer !== undefined) clearTimeout(before.timer);
  holds.set(id, {
    hover: before?.hover ?? false,
    focus: before?.focus ?? false,
    left: life,
    due: null,
    timer: null,
  });
  // A message that waits for the reader has no clock to start; sync leaves it alone.
  sync(id);
  return id;
}

/**
 * Takes one message away by the identity `notify` returned. A message that must not outlive its
 * page (the "Updated to" notice, on the Updates page) uses it.
 */
export function dismissNotification(id: string): void {
  go(id);
}

/**
 * Takes every notification away at once. The editor has no use for it: the tests do, so that
 * one test's message is never the next test's message.
 */
export function dismissNotifications(): void {
  for (const id of Array.from(holds.keys())) go(id);
}

interface NotifyBodyProps {
  id: string;
  text: string;
  tone: NotifyTone;
  action?: NotifyAction;
  /** True when the notification waits for the reader instead of a clock. */
  keeps: boolean;
  /** A second line under the message. */
  detail?: string;
  /** Runs when the reader closes the message with its close key. */
  onClose?: () => void;
}

/**
 * The body of a notification: the mark for its tone, the words, and the answers it carries. It
 * is the only part of a notification the app draws.
 */
function NotifyBody({ id, text, tone, action, keeps, detail, onClose }: NotifyBodyProps) {
  const Icon = TONE_ICON[tone];
  return (
    <div
      className="notify-body"
      data-tone={tone}
      role={tone === "down" ? "alert" : "status"}
      aria-live={tone === "down" ? "assertive" : "polite"}
      onMouseEnter={() => touch(id, { hover: true })}
      onMouseLeave={() => touch(id, { hover: false })}
      onFocus={() => touch(id, { focus: true })}
      onBlur={(event) => {
        // The focus moved to another key of this notification: it is still held.
        const next = event.relatedTarget;
        if (next instanceof Node && event.currentTarget.contains(next)) return;
        touch(id, { focus: false });
      }}
    >
      <span className="notify-icon" aria-hidden="true">
        <Icon size={16} weight="fill" />
      </span>
      <span className="notify-text">
        {detail ? <span className="notify-title">{text}</span> : text}
        {detail && <span className="notify-detail">{detail}</span>}
      </span>
      {action && (
        <Button
          variant="ghost"
          size="sm"
          className="notify-action"
          onClick={() => {
            if (action.onClick() === false) pin(id);
            else go(id);
          }}
        >
          {action.label}
        </Button>
      )}
      {keeps && (
        <IconButton
          label="Dismiss"
          tip={false}
          size="sm"
          className="notify-close"
          icon={<X size={14} weight="bold" />}
          onClick={() => {
            onClose?.();
            go(id);
          }}
        />
      )}
    </div>
  );
}

/**
 * The lane. It sits at the bottom right of the window and holds three notifications at a time.
 */
export function NotifyHost() {
  const lane = useRef<HTMLElement>(null);

  useEffect(() => {
    // The library's own region would say every message once more, politely, on top of the
    // role each notification carries. It stays quiet.
    lane.current?.setAttribute("aria-live", "off");
  }, []);

  return (
    <Toaster
      ref={lane}
      className="notify-lane"
      position="bottom-right"
      visibleToasts={3}
      // The lane stays open: all three messages must be readable at once.
      expand
      gap={10}
      hotkey={[]}
      customAriaLabel="Notifications"
      toastOptions={{ duration: Infinity }}
    />
  );
}
