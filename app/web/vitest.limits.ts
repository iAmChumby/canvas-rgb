// The test limits, in one place so vitest.config.mts, vitest.setup.ts and the test that pins
// them read the same numbers (#59). No side effects: importing this changes nothing.
export const ASYNC_UTIL_TIMEOUT_MS = 10_000;
export const TEST_TIMEOUT_MS = 20_000;
