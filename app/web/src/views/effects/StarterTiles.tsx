"use client";

// The empty builder's starters (#257 phase 4, spec §5): three whole stacks, each a click away, so
// "New effect" opens on the stage with something to try rather than a prose list. A tile shows the
// stack's own effect thumbs and its size; the whole stack lands in one undo step.

import { Flask } from "@phosphor-icons/react";
import { Thumb } from "@/components/Thumb";
import type { StarterStack } from "@/lib/demoStacks";

export function StarterTiles({
  starters,
  onAdd,
}: {
  starters: StarterStack[];
  onAdd: (starter: StarterStack) => void;
}) {
  return (
    <section className="rd-starters" aria-label="Start with a stack">
      <div className="rd-starters-head">
        <h2>Start with a stack</h2>
        <span className="mute">
          <Flask size={14} aria-hidden="true" /> A whole stack in one click
        </span>
      </div>
      <div className="rd-starters-grid">
        {starters.map((starter) => (
          <button
            key={starter.id}
            type="button"
            className="rd-starter"
            aria-label={`Add the ${starter.name} starter stack`}
            title={starter.blurb}
            onClick={() => onAdd(starter)}
          >
            <span className="rd-starter-thumbs" aria-hidden="true">
              {starter.layers.slice(0, 3).map((layer, index) => (
                <Thumb
                  key={`${layer.effect}-${index}`}
                  id={layer.effect}
                  size="sm"
                  label={layer.name}
                  live={false}
                />
              ))}
            </span>
            <span className="rd-starter-text">
              <span className="rd-starter-name">{starter.name}</span>
              <span className="rd-starter-meta">
                {starter.layers.length} {starter.layers.length === 1 ? "layer" : "layers"}
              </span>
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
