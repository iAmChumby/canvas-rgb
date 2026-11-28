// @vitest-environment node
// Guard for #579 (R1): a preview inside the tray sends the tray nothing that reads or changes the
// real PC. `trayHost()` in `lib/host.ts` is null in a preview; `tauriHost()` is not. So only the
// functions listed here may call `tauriHost()`: a new tray command takes `trayHost()`.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const HOST = resolve(process.cwd(), "src/lib/host.ts");

/**
 * `isDemo` asks whether there is a tray at all (and says "demo" in a preview first); `trayHost` is
 * the gate; `openExternal` opens a link in the person's browser; `onNavigate` only listens for the
 * tray's own page request.
 */
const ALLOWED = ["isDemo", "onNavigate", "openExternal", "trayHost"];

/** The top-level functions whose body calls `tauriHost()`. */
function callers(source: string): string[] {
  const found = new Set<string>();
  let current = "";
  for (const line of source.split("\n")) {
    const declared = /^(?:export )?(?:async )?function (\w+)/.exec(line) ?? /^(?:export )?const (\w+)/.exec(line);
    if (declared?.[1]) current = declared[1];
    if (/^\s*(?:\/\/|\*|\/\*)/.test(line)) continue;
    if (/\btauriHost\(\)/.test(line) && !/function tauriHost\(/.test(line)) found.add(current);
  }
  return [...found].sort();
}

describe("a preview reaches the tray only where it is allowed (#579)", () => {
  it("only the listed functions call tauriHost()", () => {
    expect(callers(readFileSync(HOST, "utf8"))).toEqual(ALLOWED);
  });

  it("the matcher names the function a call sits in", () => {
    const sample = "export async function save(): Promise<void> {\n  const host = tauriHost();\n}\n";
    expect(callers(sample)).toEqual(["save"]);
  });
});
