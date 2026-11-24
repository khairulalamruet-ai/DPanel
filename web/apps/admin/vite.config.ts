import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://localhost:3000",
        changeOrigin: true,
        secure: false,
      },
    },
  },
  resolve: {
    alias: {
      "@builtpanel/ui": path.resolve(__dirname, "../../packages/ui/src"),
      "@builtpanel/api": path.resolve(__dirname, "../../packages/api/src"),
      "@builtpanel/auth": path.resolve(__dirname, "../../packages/auth/src"),
      "@builtpanel/i18n": path.resolve(__dirname, "../../packages/i18n/src"),
      "@builtpanel/shell": path.resolve(__dirname, "../../packages/shell/src"),
      "@builtpanel/modules": path.resolve(__dirname, "../../packages/modules/src"),
    },
  },
});
