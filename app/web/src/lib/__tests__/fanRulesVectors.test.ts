// The editor's copy of the daemon's fan rules, checked against the Rust rules' own vectors.
//
// The daemon applies the fan floors in crates/canvas-proto/src/fans.rs (the spine, read-only). The
// editor copies those constants and rules in app/web/src/lib/fan-rules.ts so it draws the same
// floor and hot-floor line. The two are tied by fan-rules.vectors.json: the Rust example prints
// the constants and verdicts, the Rust test keeps the file fresh, and this test runs the
// TypeScript mirror over every row.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import * as fanRules from "@/lib/fan-rules";
import { baseFloor, cpuLimitInIdleRange, hotFloor } from "@/lib/fan-rules";

interface ConstantRow {
  name: string;
  value: number | string;
}

type CaseRow =
  | { name: string; rule: "baseFloor"; header: string; configured: number; expected: number }
  | { name: string; rule: "hotFloor"; header: string; configured: number; expected: number }
  | { name: string; rule: "cpuLimitInIdleRange"; kind: string; limit_c: number; expected: boolean };

interface Vectors {
  note: string;
  constants: ConstantRow[];
  cases: CaseRow[];
}

/**
 * Where the shared vectors live: next to this test, whether the tests run from the editor's
 * folder or from the repository root (as drive.vectors.json and arrangement.vectors.json are
 * found).
 */
function vectorsPath(): string {
  const names = [
    join(process.cwd(), "src/lib/__tests__/fan-rules.vectors.json"),
    join(process.cwd(), "app/web/src/lib/__tests__/fan-rules.vectors.json"),
  ];
  const found = names.find((name) => existsSync(name));
  if (!found) {
    throw new Error(`the Rust fan vectors are not there: looked in ${names.join(" and ")}`);
  }
  return found;
}

function vectors(): Vectors {
  return JSON.parse(readFileSync(vectorsPath(), "utf8")) as Vectors;
}

/** The spine constant name -> the editor's export name (they were shortened in the mirror). */
const exportName: Record<string, string> = {
  CPU_FAN_HEADER: "CPU_FAN",
  DEFAULT_FLOOR_PERCENT: "DEFAULT_FLOOR",
  CPU_FAN_MIN_PERCENT: "CPU_FAN_MIN",
  GPU_FAN_HEADER_PREFIX: "GPU_FAN",
  GPU_FAN_MIN_PERCENT: "GPU_FAN_MIN",
  HOT_TEMP_C: "HOT_TEMP_C",
  HOT_FLOOR_PERCENT: "HOT_FLOOR",
  HOT_CPU_FAN_FLOOR_PERCENT: "HOT_CPU_FAN_FLOOR",
  CPU_IDLE_LIMIT_C: "CPU_IDLE_LIMIT_C",
};

describe("the Rust fan rules' own output", () => {
  it("fan-rules.ts mirrors every spine constant", () => {
    const doc = vectors();
    // A trimmed or stale file fails here rather than passing quietly.
    expect(doc.constants.length).toBe(9);
    expect(Object.keys(exportName).sort()).toEqual(doc.constants.map((row) => row.name).sort());
    const exports = fanRules as unknown as Record<string, number | string>;
    for (const row of doc.constants) {
      const name = exportName[row.name] ?? row.name;
      expect(exports[name], row.name).toBe(row.value);
    }
  });

  it("fan-rules.ts matches the Rust fan rules on every row", () => {
    const doc = vectors();
    // A trimmed or stale file fails here rather than passing quietly.
    expect(doc.cases.length).toBe(33);
    for (const row of doc.cases) {
      const actual =
        row.rule === "baseFloor"
          ? baseFloor(row.header, row.configured)
          : row.rule === "hotFloor"
            ? hotFloor(row.header, row.configured)
            : cpuLimitInIdleRange(row.kind, row.limit_c);
      expect(actual, `${row.name}: ${row.rule}`).toBe(row.expected);
    }
  });
});
