/*
 * C's nine area keys, mapped to the area names the published API reference uses. Kept separate
 * from docs-data.mjs so client components can import it without pulling in Node's file system.
 */
export const AREA_KEYS = [
  { label: "Status", areas: ["Status and output"], tone: "a1" },
  { label: "Devices", areas: ["Devices and lights"], tone: "a1" },
  { label: "Effects", areas: ["Effects"], tone: "a2" },
  { label: "Stacks", areas: ["Stacks and patterns"], tone: "a2" },
  { label: "Fans", areas: ["Fans"], tone: "a3" },
  { label: "Scenes", areas: ["Scenes and chains"], tone: "a3" },
  { label: "Audio", areas: ["Audio"], tone: "a5" },
  { label: "Preview", areas: ["Preview"], tone: "a5" },
  {
    label: "System",
    areas: ["Displays and peripherals", "Settings and update", "Diagnostics"],
    tone: "a4",
  },
];
