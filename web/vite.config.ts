import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { ADDRESS_VARIABLE, apiProxy, readBackendAddress } from "./dev-proxy";

const ENV_FILE = fileURLToPath(new URL("../.env", import.meta.url));

export default defineConfig(({ command }) => {
  const serving = command === "serve" && process.env.VITEST === undefined;

  let proxy;
  if (serving) {
    try {
      proxy = apiProxy(readBackendAddress(ENV_FILE));
    } catch (cause) {
      console.warn(
        `[vite] not proxying /api, /health or /ready: ${String(cause)}\n` +
          `[vite] set ${ADDRESS_VARIABLE} in ${ENV_FILE}, the one place this repository takes the port from.`,
      );
    }
  }

  return {
    plugins: [react()],
    build: {
      outDir: "dist",
      emptyOutDir: true,
      sourcemap: false,
    },
    server: {
      port: 5178,
      strictPort: true,
      ...(proxy === undefined ? {} : { proxy }),
    },
    test: {
      environment: "jsdom",
      globals: true,
      setupFiles: ["./tests/support/setup.ts"],
      include: ["tests/**/*.test.ts", "tests/**/*.test.tsx"],
      coverage: {
        provider: "v8",
        reporter: ["text", "json-summary"],
        include: ["src/**/*.ts", "src/**/*.tsx", "dev-proxy.ts"],
        exclude: ["src/main.tsx", "src/types/**"],
      },
    },
  };
});
