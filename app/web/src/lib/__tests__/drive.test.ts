// The editor's copy of the daemon's drive rule, checked against the Rust rule's own vectors.
//
// The daemon decides which devices it writes to (crates/canvas-adapters/src/drive.rs,
// is_driven). The editor copies that rule in app/web/src/lib/drive.ts so it offers Remove only
// on lights the daemon would delete. The two are tied by drive.vectors.json: the Rust example
// prints the ids and its verdicts, the Rust test keeps the file fresh, and this test runs the
// TypeScript rule over every row.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { NOT_DRIVEN_DEVICES, driven } from "@/lib/drive";

interface VectorCase {
  name: string;
  id: string;
  driven: boolean;
}

interface Vectors {
  note: string;
  not_driven: string[];
  cases: VectorCase[];
}

/**
 * Where the shared vectors live: next to this test, whether the tests run from the editor's
 * folder or from the repository root (as arrangement.vectors.json is found).
 */
function vectorsPath(): string {
  const names = [
    join(process.cwd(), "src/lib/__tests__/drive.vectors.json"),
    join(process.cwd(), "app/web/src/lib/__tests__/drive.vectors.json"),
  ];
  const found = names.find((name) => existsSync(name));
  if (!found) {
    throw new Error(`the Rust drive vectors are not there: looked in ${names.join(" and ")}`);
  }
  return found;
}

describe("the Rust drive rule's own output", () => {
  it("drive.ts lists exactly the HID paths the daemon never drives", () => {
    // A pattern added on one side only, that no case matches, fails here (#501 review).
    const vectors = JSON.parse(readFileSync(vectorsPath(), "utf8")) as Vectors;
    expect([...NOT_DRIVEN_DEVICES]).toEqual(vectors.not_driven);
  });

  it("drive.ts matches the Rust drive vectors on every id", () => {
    const vectors = JSON.parse(readFileSync(vectorsPath(), "utf8")) as Vectors;
    // A trimmed or stale file fails here rather than passing quietly.
    expect(vectors.cases.length).toBe(20);
    for (const testCase of vectors.cases) {
      expect(driven(testCase.id), `${testCase.name}: ${testCase.id}`).toBe(testCase.driven);
    }
  });
});
