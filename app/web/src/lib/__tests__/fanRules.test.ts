import { describe, expect, it } from "vitest";
import { hotRuleText } from "@/lib/fan-rules";

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
});
