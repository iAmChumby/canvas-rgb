import { describe, expect, it } from "vitest";
import { buildReportUrl, REPORT_URL_MAX } from "@/lib/reportUrl";
import type { AboutReport } from "@/proto/AboutReport";

function report(hardware: string): AboutReport {
  return {
    markdown: "not used by the URL builder",
    version: "0.20.0",
    windows: "Windows 11 Pro 10.0.26100",
    hardware: "RGB things",
    other_rgb_apps: "none seen",
    ...(hardware ? { hardware } : {}),
  } as AboutReport;
}

describe("the Report issue URL (#575 R7)", () => {
  it("stays under the cap on a worst-case report and drops hardware first", () => {
    const huge = report("device ".repeat(2000));
    const built = buildReportUrl(huge, `Error shown: ${"a".repeat(9000)}`);
    expect(built.url.length).toBeLessThanOrEqual(REPORT_URL_MAX);
    expect(built.droppedHardware).toBe(true);
    expect(new URL(built.url).searchParams.has("hardware")).toBe(false);
    expect(built.truncated).toBe(true);
    expect(built.whatHappened).toContain("(truncated)");
  });

  it("keeps every field when the report fits", () => {
    const built = buildReportUrl(report("one keyboard"), "The device did not answer.");
    expect(built.url.length).toBeLessThanOrEqual(REPORT_URL_MAX);
    expect(built.droppedHardware).toBe(false);
    expect(built.truncated).toBe(false);
    const params = new URL(built.url).searchParams;
    expect(params.get("template")).toBe("bug_report.yml");
    expect(params.get("version")).toBe("0.20.0");
    expect(params.get("what-happened")).toContain("Error shown: The device did not answer.");
    expect(params.has("logs")).toBe(false);
    expect(params.has("steps")).toBe(false);
    expect(params.has("expected")).toBe(false);
    expect(params.has("status")).toBe(false);
  });

  it("never takes what-happened from a plain local Error", () => {
    // A plain Error's message is a local detail (R6). Only a scrubbed title is passed in.
    const built = buildReportUrl(report("one keyboard"), null);
    expect(built.whatHappened).toBe("");
  });
});
