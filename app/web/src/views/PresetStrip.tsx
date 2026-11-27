"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
  ArrowCounterClockwise,
  Check,
  CopySimple,
  DotsThree,
  PencilSimple,
  Plus,
  Trash,
  X,
} from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { useSuccessPulse } from "@/components/Motion";
import { useMediaQuery } from "@/components/MediaQuery";
import { NameField } from "@/components/NameField";
import { Panel } from "@/components/Panel";
import { Popover } from "@/components/Popover";
import { presetFailure, type Doing, type Failure } from "@/lib/errors";
import { diffProps, nameTaken } from "@/lib/presetMatch";
import type { RunningPreset } from "@/lib/runningPreset";
import type { Preset } from "@/proto/Preset";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { Props } from "@/proto/props";
import { ErrorState } from "@/components/ErrorState";

/** What the name field is for: a new preset from the live values, or a rename. */
type Naming = { kind: "save" } | { kind: "saveAs" } | { kind: "rename"; name: string };


/** How many changes the diff line spells out before it says "+N more". */
const DIFF_SHOWN = 2;

/**
 * The effect's presets as keys: the built-in ones, then the owner's, then "Save current".
 *
 * Under the keys sit two lines of fixed height, so no state moves the page. The first (36 px)
 * says what the lights are running ("Matches Storm", "Custom values"), or, when the live values
 * differ from the running preset, "Changed from Storm" with Update, Save as new and Reset, or
 * holds the name field while a name is typed, or a Save key that spins while it is sent. The second (22 px) prints the
 * change ("Speed 8 to 35") or, when something was refused, the reason.
 *
 * The strip owns only what it shows (the name field, the open menu). Every change goes out
 * through a callback that returns when the daemon has answered; a callback that throws leaves
 * the field open with the message under it. A preset that was deleted says so in the notification
 * lane, with its Undo key (#366): the strip holds no message of its own that comes and goes.
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
  onDelete: (name: string) => Promise<void>;
}) {
  const narrow = useMediaQuery("(max-width: 720px)");
  const errorId = useId();
  const [naming, setNaming] = useState<Naming | null>(null);
  const [value, setValue] = useState("");
  const [working, setWorking] = useState<Doing | null>(null);
  const [error, setError] = useState<Failure | null>(null);
  // The name being saved, shown as a held key at the end of the owner's presets.
  const [saving, setSaving] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ name: string; confirm: boolean } | null>(null);
  const menuAnchor = useRef<HTMLElement | null>(null);
  // One pulse on the Save chip when a preset lands (issue #187, rule 6).
  const { el: savePulseEl, fire: pulseSave } = useSuccessPulse();
  const scroller = useRef<HTMLDivElement>(null);
  const busy = working !== null;

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
  const changes = running?.modified && runningAny ? diffProps(defs, runningAny.props, live) : [];
  const changeText = (() => {
    const shown = changes
      .slice(0, DIFF_SHOWN)
      .map((c) => `${c.label} ${c.from} to ${c.to}`)
      .join(", ");
    return changes.length > DIFF_SHOWN ? `${shown} +${changes.length - DIFF_SHOWN} more` : shown;
  })();

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
  const attempt = async (doing: Doing, fn: () => Promise<void>, held?: string): Promise<boolean> => {
    setWorking(doing);
    setError(null);
    if (held) setSaving(held);
    try {
      await fn();
      return true;
    } catch (e) {
      setError(presetFailure(e, doing));
      return false;
    } finally {
      setWorking(null);
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
        ? await attempt("rename", () => onRename(naming.name, name))
        : await attempt("save", () => onSave(name), name);
    // One pulse on the Save chip when a preset lands (issue #187, rule 6):
    // a rename is not a save, so it never pulses.
    // A save or a save-as pulses the Save chip; a rename does not.
    if (ok && naming.kind !== "rename") pulseSave();
    if (ok) close();
  };

  const doUpdate = async () => {
    if (running) await attempt("update", () => onUpdate(running.name));
  };
  const doReset = async () => {
    if (running) await attempt("apply", () => onReset(running.name));
  };

  const chipFor = (p: Preset, mine: boolean) => {
    const lit = running?.name === p.name;
    const marked = lit && running.modified;
    // A built-in preset added under a name this owner's preset already uses wins: the daemon
    // looks up built-ins first, so the user's key cannot be reached until it is renamed.
    const shadowed = mine && builtIn.some((b) => b.name.toLowerCase() === p.name.toLowerCase());
    const chip = (
      <Chip
        key={p.name}
        label={`Apply preset ${p.name}${marked ? " (modified)" : ""}${shadowed ? " (shadowed)" : ""}`}
        pressed={lit}
        marked={marked}
        title={shadowed ? "A built-in preset now has this name; the built-in wins" : undefined}
        onClick={() => {
          setError(null);
          void attempt("apply", () => onApply(p.name));
        }}
      >
        {p.name}
        {shadowed && <span className="preset-shadowed">shadowed</span>}
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

  const saveWord = working === "save" || working === "update" || working === "rename";

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
        {/* The keys stay where they are while the save is sent: Save wears the busy spinner and
            Cancel waits, so nothing under the pointer is replaced by a word. */}
        {narrow ? (
          <>
            <IconButton tip={false} label={verb} busy={saveWord} icon={<Check size={16} weight="bold" />} onClick={() => void submit()} />
            <IconButton tip={false} label="Cancel" disabled={saveWord} icon={<X size={16} />} onClick={close} />
          </>
        ) : (
          <>
            <Button variant="primary" size="sm" busy={saveWord} onClick={() => void submit()}>
              {verb}
            </Button>
            <Button variant="ghost" size="sm" disabled={saveWord} onClick={close}>
              Cancel
            </Button>
          </>
        )}
      </>
    );
  } else if (running?.modified) {
    line = (
      <>
        <span className="preset-diff" title={`Changed from ${running.name}`}>
          <span className="preset-diff-led" aria-hidden="true" />
          <span className="preset-status preset-changed">
            Changed from <span className="preset-status-name">{running.name}</span>
          </span>
        </span>
        <span className="preset-actions">
          {runningUser && (
            <Button
              variant="primary"
              size="sm"
              busy={working === "update"}
              disabled={(busy && working !== "update") || disabled}
              aria-label={`Update ${running.name}`}
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
            busy={working === "apply"}
            disabled={busy && working !== "apply"}
            onClick={() => void doReset()}
          />
        </span>
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
            ref={savePulseEl}
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
        {error !== null ? (
          <ErrorState compact className="preset-note" id={errorId} error={error} />
        ) : (
          <p
            className="preset-note"
            id={errorId}
            title={changes.length > 0 ? changes.map((c) => `${c.label} ${c.from} to ${c.to}`).join("\n") : undefined}
          >
            {naming ? "" : changeText}
          </p>
        )}
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
                    busy={working === "delete"}
                    disabled={busy && working !== "delete"}
                    onClick={() => {
                      const name = menu.name;
                      setMenu(null);
                      void attempt("delete", () => onDelete(name));
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
                    void attempt("duplicate", () => onDuplicate(name));
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
