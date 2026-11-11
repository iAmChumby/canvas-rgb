import { describe, expect, it } from "vitest";
import { CPU_IDLE_LIMIT_C, cpuLimitInIdleRange, hotRuleShortText, hotRuleText } from "@/lib/fan-rules";

// Issue #255 review: the sentence names the line and the ceiling without decimals, rounded the
// same way the service rounds the status reason (ties away from zero for these positive values).
describe("the don't-exceed sentence", () => {
  it("fills in the line and the ceiling and rounds them like the status", () => {
    expect(hotRuleText(50.5, "CPU", 84.5)).toBe(
      "Above 51 °C the fan speeds up to bring CPU back under it; at 85 °C it runs at full speed.",
    );
  });

  it("names a board sensor by its own name", () => {
    expect(hotRuleText(50, "SYSTIN", 90)).toBe(
      "Above 50 °C the fan speeds up to bring SYSTIN back under it; at 90 °C it runs at full speed.",
    );
  });

  // Issue #297: with the idle-range warning under the control, the sentence is cut to one line so
  // the Fans right panel keeps the height it had; the warning carries the "run hard" half.
  it("has a one-line form for the idle-range warning", () => {
    expect(hotRuleShortText(50, 84)).toBe("Above 50 °C the fan speeds up; at 84 °C it runs at full speed.");
    expect(hotRuleShortText(undefined, undefined)).toBe(
      "Above its hot limit the fan speeds up; at its safety ceiling it runs at full speed.",
    );
  });
});

// Issue #297: a CPU line under the fixed 60 °C warning threshold (the code's 50 °C hot line
// plus 10 °C) keeps the hold engaged almost all the time; the editor warns. It is a fixed rule, so
// the warning says only what that rule means.
describe("the CPU hot-line warning threshold", () => {
  it("covers a 50 °C line and not a 75 °C one", () => {
    expect(CPU_IDLE_LIMIT_C).toBe(60);
    expect(cpuLimitInIdleRange("cpu", 50)).toBe(true);
    expect(cpuLimitInIdleRange("cpu", 59)).toBe(true);
    // The warning says "Below 60 °C", so a 60 °C line itself gets none.
    expect(cpuLimitInIdleRange("cpu", 60)).toBe(false);
    expect(cpuLimitInIdleRange("cpu", 75)).toBe(false);
  });

  it("is only about the CPU: a GPU or board line is the user's own call", () => {
    expect(cpuLimitInIdleRange("gpu", 50)).toBe(false);
    expect(cpuLimitInIdleRange("board", 50)).toBe(false);
  });
});
