import { create } from "zustand";
import type { ChainColumns, Snapshot, StreamFrame } from "../types/api";
import { api } from "../api/client";

export type Link =
  | "connecting"
  | "open"
  | "reconnecting"
  | "unavailable"
  | "not_ready";

export const STREAM_SILENCE_MS = 3_000;

export interface EngineStore {
  snapshot: Snapshot | null;
  chain: ChainColumns | null;
  selected: string;
  symbols: string[];
  link: Link;
  synchronised: boolean;
  sequence: number | null;
  gaps: number;
  feedAgeMs: number | null;
  feedSilenceLimitMs: number | null;
  publishIntervalMs: number | null;
  receivedAt: number | null;
  readyReasons: string[];
  error: string | null;

  connect: () => void;
  select: (symbol: string) => void;
  applyFrame: (frame: StreamFrame) => void;
  markReconnecting: () => void;
  resync: () => Promise<void>;
}

let source: EventSource | null = null;

export function closeStream() {
  source?.close();
  source = null;
}

export const isStale = (
  store: Pick<
    EngineStore,
    "link" | "synchronised" | "feedAgeMs" | "feedSilenceLimitMs" | "receivedAt"
  >,
  now: number,
): boolean => {
  if (store.link !== "open" || !store.synchronised) {
    return true;
  }
  if (store.receivedAt !== null && now - store.receivedAt > STREAM_SILENCE_MS) {
    return true;
  }
  if (store.feedAgeMs !== null && store.feedSilenceLimitMs !== null) {
    return store.feedAgeMs > store.feedSilenceLimitMs;
  }
  return false;
};

export const statusLabel = (link: Link, stale: boolean): string => {
  if (link === "unavailable") return "backend unavailable";
  if (link === "not_ready") return "backend not ready";
  if (link === "connecting") return "connecting";
  if (link === "reconnecting") return "reconnecting";
  return stale ? "data stale" : "live";
};

export const useEngine = create<EngineStore>((set, get) => ({
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

  applyFrame: (frame) => {
    const previous = get().sequence;
    const jumped =
      previous !== null && frame.sequence > previous + 1 ? get().gaps + 1 : get().gaps;

    set({
      snapshot: frame.state,
      chain: frame.chain ?? get().chain,
      sequence: frame.sequence,
      gaps: jumped,
      feedAgeMs: frame.feed_age_ms ?? null,
      feedSilenceLimitMs: frame.feed_silence_limit_ms,
      publishIntervalMs: frame.publish_interval_ms,
      receivedAt: Date.now(),
      link: "open",
      error: null,
    });
  },

  markReconnecting: () => {
    set({ link: "reconnecting", synchronised: false });
  },

  resync: async () => {
    const symbol = get().selected;
    try {
      const [chain, ready] = await Promise.all([
        symbol ? api.chainColumns(symbol) : Promise.resolve(null),
        api.ready(),
      ]);
      set({
        chain: chain ?? get().chain,
        readyReasons: ready.reasons,
        synchronised: true,
        link: ready.ready ? "open" : "not_ready",
      });
    } catch (cause) {
      set({ error: String(cause), link: "unavailable", synchronised: false });
    }
  },

  connect: () => {
    void api
      .symbols()
      .then((symbols) => {
        const selected = get().selected;
        const initial =
          symbols.length === 0
            ? selected
            : symbols.includes(selected)
              ? selected
              : (symbols[0] as string);
        set({ symbols, selected: initial });
        openStream(initial, set, get);
      })
      .catch((cause: unknown) => {
        set({ error: String(cause), link: "unavailable", synchronised: false });
        openStream(get().selected, set, get);
      });
  },

  select: (symbol) => {
    if (symbol === get().selected) {
      return;
    }
    set({ selected: symbol, chain: null, synchronised: false, sequence: null });
    openStream(symbol, set, get);
  },
}));

function openStream(
  symbol: string,
  set: (partial: Partial<EngineStore>) => void,
  get: () => EngineStore,
) {
  closeStream();
  const query = symbol ? `?chain=${encodeURIComponent(symbol)}` : "";
  const stream = new EventSource(`/api/stream${query}`);
  source = stream;

  stream.onopen = () => {
    set({ link: "open", error: null });
    void get().resync();
  };

  stream.onmessage = (event) => {
    try {
      get().applyFrame(JSON.parse(event.data) as StreamFrame);
    } catch (cause) {
      set({ error: `unreadable frame: ${String(cause)}` });
    }
  };

  stream.onerror = () => {
    get().markReconnecting();
  };
}
