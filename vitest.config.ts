import { defineConfig } from "vitest/config";

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify("0.0.0-test"),
    __APP_BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00.000Z"),
  },
  test: {
    globals: true,
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/__tests__/setup.ts"],
  },
});
