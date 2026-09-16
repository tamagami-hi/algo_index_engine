import { create } from "zustand";
import type { Resolution, Strategy } from "../types/api";
import { api } from "../api/client";

interface StrategyStore {
  strategies: Strategy[];
  active: string[];
  resolutions: Record<string, Resolution>;
  busy: string | null;
  error: string | null;

  load: () => Promise<void>;
  save: (strategy: Strategy) => Promise<boolean>;
  remove: (id: string) => Promise<void>;
  toggle: (id: string, on: boolean) => Promise<void>;
  resolve: (id: string) => Promise<void>;
}

export const useStrategies = create<StrategyStore>((set, get) => ({
  strategies: [],
  active: [],
  resolutions: {},
  busy: null,
  error: null,

  load: async () => {
    try {
      const [strategies, active] = await Promise.all([
        api.strategies(),
        api.active(),
      ]);
      set({ strategies, active, error: null });
    } catch (cause) {
      set({ error: String(cause) });
    }
  },

  save: async (strategy) => {
    set({ busy: strategy.id, error: null });
    try {
      await api.saveStrategy(strategy.id, strategy);
      await get().load();
      return true;
    } catch (cause) {
      set({ error: String(cause) });
      return false;
    } finally {
      set({ busy: null });
    }
  },

  remove: async (id) => {
    set({ busy: id, error: null });
    try {
      await api.deleteStrategy(id);
      await get().load();
    } catch (cause) {
      set({ error: String(cause) });
    } finally {
      set({ busy: null });
    }
  },

  toggle: async (id, on) => {
    set({ busy: id, error: null });
    try {
      const active = on ? await api.activate(id) : await api.deactivate(id);
      set({ active });
    } catch (cause) {
      set({ error: String(cause) });
    } finally {
      set({ busy: null });
    }
  },

  resolve: async (id) => {
    try {
      const resolution = await api.resolveStrategy(id);
      set({ resolutions: { ...get().resolutions, [id]: resolution } });
    } catch (cause) {
      set({ error: String(cause) });
    }
  },
}));
