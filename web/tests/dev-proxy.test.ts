import { describe, expect, it } from "vitest";
import {
  apiProxy,
  backendAddress,
  parseEnvFile,
} from "../dev-proxy";

describe("Vite API proxy", () => {
  it("derives every Axum route target from the configured backend address", () => {
    const proxy = apiProxy("127.0.0.1:49123");

    expect(proxy["/api"]).toMatchObject({ target: "http://127.0.0.1:49123" });
    expect(proxy["/health"]).toMatchObject({ target: "http://127.0.0.1:49123" });
    expect(proxy["/ready"]).toMatchObject({ target: "http://127.0.0.1:49123" });
  });

  it("requires a backend address for the development server", () => {
    expect(() => apiProxy("")).toThrow("BLACKBOX_HTTP_ADDR");
  });
});

describe("the env file is the only source of the port", () => {
  it("expands the address from the port defined beside it", () => {
    expect(
      backendAddress(
        [
          "BLACKBOX_HTTP_PORT=8787",
          "BLACKBOX_HTTP_ADDR=127.0.0.1:${BLACKBOX_HTTP_PORT}",
        ].join("\n"),
      ),
    ).toBe("127.0.0.1:8787");
  });

  it("moves the proxy when the one port changes", () => {
    const env = (port: string) =>
      [
        `BLACKBOX_HTTP_PORT=${port}`,
        "BLACKBOX_HTTP_ADDR=127.0.0.1:${BLACKBOX_HTTP_PORT}",
      ].join("\n");

    expect(apiProxy(backendAddress(env("8787")))["/api"]).toMatchObject({
      target: "http://127.0.0.1:8787",
    });
    expect(apiProxy(backendAddress(env("49601")))["/api"]).toMatchObject({
      target: "http://127.0.0.1:49601",
    });
  });

  it("accepts an address written out in full", () => {
    expect(backendAddress("BLACKBOX_HTTP_ADDR=127.0.0.1:9000")).toBe(
      "127.0.0.1:9000",
    );
  });

  it("refuses an env file that names no address", () => {
    expect(() => backendAddress("BLACKBOX_HTTP_PORT=8787")).toThrow(
      "BLACKBOX_HTTP_ADDR",
    );
    expect(() => backendAddress("BLACKBOX_HTTP_ADDR=")).toThrow(
      "BLACKBOX_HTTP_ADDR",
    );
    expect(() => backendAddress("")).toThrow("no port is defaulted");
  });

  it("refuses a reference the same file never defines", () => {
    expect(() =>
      backendAddress("BLACKBOX_HTTP_ADDR=127.0.0.1:${MISSING_PORT}"),
    ).toThrow("unresolved reference");
  });

  it("ignores comments, blanks and quoting the way Compose does", () => {
    const values = parseEnvFile(
      [
        "# the backend port",
        "",
        '  BLACKBOX_HTTP_PORT="8787"  ',
        "BLACKBOX_HTTP_ADDR='127.0.0.1:${BLACKBOX_HTTP_PORT}'",
        "NOT_AN_ASSIGNMENT",
        "=orphan",
      ].join("\n"),
    );

    expect(values.get("BLACKBOX_HTTP_PORT")).toBe("8787");
    expect(values.get("BLACKBOX_HTTP_ADDR")).toBe("127.0.0.1:8787");
    expect(values.has("NOT_AN_ASSIGNMENT")).toBe(false);
    expect(values.has("")).toBe(false);
  });

  it("keeps a value that refers to itself rather than looping", () => {
    const values = parseEnvFile("SELF=${SELF}/callback");
    expect(values.get("SELF")).toBe("${SELF}/callback");
  });

  it("reads the redirect URL through the same one port", () => {
    const values = parseEnvFile(
      [
        "BLACKBOX_HTTP_PORT=8787",
        "BLACKBOX_HTTP_ADDR=127.0.0.1:${BLACKBOX_HTTP_PORT}",
        "DHAN_REDIRECT_URL=http://127.0.0.1:${BLACKBOX_HTTP_PORT}/dhan/callback",
      ].join("\n"),
    );

    expect(values.get("DHAN_REDIRECT_URL")).toBe(
      "http://127.0.0.1:8787/dhan/callback",
    );
    expect(values.get("BLACKBOX_HTTP_ADDR")).toBe("127.0.0.1:8787");
  });
});
