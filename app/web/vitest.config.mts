import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import { TEST_TIMEOUT_MS } from "./vitest.limits";

export default defineConfig({
  // tsconfig has jsx: react-jsx, which the transform reads; no React plugin needed.
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // Raises Testing Library's async limit; the test timeout is raised with it (vitest.setup.ts, #59).
    setupFiles: ["./vitest.setup.ts"],
    testTimeout: TEST_TIMEOUT_MS,
  },
});
