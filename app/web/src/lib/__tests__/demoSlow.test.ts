import { afterEach, describe, expect, it } from "vitest";
import { answerDelayMs, slowQueryMs } from "@/lib/demoScenarios";

// The `?slow=` flag makes the demo daemon answer late so each wait of the loading kit can be seen.
function at(search: string) {
  window.history.replaceState(null, "", `/${search}`);
}

afterEach(() => at(""));

describe("the demo's ?slow flag", () => {
  it("has no effect without it", () => {
    at("");
    expect(slowQueryMs()).toBeNull();
    expect(answerDelayMs()).toBe(2);
  });

  it("delays every answer by the given milliseconds", () => {
    at("?slow=1500");
    expect(slowQueryMs()).toBe(1500);
    expect(answerDelayMs()).toBe(1500);
  });

  it("means 1500 with no number, caps at 10 s, and ignores nonsense", () => {
    at("?slow");
    expect(answerDelayMs()).toBe(1500);
    at("?slow=999999");
    expect(answerDelayMs()).toBe(10_000);
    at("?slow=abc");
    expect(slowQueryMs()).toBeNull();
    at("?slow=-5");
    expect(slowQueryMs()).toBeNull();
  });

  it("leaves the older ?state=slow scenario at 700 ms", () => {
    at("?state=slow");
    expect(answerDelayMs()).toBe(700);
  });
});
