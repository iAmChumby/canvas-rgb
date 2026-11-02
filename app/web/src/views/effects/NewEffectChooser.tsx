"use client";

// New from code (D2.3): a blank file second, a copy of an existing HTML effect first. A Rust
// built-in has no HTML source to copy (the route answers 404), so only `html:` effects are
// offered. Reading a copy's source has its own wait and its own error.

import { CaretLeft, CopySimple, FilePlus } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Panel } from "@/components/Panel";
import { Thumb } from "@/components/Thumb";
import type { EffectSummary } from "@/proto/EffectSummary";

export function NewEffectChooser({
  effects,
  busy,
  error,
  narrow,
  onBlank,
  onCopy,
  onCancel,
}: {
  effects: EffectSummary[];
  /** The effect whose source is being read now. */
  busy: string | null;
  error: string | null;
  narrow: boolean;
  onBlank: () => void;
  onCopy: (id: string) => void;
  onCancel: () => void;
}) {
  const html = effects.filter((effect) => effect.id.startsWith("html:"));
  const body = (
    <div className="eff-choose" data-testid="new-effect">
      <div className="eff-choose-head">
        {narrow ? (
          <IconButton label="Back to the effects" icon={<CaretLeft size={18} />} onClick={onCancel} />
        ) : null}
        <h2 className="effect-title">New from code</h2>
        {!narrow && (
          <span className="eff-choose-cancel">
            <Button size="sm" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          </span>
        )}
      </div>

      <Panel title="Copy an effect" className="eff-choose-panel">
        <p className="dim eff-choose-sub">
          A user or stock HTML effect. Its author and licence line stay at the top of the file.
        </p>
        {html.length === 0 ? (
          <p className="dim eff-choose-empty">There is no HTML effect to copy yet. Start blank instead.</p>
        ) : (
          <div className="eff-choose-list">
            {html.map((effect) => (
              <button
                key={effect.id}
                type="button"
                className="eff-choose-row"
                aria-label={`Copy ${effect.name}`}
                disabled={busy !== null}
                onClick={() => onCopy(effect.id)}
              >
                <span className="eff-choose-thumb">
                  <Thumb id={effect.id} size="sm" label={effect.name} />
                </span>
                <span className="eff-choose-name">{effect.name}</span>
                <Chip html>HTML</Chip>
                <span className="eff-choose-go">
                  {busy === effect.id ? (
                    "Reading…"
                  ) : (
                    <>
                      <CopySimple size={14} aria-hidden="true" /> Copy
                    </>
                  )}
                </span>
              </button>
            ))}
          </div>
        )}
        {error && (
          <p className="error-line" role="alert">
            {error}
          </p>
        )}
      </Panel>

      <Panel title="Start blank" className="eff-choose-panel">
        <p className="dim eff-choose-sub">
          The minimal file from the docs: a canvas, a colour and a draw loop that moves.
        </p>
        <Button variant="primary" size="sm" icon={<FilePlus size={15} />} disabled={busy !== null} onClick={onBlank}>
          Blank effect
        </Button>
      </Panel>
    </div>
  );

  if (!narrow) return body;
  return (
    <div className="eff-layer" role="dialog" aria-label="New from code">
      <div className="eff-layer-body eff-layer-chooser">{body}</div>
    </div>
  );
}
