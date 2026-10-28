// The effect categories the gallery filters on.
//
// The map is the editor's own: the daemon's effect list carries a name and a description, but
// no category, and the spine does not get a field for one. A built-in effect's id is stable
// (the daemon's registry names them), so a static map is enough. An effect the user added is
// in no category and shows under All and Favourites only.

export type EffectCategory = "ambient" | "motion" | "reactive" | "nature";

export interface CategoryOption {
  value: EffectCategory;
  label: string;
  /** One line on what belongs here. Shown as the chip's tooltip. */
  hint: string;
}

export const EFFECT_CATEGORIES: CategoryOption[] = [
  { value: "ambient", label: "Solid & ambient", hint: "One colour, held or pulsing" },
  { value: "motion", label: "Motion", hint: "Patterns that move on their own" },
  { value: "reactive", label: "Reactive", hint: "Effects that read the screen or the audio" },
  { value: "nature", label: "Nature", hint: "Weather, fire and the northern lights" },
];

const BY_ID: Record<string, EffectCategory> = {
  solid: "ambient",
  gradient: "ambient",
  breathing: "ambient",
  rainbow_wave: "motion",
  spiral: "motion",
  color_cycle: "motion",
  screen_ambience: "reactive",
  audio_spectrum: "reactive",
  aurora: "nature",
  rain: "nature",
  fire: "nature",
};

/** The category of an effect, or null when it has none (a user effect). */
export function categoryOf(id: string): EffectCategory | null {
  return BY_ID[id] ?? null;
}

/** How many of the effects in `ids` are in each category. */
export function categoryCounts(ids: string[]): Record<EffectCategory, number> {
  const counts: Record<EffectCategory, number> = { ambient: 0, motion: 0, reactive: 0, nature: 0 };
  for (const id of ids) {
    const c = BY_ID[id];
    if (c) counts[c] += 1;
  }
  return counts;
}
