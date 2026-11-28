// The editor's device-id scrub, held to the daemon's (#576, P6).
//
// The rules live in crates/canvas-diag/src/scrub.rs (read here, never written). Both sides run
// crates/canvas-diag/tests/fixtures/scrub-cases.json, and the prefix list is compared with the
// one in the Rust source, so a prefix added on one side alone fails here.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { ADAPTER_PREFIXES, DEVICE_TOKEN, scrubDeviceIds } from "@/lib/errors";

interface Case {
  input: string;
  expected: string;
}

/** Vitest runs from app/web; the crate sits two levels up. */
const CRATE = join(process.cwd(), "..", "..", "crates", "canvas-diag");
const cases = JSON.parse(readFileSync(join(CRATE, "tests", "fixtures", "scrub-cases.json"), "utf8")) as Case[];
const rust = readFileSync(join(CRATE, "src", "scrub.rs"), "utf8");

describe("the device-id scrub", () => {
  it("has cases to run", () => {
    expect(cases.length).toBeGreaterThan(0);
  });

  it.each(cases)("scrubs $input as the daemon does", ({ input, expected }) => {
    expect(scrubDeviceIds(input)).toBe(expected);
  });

  it("knows the prefixes and the token the daemon knows", () => {
    const list = /pub const ADAPTER_PREFIXES: &\[&str\] = &\[([^\]]*)\]/.exec(rust)?.[1] ?? "";
    expect(ADAPTER_PREFIXES).toEqual([...list.matchAll(/"([^"]+)"/g)].map((m) => m[1]));
    expect(rust).toContain(`pub const DEVICE_TOKEN: &str = "${DEVICE_TOKEN}";`);
  });

  it.each(ADAPTER_PREFIXES)("takes a %s id whole", (prefix) => {
    expect(scrubDeviceIds(`lost ${prefix}:A1:B2#z0 again`)).toBe("lost the device again");
  });

  it.each([
    // A prefix inside a longer word, and a prefix with nothing after the colon.
    "monitoring:on",
    "my_usb:1",
    "usb: not found",
  ])("leaves %j alone", (text) => {
    expect(scrubDeviceIds(text)).toBe(text);
  });

  it("ends an id where the daemon ends it", () => {
    expect(scrubDeviceIds('{\\"id\\":\\"usb:12\\"}')).toBe('{\\"id\\":\\"the device\\"}');
    expect(scrubDeviceIds("<hid:1.2>\nnext")).toBe("<the device>\nnext");
    expect(scrubDeviceIds("é cpu:0.\tdone")).toBe("é the device.\tdone");
  });
});
