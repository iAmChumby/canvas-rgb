// Class guard for #120 (cause:timing-test, second instance): a test must wait for the editor
// to mount through the shared helper, never read a CodeMirror node synchronously. This test
// reads every `*.test.tsx` file as text and fails on any direct `.cm-*` query, so a new test
// that copies the old direct `.cm-content` query pattern cannot land. The helper module
// (`src/test/editor.ts`) is not a test file, so its three queries are the only ones allowed.
// Built from fragments so this file does not itself contain a forbidden call.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = resolve(process.cwd(), "src");

function testFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...testFiles(path));
    else if (entry.name.endsWith(".test.tsx")) out.push(path);
  }
  return out.sort();
}

const FORBIDDEN = [
  "querySelector(" + '".cm-content"',
  "querySelector(" + '".cm-line"',
  "querySelector(" + '".cm-lint-marker',
  "querySelectorAll(" + '".cm-content"',
  "querySelectorAll(" + '".cm-line"',
  "querySelectorAll(" + '".cm-lint-marker',
];

describe("editor queries in tests go through the shared helper (#120)", () => {
  it("no test file queries .cm-content, .cm-line or .cm-lint-marker directly", () => {
    const offenders: string[] = [];
    for (const file of testFiles(SRC)) {
      const lines = readFileSync(file, "utf8").replace(/\r\n/g, "\n").split("\n");
      lines.forEach((line, i) => {
        if (FORBIDDEN.some((pattern) => line.includes(pattern))) {
          offenders.push(`${relative(process.cwd(), file)}:${i + 1}: ${line.trim()}`);
        }
      });
    }
    expect(offenders).toEqual([]);
  });
});
