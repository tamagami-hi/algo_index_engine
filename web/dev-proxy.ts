import { readFileSync } from "node:fs";

export const ADDRESS_VARIABLE = "BLACKBOX_HTTP_ADDR";

export interface ProxyTarget {
  target: string;
  changeOrigin: boolean;
}

export function parseEnvFile(text: string): Map<string, string> {
  const values = new Map<string, string>();

  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (trimmed === "" || trimmed.startsWith("#")) {
      continue;
    }
    const separator = trimmed.indexOf("=");
    if (separator <= 0) {
      continue;
    }
    const key = trimmed.slice(0, separator).trim();
    let value = trimmed.slice(separator + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"') && value.length > 1) ||
      (value.startsWith("'") && value.endsWith("'") && value.length > 1)
    ) {
      value = value.slice(1, -1);
    }
    values.set(key, value);
  }

  const expand = (raw: string, seen: Set<string>): string =>
    raw.replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (whole, name: string) => {
      if (seen.has(name)) {
        return whole;
      }
      const referenced = values.get(name);
      return referenced === undefined
        ? whole
        : expand(referenced, new Set(seen).add(name));
    });

  for (const [key, value] of [...values]) {
    values.set(key, expand(value, new Set([key])));
  }

  return values;
}

export function backendAddress(envText: string): string {
  const configured = parseEnvFile(envText).get(ADDRESS_VARIABLE)?.trim() ?? "";
  if (configured === "") {
    throw new Error(
      `${ADDRESS_VARIABLE} must be set in the env file so the dev server knows where the engine listens; no port is defaulted`,
    );
  }
  if (/\$\{/.test(configured)) {
    throw new Error(
      `${ADDRESS_VARIABLE} still contains an unresolved reference; define every variable it names in the same env file`,
    );
  }
  return configured;
}

export function readBackendAddress(envPath: string): string {
  return backendAddress(readFileSync(envPath, "utf8"));
}

export function apiProxy(address: string): Record<string, ProxyTarget> {
  const configured = address.trim();
  if (configured === "") {
    throw new Error(
      `${ADDRESS_VARIABLE} must be set so the dev server knows where the engine listens; no port is defaulted`,
    );
  }
  const target = `http://${configured}`;
  return {
    "/api": { target, changeOrigin: true },
    "/health": { target, changeOrigin: true },
    "/ready": { target, changeOrigin: true },
  };
}
