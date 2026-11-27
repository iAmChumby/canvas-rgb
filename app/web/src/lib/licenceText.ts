/**
 * Reflows a generated licence notice for display (#584).
 *
 * cargo-about and the npm notices script copy the upstream licence files with
 * their layout intact: ~72-column lines, every line indented. Shown as
 * pre-wrapped text those breaks become hard breaks and each line wraps again
 * on a phone. The notice files stay verbatim; this undoes the layout at render
 * time so paragraphs wrap to the container instead.
 *
 * Blank lines stay paragraph breaks. Standalone lines — the `[1]` notice
 * separator, a copyright line, a licence title, the Apache version and URL —
 * stay on a line of their own. Everything else in a block is one paragraph.
 */

const SEPARATOR = /^\[\d+\]$/;
// A copyright *notice* carries a year, (c) or (C); "copyright license to
// reproduce ..." is ordinary prose and must stay inside its paragraph.
const COPYRIGHT = /^copyright\b.*(?:\(\s*c\s*\)|©|\b(?:19|20)\d{2}\b)/i;
const TITLE = /^(?:[A-Z][\w.+-]*\s+){1,3}License\b/;
const VERSION = /^version\s+\d/i;
const URL = /^https?:\/\/\S+$/i;
const HARD = [SEPARATOR, COPYRIGHT, TITLE, VERSION, URL];
const BREAK_BEFORE = [SEPARATOR, COPYRIGHT, TITLE];

const isHardLine = (line: string) => HARD.some((re) => re.test(line));
const indentOf = (line: string) => /^[\t ]*/.exec(line)?.[0].length ?? 0;

export function reflowLicenceText(text: string): string[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const indents = lines.filter((line) => line.trim()).map(indentOf);
  const common = indents.length ? Math.min(...indents) : 0;

  const blocks: string[][] = [];
  let block: string[] = [];
  const flush = () => {
    if (block.length) blocks.push(block);
    block = [];
  };

  for (const raw of lines) {
    const line = raw.slice(common).trim();
    if (!line) {
      flush();
      continue;
    }
    if (BREAK_BEFORE.some((re) => re.test(line))) flush();
    block.push(line);
    // A heading, separator or copyright line never shares a paragraph with
    // the text around it, even when the notice has no blank line there.
    if (isHardLine(line)) flush();
  }
  flush();

  return blocks.flatMap((linesInBlock) =>
    linesInBlock.every(isHardLine) ? linesInBlock : [linesInBlock.join(" ")],
  );
}
