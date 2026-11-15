"use client";

// A pattern's own page: its name, and the editor on it.
//
// A new pattern starts with the next free "Pattern n" in a name field, and Save writes the whole
// document under the name in that field: `putPattern` takes the id in the path and the same id in
// the body, and a save never renames. A saved pattern has no field - its id is its name - and says
// instead, in the row under its name, that a change here reaches every group playing it.
//
// The editor keeps the draft on the lights and hands its document up on Save; this page adds the
// two things the library owns: the name, and what happens when the name is refused (a name
// another pattern has is the store's own conflict, and the editor shows that sentence in its row).
//
// Leaving without saving asks nothing: the work is the owner's to keep or drop, and the draft the
// editor opened ends when it goes.

import { useState } from "react";
import { NameField } from "@/components/NameField";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { PatternEditor } from "@/views/patterns/PatternEditor";
import { patternName } from "@/views/effects/PatternTile";
import type { Pattern } from "@/proto/Pattern";
import s from "@/views/effects/newEffect.module.css";

/** `PATTERN_ID_PREFIX` from the spine. */
const PATTERN_ID_PREFIX = "pattern:";
/** `PATTERN_NAME_MAX` from the spine, before the prefix. */
const NAME_MAX = 60;

/** The first "Pattern n" no saved pattern has, ignoring case, as the daemon compares names. */
export function nextFreeName(taken: string[]): string {
  const used = new Set(taken.map((id) => patternName(id).toLowerCase()));
  for (let n = 1; n < 10_000; n += 1) {
    const name = `Pattern ${n}`;
    if (!used.has(name.toLowerCase())) return name;
  }
  return "Pattern";
}

export function PatternNew({
  pattern,
  fresh,
  taken,
  onClose,
  onSaved,
}: {
  /** The document the editor opens: all off for a new pattern, the saved one to edit. */
  pattern: Pattern;
  /** A new pattern: the name is a field, and Save puts it in the library. */
  fresh: boolean;
  /** Every saved pattern's id, so the default name is free. */
  taken: string[];
  /** Out of the page: the draft the editor opened ends with it. */
  onClose: () => void;
  /** The pattern the daemon saved, whole. */
  onSaved: (saved: Pattern) => void;
}) {
  const [name, setName] = useState(() => (fresh ? nextFreeName(taken) : patternName(pattern.id)));
  const trimmed = name.trim();
  // The id is the name: the editor's head and its accessible name read it, so the field is the
  // one place a new pattern is named and the head follows it as it is typed.
  const id = `${PATTERN_ID_PREFIX}${trimmed}`;
  /** The saved patterns' names, compared the way the store compares them: ignoring case. */
  const takenNames = new Set(taken.map((saved) => patternName(saved).toLowerCase()));

  /**
   * The daemon's answer, or its refusal thrown to the editor's own row.
   *
   * A new pattern may not take a saved pattern's name, ignoring case. That is the store's own
   * rule, and it is said here because the id a fresh Save would send is that pattern's id: the
   * daemon replaces a pattern saved under the id it is given, so typing an existing name would
   * overwrite that pattern and rename nothing. A refused name changes nothing, and the editor
   * stays open with the work in it.
   */
  const save = async (document: Pattern) => {
    if (fresh && takenNames.has(trimmed.toLowerCase())) {
      throw new Error(`${trimmed} is already another pattern's name, ignoring case`);
    }
    const saved = await api.putPattern(id, { ...document, id });
    // The library's own list gains the pattern, and the inspector shows the saved document.
    await refreshAll();
    onSaved(saved);
  };

  /** Enter finishes the name; Save, the editor's own key under the head, keeps the work. */
  const doneNaming = () => {
    if (typeof document !== "undefined" && document.activeElement instanceof HTMLElement) {
      document.activeElement.blur();
    }
  };

  return (
    <div className={s.page} data-testid="pattern-page" data-fresh={fresh ? "true" : "false"}>
      {fresh && (
        <div className={s.pageHead}>
          <span className={s.pageField}>
            <NameField
              label="Name"
              value={name}
              maxLength={NAME_MAX}
              invalid={trimmed.length === 0}
              onChange={setName}
              onEnter={doneNaming}
              onEscape={onClose}
            />
          </span>
        </div>
      )}
      <PatternEditor
        // One pattern is one editor: the document is read once, and its history lives with it.
        key={patternName(pattern.id)}
        pattern={{ ...pattern, id }}
        // A saved pattern says, in the row under its name, that a change here reaches its groups.
        note={fresh ? null : "Changes here reach every group that plays it"}
        onSave={save}
        onCancel={onClose}
      />
    </div>
  );
}
