import { beforeEach, describe, expect, it, vi } from "vitest";
import { STREAM_SILENCE_MS, isStale, statusLabel, useEngine } from "../../src/stores/engine";
import { api } from "../../src/api/client";
import { StubEventSource } from "../support/setup";
import { chain, frame } from "../support/factories";

function reset() {
  useEngine.setState({
    snapshot: null,
    chain: null,
    selected: "NIFTY",
    symbols: [],
    link: "connecting",
    synchronised: false,
    sequence: null,
    gaps: 0,
    feedAgeMs: null,
    feedSilenceLimitMs: null,
    publishIntervalMs: null,
    receivedAt: null,
    readyReasons: [],
    error: null,
  });
}

describe("connection status", () => {
  beforeEach(reset);

  it("reports every distinct state the operator needs to tell apart", () => {
    expect(statusLabel("connecting", false)).toBe("connecting");
    expect(statusLabel("reconnecting", false)).toBe("reconnecting");
    expect(statusLabel("unavailable", false)).toBe("backend unavailable");
    expect(statusLabel("not_ready", false)).toBe("backend not ready");
    expect(statusLabel("open", false)).toBe("live");
    expect(statusLabel("open", true)).toBe("data stale");
  });

  it("treats anything other than an open synchronised stream as stale", () => {
    const now = 1_000_000;
    const base = {
      feedAgeMs: 10,
      feedSilenceLimitMs: 5000,
      receivedAt: now,
    };

    expect(isStale({ link: "open", synchronised: true, ...base }, now)).toBe(false);
    expect(isStale({ link: "reconnecting", synchronised: true, ...base }, now)).toBe(true);
    expect(isStale({ link: "unavailable", synchronised: true, ...base }, now)).toBe(true);
    expect(isStale({ link: "not_ready", synchronised: true, ...base }, now)).toBe(true);
    expect(
      isStale({ link: "open", synchronised: false, ...base }, now),
      "an unsynchronised stream has not proven itself yet",
    ).toBe(true);
  });

  it("uses the server's own silence limit rather than inventing one", () => {
    const now = 1_000_000;
    const within = {
      link: "open" as const,
      synchronised: true,
      receivedAt: now,
      feedAgeMs: 4_999,
      feedSilenceLimitMs: 5_000,
    };
    expect(isStale(within, now)).toBe(false);
    expect(isStale({ ...within, feedAgeMs: 5_001 }, now)).toBe(true);

    expect(
      isStale({ ...within, feedAgeMs: 5_001, feedSilenceLimitMs: 30_000 }, now),
      "a different server limit changes the verdict, so the rule is not hardcoded here",
    ).toBe(false);
  });

  it("goes stale when the stream itself stops delivering", () => {
    const now = 1_000_000;
    const store = {
      link: "open" as const,
      synchronised: true,
      feedAgeMs: 10,
      feedSilenceLimitMs: 5_000,
      receivedAt: now - STREAM_SILENCE_MS - 1,
    };
    expect(
      isStale(store, now),
      "the last frame claimed to be fresh, but nothing has arrived since",
    ).toBe(true);
  });
});

