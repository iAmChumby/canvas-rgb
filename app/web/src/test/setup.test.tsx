import { render, screen } from "@testing-library/react";
import { getConfig } from "@testing-library/dom";
import { useEffect, useState } from "react";
import { expect, test } from "vitest";
import { ASYNC_UTIL_TIMEOUT_MS, TEST_TIMEOUT_MS } from "../../vitest.limits";

// The limits that keep a slow CI runner from failing a test for the machine's speed (#59).
test("the async limit and the test timeout are raised together, the test timeout above", (ctx) => {
  expect(getConfig().asyncUtilTimeout).toBe(ASYNC_UTIL_TIMEOUT_MS);
  expect(ctx.task.timeout).toBe(TEST_TIMEOUT_MS);
  expect(ASYNC_UTIL_TIMEOUT_MS).toBeGreaterThan(1000); // Testing Library's own default
  expect(TEST_TIMEOUT_MS).toBeGreaterThan(ASYNC_UTIL_TIMEOUT_MS);
});

function Slow({ afterMs }: { afterMs: number }) {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    const id = setTimeout(() => setShown(true), afterMs);
    return () => clearTimeout(id);
  }, [afterMs]);
  return shown ? <p>arrived</p> : null;
}

test("an element that appears after the old 1 s default is still found", async () => {
  render(<Slow afterMs={1500} />);
  expect(await screen.findByText("arrived")).toBeTruthy();
});
