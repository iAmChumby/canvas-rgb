"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
  ArrowCounterClockwise,
  Check,
  CopySimple,
  DotsThree,
  FloppyDisk,
  PencilSimple,
  Plus,
  Trash,
  X,
} from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { useMediaQuery } from "@/components/MediaQuery";
import { NameField } from "@/components/NameField";
import { Panel } from "@/components/Panel";
import { Popover } from "@/components/Popover";
import { messageOf } from "@/lib/errors";
import { diffProps, nameTaken } from "@/lib/presetMatch";
import type { RunningPreset } from "@/lib/runningPreset";
import type { Preset } from "@/proto/Preset";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { Props } from "@/proto/props";

/** What the name field is for: a new preset from the live values, or a rename. */
type Naming = { kind: "save" } | { kind: "saveAs" } | { kind: "rename"; name: string };

/** How many changes the diff line spells out before it says "+N more". */
const DIFF_SHOWN = 2;

/**
 * The effect's presets as keys: the built-in ones, then the owner's, then "Save current".
 *
 * The strip has one line under the keys, always the same height, so no state moves the page:
 * it says what the lights are running (a preset, or custom values); when the live values differ
 * from the running preset it lists the change with Update, Save as new and Reset; and while a
 * name is being typed it holds the name field. A refusal prints under that line, in space that is
 * always set aside for it.
 *
 * The strip owns only what it shows (the name field, the open menu). Every change goes out
 * through a callback that returns when the daemon has answered; a callback that throws leaves
 * the field open with the message under it.
 */
