// The colour of a group row.
//
// The accent used to be the row's place in the scene: the first group was one colour, the second
// another, and a group that moved took the colour of the place it came to. A group is a thing of
// its own, though, and it keeps its colour when it moves: the colour comes from a stable hash of
// the group's own id (#384).
//
// A scene made before this may show a different colour once: the colour a group has now is the one
// its id gives it.

/** The five accents a group's row can have. */
const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

/**
 * The accent of a group, from a stable hash of its id.
 *
 * The hash is `hash * 33 + the character's number`, the string hash of Daniel J. Bernstein: the
 * same id always gives the same number, on every window and every run, and the numbers spread
 * evenly over the five accents. So a group keeps its colour when it moves and when the page is
 * read again.
 */
export function groupAccent(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) {
    hash = (Math.imul(hash, 33) + id.charCodeAt(i)) >>> 0;
  }
  return ACCENTS[hash % ACCENTS.length] as string;
}
