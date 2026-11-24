import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
  plugins: [react()],
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./vitest.setup.ts"],
    testTimeout: 20000,
    hookTimeout: 20000,
    include: ["packages/**/*.test.{ts,tsx}", "apps/**/*.test.{ts,tsx}"],
  },
  resolve: {
    alias: {
      "@builtpanel/ui": path.resolve(__dirname, "./packages/ui/src"),
      "@builtpanel/api": path.resolve(__dirname, "./packages/api/src"),
      "@builtpanel/auth": path.resolve(__dirname, "./packages/auth/src"),
      "@builtpanel/i18n": path.resolve(__dirname, "./packages/i18n/src"),
      "@builtpanel/shell": path.resolve(__dirname, "./packages/shell/src"),
      "@builtpanel/modules": path.resolve(__dirname, "./packages/modules/src"),
    },
  },
});