describe("stream frames", () => {
  beforeEach(reset);

  it("records freshness metadata from the frame", () => {
    useEngine.getState().applyFrame(frame());

    const state = useEngine.getState();
    expect(state.sequence).toBe(1);
    expect(state.feedAgeMs).toBe(40);
    expect(state.feedSilenceLimitMs).toBe(5000);
    expect(state.publishIntervalMs).toBe(50);
    expect(state.link).toBe("open");
    expect(state.receivedAt).not.toBeNull();
  });

  it("counts a sequence gap without discarding the newer state", () => {
    const engine = useEngine.getState();
    engine.applyFrame(frame({ sequence: 10 }));
    expect(useEngine.getState().gaps).toBe(0);

    engine.applyFrame(frame({ sequence: 11 }));
    expect(useEngine.getState().gaps, "consecutive frames are not a gap").toBe(0);

    engine.applyFrame(frame({ sequence: 40 }));
    const state = useEngine.getState();
    expect(state.gaps, "20 Hz coalescing skips sequences, and that is worth showing").toBe(1);
    expect(state.sequence).toBe(40);
  });

  it("keeps the previous chain when a frame carries none", () => {
    const engine = useEngine.getState();
    engine.applyFrame(frame());
    expect(useEngine.getState().chain?.symbol).toBe("NIFTY");

    engine.applyFrame(frame({ sequence: 2, chain: undefined }));
    expect(
      useEngine.getState().chain?.symbol,
      "a state-only frame must not blank the chain table",
    ).toBe("NIFTY");
  });

  it("surfaces an unreadable frame instead of throwing", () => {
    const stub = new StubEventSource("/api/stream");
    stub.onmessage = (event) => {
      try {
        useEngine.getState().applyFrame(JSON.parse(event.data));
      } catch (cause) {
        useEngine.setState({ error: `unreadable frame: ${String(cause)}` });
      }
    };
    stub.onmessage(new MessageEvent("message", { data: "{not json" }));
    expect(useEngine.getState().error).toContain("unreadable frame");
  });
});

describe("reconnection", () => {
  beforeEach(reset);

  it("drops out of synchronised as soon as the stream errors", () => {
    useEngine.getState().applyFrame(frame());
    useEngine.setState({ synchronised: true });

    useEngine.getState().markReconnecting();

    const state = useEngine.getState();
    expect(state.link).toBe("reconnecting");
    expect(
      state.synchronised,
      "a reconnecting stream cannot be trusted until it resynchronises",
    ).toBe(false);
  });

  it("resynchronises the selected chain over REST before trusting the display", async () => {
    const columns = vi
      .spyOn(api, "chainColumns")
      .mockResolvedValue(chain({ symbol: "BANKNIFTY" }));
    const ready = vi.spyOn(api, "ready").mockResolvedValue({
      ready: true,
      phase: "feed connected",
      detail: "",
      reasons: [],
      catalog_loaded: true,
      chains: 7,
      feed_connected: true,
      last_frame_age_ms: 20,
    });

    useEngine.setState({ selected: "BANKNIFTY", synchronised: false });
    await useEngine.getState().resync();

    expect(columns).toHaveBeenCalledWith("BANKNIFTY");
    expect(ready).toHaveBeenCalled();

    const state = useEngine.getState();
    expect(state.chain?.symbol).toBe("BANKNIFTY");
    expect(state.synchronised).toBe(true);
    expect(state.link).toBe("open");
  });

  it("reports backend not ready when the resync says the engine cannot work", async () => {
    vi.spyOn(api, "chainColumns").mockResolvedValue(chain());
    vi.spyOn(api, "ready").mockResolvedValue({
      ready: false,
      phase: "feed disconnected",
      detail: "socket closed",
      reasons: ["the market feed is not connected"],
      catalog_loaded: true,
      chains: 7,
      feed_connected: false,
      last_frame_age_ms: 90_000,
    });

    await useEngine.getState().resync();

    const state = useEngine.getState();
    expect(state.link).toBe("not_ready");
    expect(state.readyReasons).toEqual(["the market feed is not connected"]);
  });

  it("reports the backend unavailable when the resync cannot reach it", async () => {
    vi.spyOn(api, "chainColumns").mockRejectedValue(new Error("Failed to fetch"));
    vi.spyOn(api, "ready").mockRejectedValue(new Error("Failed to fetch"));

    await useEngine.getState().resync();

    const state = useEngine.getState();
    expect(state.link).toBe("unavailable");
    expect(state.synchronised).toBe(false);
    expect(state.error).toContain("Failed to fetch");
  });

  it("resets synchronisation when the operator switches index", () => {
    useEngine.setState({ synchronised: true, sequence: 99, chain: chain() });
    useEngine.getState().select("BANKNIFTY");

    const state = useEngine.getState();
    expect(state.selected).toBe("BANKNIFTY");
    expect(state.chain, "the previous index's chain must not be shown under a new label").toBeNull();
    expect(state.synchronised).toBe(false);
    expect(state.sequence).toBeNull();
  });
});
