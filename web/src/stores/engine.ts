import { create } from "zustand";
import type { ChainColumns, Snapshot, StreamFrame } from "../types/api";
import { api } from "../api/client";

export type Link = "connecting" | "open" | "reconnecting";

interface EngineStore {
  snapshot: Snapshot | null;
  chain: ChainColumns | null;
  selected: string;
  symbols: string[];
  link: Link;
  receivedAt: number | null;
  error: string | null;

  connect: () => void;
  select: (symbol: string) => void;
}

let source: EventSource | null = null;

function open(symbol: string, set: (partial: Partial<EngineStore>) => void) {
  source?.close();
  const query = symbol ? `?chain=${encodeURIComponent(symbol)}` : "";
  const stream = new EventSource(`/api/stream${query}`);
  source = stream;

  stream.onopen = () => set({ link: "open", error: null });

  stream.onmessage = (event) => {
    try {
      const frame = JSON.parse(event.data) as StreamFrame;
      set({
        snapshot: frame.state,
        chain: frame.chain ?? null,
        link: "open",
        receivedAt: Date.now(),
      });
    } catch (cause) {
      set({ error: `unreadable frame: ${String(cause)}` });
    }
  };

  stream.onerror = () => set({ link: "reconnecting" });
}

export const useEngine = create<EngineStore>((set, get) => ({
  snapshot: null,
  chain: null,
  selected: "NIFTY",
  symbols: [],
  link: "connecting",
  receivedAt: null,
  error: null,

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
        open(initial, set);
      })
      .catch((cause: unknown) => {
        set({ error: String(cause), link: "reconnecting" });
        open(get().selected, set);
      });
  },

  select: (symbol) => {
    if (symbol === get().selected) {
      return;
    }
    set({ selected: symbol, chain: null });
    open(symbol, set);
  },
}));
