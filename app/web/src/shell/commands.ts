// The command palette's scene rows (Task 11): one "Set scene: <name>" per scene. There is no
// layout command: the palette's pages, its own actions and these scenes are the whole list.

import type { Command } from "@/components/CommandPalette";
import type { ScenesResponse } from "@/proto/ScenesResponse";

export function sceneCommands(answer: ScenesResponse | null, set: (id: string) => void): Command[] {
  if (!answer) return [];
  return answer.scenes.map((scene) => ({
    id: `scene-${scene.id}`,
    group: "Scenes",
    label: `Set scene: ${scene.name}`,
    hint: scene.id === answer.active ? "active" : undefined,
    run: () => set(scene.id),
  }));
}
