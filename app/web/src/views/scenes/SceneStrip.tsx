"use client";

import { Plus } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Skeleton } from "@/components/Skeleton";
import type { Scene } from "@/proto/Scene";
import s from "@/views/scenes/scenes.module.css";

/**
 * The scene keys: one key per scene, the active one latched with its LED lit, then a "+" key
 * that makes a copy of the active scene. It stays above the group rows while they scroll. A key
 * sets the scene on one press and never changes size: the LED sits in the key whether it is lit
 * or not.
 */
export function SceneStrip({
  scenes,
  active,
  loading = false,
  onSet,
  onNew,
}: {
  scenes: Scene[];
  active: string | null;
  /** The scenes are being read: three placeholder keys hold the strip's room. */
  loading?: boolean;
  onSet: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <div className={s.strip} role="group" aria-label="Scenes">
      {loading &&
        [112, 88, 120].map((w, i) => <Skeleton key={i} width={w} height={40} radius={10} />)}
      {scenes.map((scene) => (
        <Chip
          key={scene.id}
          className={s.sceneKey}
          pressed={scene.id === active}
          label={scene.name}
          title={scene.name}
          onClick={() => onSet(scene.id)}
        >
          <span className="one-line">{scene.name}</span>
        </Chip>
      ))}
      <Button
        className={s.plusKey}
        aria-label="New scene"
        title="New scene (a copy of this one)"
        disabled={active === null}
        onClick={onNew}
      >
        <Plus size={16} weight="bold" />
      </Button>
    </div>
  );
}
