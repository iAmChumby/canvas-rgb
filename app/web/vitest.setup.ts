// Test setup, wired through `setupFiles` in vitest.config.mts.
//
// Testing Library's `findBy*` / `waitFor` give up after 1 s by default, and vitest ends a test
// after 5 s. On a busy CI runner one render can take longer than a second, and the test fails
// for the machine's speed and not for the code (class `cause:timing-test`, #59).
//
// A `findBy*` returns the moment its element appears, so a long limit costs nothing on a pass.
// It only decides how long a test that is going to fail waits before it says so. The test
// timeout is above the async limit, so the finder reports "element not found" before vitest
// reports "test timed out".
import { configure } from "@testing-library/dom";
import { ASYNC_UTIL_TIMEOUT_MS } from "./vitest.limits";

configure({ asyncUtilTimeout: ASYNC_UTIL_TIMEOUT_MS });
