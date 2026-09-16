import type {
  ChainColumns,
  ChainMetrics,
  Listing,
  Resolution,
  Snapshot,
  Strategy,
} from "../types/api";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    headers: init?.body ? { "content-type": "application/json" } : undefined,
    ...init,
  });
  if (!response.ok) {
    let detail = `${response.status} ${response.statusText}`;
    try {
      const body = await response.json();
      if (body && typeof body === "object") {
        detail = JSON.stringify(body);
      }
    } catch {
    }
    throw new Error(detail);
  }
  return (await response.json()) as T;
}

export const api = {
  state: () => request<Snapshot>("/api/state"),
  symbols: () => request<string[]>("/api/symbols"),
  chains: () => request<ChainMetrics[]>("/api/chains"),
  chainColumns: (symbol: string) =>
    request<ChainColumns>(`/api/chain/${encodeURIComponent(symbol)}/columns`),

  strategies: () => request<Listing>("/api/strategies"),
  strategyTemplate: (underlying: string) =>
    request<Strategy>(
      `/api/strategies/template?chain=${encodeURIComponent(underlying)}`,
    ),
  saveStrategy: (id: string, strategy: Strategy) =>
    request<Strategy>(`/api/strategies/${encodeURIComponent(id)}`, {
      method: "PUT",
      body: JSON.stringify(strategy),
    }),
  deleteStrategy: (id: string) =>
    request<{ removed: boolean }>(`/api/strategies/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  resolveStrategy: (id: string) =>
    request<Resolution>(`/api/strategies/${encodeURIComponent(id)}/resolve`),

  active: () => request<string[]>("/api/active"),
  activate: (id: string) =>
    request<string[]>(`/api/strategies/${encodeURIComponent(id)}/activate`, {
      method: "POST",
    }),
  deactivate: (id: string) =>
    request<string[]>(`/api/strategies/${encodeURIComponent(id)}/deactivate`, {
      method: "POST",
    }),
};