export function PresetStrip({
  builtIn,
  user,
  defs,
  live,
  running,
  disabled,
  onApply,
  onSave,
  onUpdate,
  onReset,
  onRename,
  onDuplicate,
  onDelete,
}: {
  builtIn: Preset[];
  user: Preset[];
  defs: PropertyDef[];
  /** The values on the lights now. */
  live: Props;
  running: RunningPreset | null;
  disabled?: boolean;
  onApply: (name: string) => Promise<void>;
  /** Saves the live values under a new name. */
  onSave: (name: string) => Promise<void>;
  /** Overwrites a user preset with the live values. */
  onUpdate: (name: string) => Promise<void>;
  onReset: (name: string) => Promise<void>;
  onRename: (name: string, to: string) => Promise<void>;
  onDuplicate: (name: string) => Promise<void>;
  /** Removes a user preset. Answers a way to put it back, which the strip offers as Undo. */
  onDelete: (name: string) => Promise<(() => Promise<void>) | void>;
}) {
  const narrow = useMediaQuery("(max-width: 720px)");
  const errorId = useId();
  const [naming, setNaming] = useState<Naming | null>(null);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The name being saved, shown as a held key at the end of the owner's presets.
  const [saving, setSaving] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ name: string; confirm: boolean } | null>(null);
  // The last delete, kept for a few seconds in the line under the keys. The page's toast floats
  // at the top and may be scrolled out of sight; this stays beside the keys.
  const [undo, setUndo] = useState<{ name: string; run: () => Promise<void> } | null>(null);
  useEffect(() => {
    if (!undo) return;
    const t = setTimeout(() => setUndo(null), 8000);
    return () => clearTimeout(t);
  }, [undo]);
  const menuAnchor = useRef<HTMLElement | null>(null);
  const scroller = useRef<HTMLDivElement>(null);

  // On a phone the keys sit in one scrolling line: the lit key is brought into view.
  useEffect(() => {
    const box = scroller.current;
    const lit = box?.querySelector<HTMLElement>('[aria-pressed="true"]');
    if (!box || !lit || box.scrollWidth <= box.clientWidth) return;
    const left = lit.offsetLeft - box.offsetLeft;
    const right = left + lit.offsetWidth;
    if (left < box.scrollLeft) box.scrollTo({ left: left - 8, behavior: "smooth" });
    else if (right > box.scrollLeft + box.clientWidth) {
      box.scrollTo({ left: right - box.clientWidth + 8, behavior: "smooth" });
    }
  }, [running?.name, user.length]);

  const everyName = [builtIn, user];
  const runningUser = running ? user.find((p) => p.name === running.name) : undefined;
  const runningAny = running ? [...builtIn, ...user].find((p) => p.name === running.name) : undefined;
  const changes =
    running?.modified && runningAny ? diffProps(defs, runningAny.props, live) : [];

  const open = (n: Naming) => {
    setError(null);
    setMenu(null);
    setNaming(n);
    setValue(n.kind === "rename" ? n.name : "");
  };
  const close = () => {
    setNaming(null);
    setValue("");
    setError(null);
  };

  /** Runs one change. A refusal prints under the line; the field stays open to be fixed. */
  const attempt = async (fn: () => Promise<void>, held?: string): Promise<boolean> => {
    setBusy(true);
    setError(null);
    if (held) setSaving(held);
    try {
      await fn();
      return true;
    } catch (e) {
      setError(messageOf(e));
      return false;
    } finally {
      setBusy(false);
      setSaving(null);
    }
  };

  const submit = async () => {
    if (!naming || busy) return;
    const name = value.trim();
    if (name.length === 0) {
      setError("Give the preset a name.");
      return;
    }
    const except = naming.kind === "rename" ? naming.name : undefined;
    if (naming.kind === "rename" && name === naming.name) {
      close();
      return;
    }
    if (nameTaken(name, everyName, except)) {
      setError(`${name} is already a preset of this effect.`);
      return;
    }
    const ok =
      naming.kind === "rename"
        ? await attempt(() => onRename(naming.name, name))
        : await attempt(() => onSave(name), name);
    if (ok) close();
  };

  const doUpdate = async () => {
    if (running) await attempt(() => onUpdate(running.name));
  };
  const doReset = async () => {
    if (running) await attempt(() => onReset(running.name));
  };

  const chipFor = (p: Preset, mine: boolean) => {
    const lit = running?.name === p.name;
    const marked = lit && running.modified;
    const chip = (
      <Chip
        key={p.name}
        label={`Apply preset ${p.name}${marked ? " (modified)" : ""}`}
        pressed={lit}
        marked={marked}
        onClick={() => {
          setError(null);
          void attempt(() => onApply(p.name));
        }}
      >
        {p.name}
      </Chip>
    );
    if (!mine) return chip;
    return (
      <span className="preset-key" key={p.name} data-lit={lit ? "true" : "false"}>
        {chip}
        <span className="preset-more" onClickCapture={(e) => (menuAnchor.current = e.currentTarget)}>
          <IconButton
            size="sm"
            label={`Actions for ${p.name}`}
            icon={<DotsThree size={16} weight="bold" />}
            onClick={() => setMenu({ name: p.name, confirm: false })}
          />
        </span>
      </span>
    );
  };

  // The line under the keys: naming, changes, or a plain statement. Always one line tall.
  let line;
  if (naming) {
    const verb = naming.kind === "rename" ? "Rename" : "Save";
    line = (
      <>
        <NameField
          value={value}
          label={naming.kind === "rename" ? `New name for ${naming.name}` : "Preset name"}
          placeholder={naming.kind === "rename" ? "New name" : "Name these values"}
          disabled={busy}
          invalid={error !== null}
          describedBy={error ? errorId : undefined}
          onChange={(v) => {
            setValue(v);
            setError(null);
          }}
          onEnter={() => void submit()}
          onEscape={close}
        />
        {narrow ? (
          <>
            <IconButton label={verb} icon={<Check size={16} weight="bold" />} disabled={busy} onClick={() => void submit()} />
            <IconButton label="Cancel" icon={<X size={16} />} disabled={busy} onClick={close} />
          </>
        ) : (
          <>
            <Button variant="primary" size="sm" disabled={busy} onClick={() => void submit()}>
              {verb}
            </Button>
            <Button variant="ghost" size="sm" disabled={busy} onClick={close}>
              Cancel
            </Button>
          </>
        )}
      </>
    );
  } else if (running?.modified) {
    const shown = changes
      .slice(0, DIFF_SHOWN)
      .map((c) => `${c.label} ${c.from} to ${c.to}`)
      .join(", ");
    const more = changes.length > DIFF_SHOWN ? ` +${changes.length - DIFF_SHOWN} more` : "";
    line = (
      <>
        <span className="preset-diff" title={changes.map((c) => `${c.label} ${c.from} to ${c.to}`).join("\n")}>
          <span className="preset-diff-led" aria-hidden="true" />
          <span className="fig preset-diff-text">
            {shown}
            {more}
          </span>
        </span>
        <span className="preset-actions">
          {narrow ? (
            <>
              {runningUser && (
                <IconButton
                  label={`Update ${running.name}`}
                  icon={<FloppyDisk size={16} />}
                  disabled={busy || disabled}
                  onClick={() => void doUpdate()}
                />
              )}
              <IconButton
                label="Save as new preset"
                icon={<Plus size={16} />}
                disabled={busy}
                onClick={() => open({ kind: "saveAs" })}
              />
              <IconButton
                label={`Reset to ${running.name}`}
                icon={<ArrowCounterClockwise size={16} />}
                disabled={busy}
                onClick={() => void doReset()}
              />
            </>
          ) : (
            <>
              {runningUser && (
                <Button
                  variant="primary"
                  size="sm"
                  icon={<FloppyDisk size={14} />}
                  disabled={busy || disabled}
                  onClick={() => void doUpdate()}
                >
                  Update
                </Button>
              )}
              <IconButton
                label="Save as new preset"
                icon={<Plus size={16} />}
                disabled={busy}
                onClick={() => open({ kind: "saveAs" })}
              />
              <IconButton
                label={`Reset to ${running.name}`}
                icon={<ArrowCounterClockwise size={16} />}
                disabled={busy}
                onClick={() => void doReset()}
              />
            </>
          )}
        </span>
      </>
    );
  } else if (undo) {
    line = (
      <>
        <span className="preset-status">
          Deleted <span className="preset-status-name">{undo.name}</span>
        </span>
        <Button
          size="sm"
          icon={<ArrowCounterClockwise size={14} />}
          disabled={busy}
          onClick={() => {
            const back = undo.run;
            setUndo(null);
            void attempt(back);
          }}
        >
          Undo
        </Button>
      </>
    );
  } else {
    line = (
      <span className="preset-status">
        {running ? (
          <>
            Matches <span className="preset-status-name">{running.name}</span>
          </>
        ) : (
          "Custom values"
        )}
      </span>
    );
  }

  const menuPreset = menu ? user.find((p) => p.name === menu.name) : undefined;

  return (
    <Panel title="Presets">
      <div className="preset-strip">
        <div className="preset-chips" role="group" aria-label="Presets" ref={scroller}>
          {builtIn.map((p) => chipFor(p, false))}
          {builtIn.length > 0 && user.length + (saving ? 1 : 0) > 0 && <span className="preset-rule" aria-hidden="true" />}
          {user.map((p) => chipFor(p, true))}
          {saving && !user.some((p) => p.name === saving) && (
            <Chip pending pressed label={`Saving ${saving}`} onClick={() => undefined}>
              {saving}
            </Chip>
          )}
          <Chip
            className="chip-add"
            icon={<Plus size={14} weight="bold" />}
            pressed={naming?.kind === "save"}
            label="Save current values as a preset"
            onClick={() => (naming?.kind === "save" ? close() : open({ kind: "save" }))}
          >
            Save current
          </Chip>
        </div>
        <div className="preset-line" data-mode={naming ? "naming" : running?.modified ? "modified" : "idle"}>
          {line}
        </div>
        <p className="preset-error error-line" id={errorId} role={error ? "alert" : undefined}>
          {error}
        </p>
      </div>

      <Popover
        anchor={menuAnchor}
        open={menu !== null && menuPreset !== undefined}
        onClose={() => setMenu(null)}
        label={menu ? `Actions for ${menu.name}` : undefined}
        className="preset-menu"
      >
        {menu && menuPreset && (
          <div className="preset-menu-list">
            {menu.confirm ? (
              <>
                <p className="preset-menu-ask">Delete {menu.name}?</p>
                <div className="row-wrap">
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={busy}
                    onClick={() => {
                      const name = menu.name;
                      setMenu(null);
                      void attempt(async () => {
                        const back = await onDelete(name);
                        if (back) setUndo({ name, run: back });
                      });
                    }}
                  >
                    Delete
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setMenu(null)}>
                    Cancel
                  </Button>
                </div>
              </>
            ) : (
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<PencilSimple size={16} />}
                  onClick={() => open({ kind: "rename", name: menu.name })}
                >
                  Rename
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<CopySimple size={16} />}
                  onClick={() => {
                    const name = menu.name;
                    setMenu(null);
                    void attempt(() => onDuplicate(name));
                  }}
                >
                  Duplicate
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<Trash size={16} />}
                  onClick={() => setMenu({ name: menu.name, confirm: true })}
                >
                  Delete
                </Button>
              </>
            )}
          </div>
        )}
      </Popover>
    </Panel>
  );
}
