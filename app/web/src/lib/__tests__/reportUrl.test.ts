import { describe, expect, it } from "vitest";
import { buildReportUrl, REPORT_URL_MAX } from "@/lib/reportUrl";
import type { AboutReport } from "@/proto/AboutReport";

function report(over: Partial<AboutReport> = {}): AboutReport {
  return {
    markdown: "not used by the URL builder",
    version: "0.20.0",
    windows: "Windows 11 Pro 10.0.26100",
    hardware: "RGB things",
    other_rgb_apps: "none seen",
    ...over,
  } as AboutReport;
}

describe("the Report issue URL (#575 R7, P1)", () => {
  it("stays under the cap on a worst-case report and drops hardware first", () => {
    const huge = report({ hardware: "device ".repeat(2000) });
    const built = buildReportUrl(huge, `Error shown: ${"a".repeat(9000)}`);
    expect(built.url.length).toBeLessThanOrEqual(REPORT_URL_MAX);
    expect(built.droppedHardware).toBe(true);
    expect(new URL(built.url).searchParams.has("hardware")).toBe(false);
    expect(built.truncated).toBe(true);
    expect(built.whatHappened).toContain("(truncated)");
  });

  it("keeps every field when the report fits", () => {
    const built = buildReportUrl(report(), "The device did not answer.");
    expect(built.url.length).toBeLessThanOrEqual(REPORT_URL_MAX);
    expect(built.droppedHardware).toBe(false);
    expect(built.truncated).toBe(false);
    const params = new URL(built.url).searchParams;
    expect(params.get("template")).toBe("bug_report.yml");
    expect(params.get("version")).toBe("0.20.0");
    expect(params.get("what-happened")).toContain("Error shown: The device did not answer.");
    for (const never of ["logs", "steps", "expected", "status"]) expect(params.has(never)).toBe(false);
  });

  it("never takes what-happened from a plain local Error", () => {
    const built = buildReportUrl(report(), null);
    expect(built.whatHappened).toBe("");
  });

  it("scrubs device ids from the text copied into the URL", () => {
    const built = buildReportUrl(report(), "openrgb:SERIAL-42 did not answer");
    expect(built.whatHappened).toContain("the device did not answer");
    expect(built.url).not.toContain("SERIAL-42");
  });

  it("takes every part of a many-part device id out of the URL and the copied text", () => {
    const built = buildReportUrl(report(), "royuan:3151:4015 and eightbitdo-ultimate-2:2dc8 did not answer.");
    expect(built.whatHappened).toContain("Error shown: the device and the device did not answer.");
    for (const part of ["royuan", "3151", "4015", "eightbitdo", "2dc8"]) {
      expect(built.url).not.toContain(part);
      expect(built.clipboardText).not.toContain(part);
    }
  });

  it("measures encoded length, not characters, for CJK, newlines and emoji", () => {
    const cases = [
      buildReportUrl(report({ hardware: "設".repeat(3000) }), "x".repeat(500)),
      buildReportUrl(report({ hardware: "\n".repeat(3000) }), "x".repeat(500)),
      buildReportUrl(report({ hardware: "😀".repeat(3000) }), "x".repeat(500)),
      buildReportUrl(report({ hardware: "ok", other_rgb_apps: "other ".repeat(4000) }), "y".repeat(500)),
    ];
    for (const built of cases) {
      expect(built.url.length).toBeLessThanOrEqual(REPORT_URL_MAX);
      expect(() => new URL(built.url)).not.toThrow();
      // No half of a surrogate pair survives: every emoji is whole or gone.
      expect(/(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/.test(built.url)).toBe(false);
    }
    // Newlines cost three encoded characters each, so a 3000-newline report must be cut.
    expect(cases[1]!.droppedHardware).toBe(true);
    // An oversized other-rgb-apps field is cut too, not left in the address.
    expect(new URL(cases[3]!.url).searchParams.get("other-rgb-apps")?.length).toBeLessThan(4000 * 6);
  });

  it("never throws and always fits for random Unicode strings", () => {
    const alphabet = ["a", " ", "\n", "設", "😀", "!", "&", "%", "ı", "𝄞", "\uD800"];
    let seed = 12345;
    const next = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff);
    for (let i = 0; i < 200; i++) {
      const pick = (n: number) => Array.from({ length: n }, () => alphabet[next() % alphabet.length]).join("");
      const built = buildReportUrl(
        report({ hardware: pick(next() % 60), other_rgb_apps: pick(next() % 60), windows: pick(5), version: pick(5) }),
        pick(next() % 60),
      );
      expect(built.url.length).toBeLessThanOrEqual(REPORT_URL_MAX);
      expect(() => new URL(built.url)).not.toThrow();
    }
  });
});
