import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

class StubEventSource {
  static instances: StubEventSource[] = [];

  url: string;
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  closed = false;

  constructor(url: string) {
    this.url = url;
    StubEventSource.instances.push(this);
  }

  close() {
    this.closed = true;
    this.readyState = 2;
  }

  emitOpen() {
    this.readyState = 1;
    this.onopen?.(new Event("open"));
  }

  emitMessage(payload: unknown) {
    this.onmessage?.(
      new MessageEvent("message", { data: JSON.stringify(payload) }),
    );
  }

  emitError() {
    this.onerror?.(new Event("error"));
  }

  static latest(): StubEventSource {
    const last = StubEventSource.instances.at(-1);
    if (!last) {
      throw new Error("no EventSource was opened");
    }
    return last;
  }

  static reset() {
    StubEventSource.instances = [];
  }
}

vi.stubGlobal("EventSource", StubEventSource);

export { StubEventSource };

afterEach(() => {
  cleanup();
  StubEventSource.reset();
  vi.restoreAllMocks();
});
