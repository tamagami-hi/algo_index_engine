import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const engine = process.env.ENGINE_ORIGIN ?? "http://127.0.0.1:8081";

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
  server: {
    port: 5178,
    strictPort: true,
    proxy: {
      "/api": { target: engine, changeOrigin: true },
      "/health": { target: engine, changeOrigin: true },
      "/ready": { target: engine, changeOrigin: true },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary"],
      include: ["src/**/*.ts", "src/**/*.tsx"],
      exclude: ["src/test/**", "src/main.tsx", "src/types/**"],
    },
  },
});
