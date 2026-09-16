import { useEffect, useState } from "react";
import { isStale, statusLabel, useEngine } from "../stores/engine";

function useNow(ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(timer);
  }, [ms]);
  return now;
}

export function LinkStatus() {
  const link = useEngine((store) => store.link);
  const synchronised = useEngine((store) => store.synchronised);
  const feedAgeMs = useEngine((store) => store.feedAgeMs);
  const feedSilenceLimitMs = useEngine((store) => store.feedSilenceLimitMs);
  const receivedAt = useEngine((store) => store.receivedAt);
  const gaps = useEngine((store) => store.gaps);
  const readyReasons = useEngine((store) => store.readyReasons);
  const now = useNow(500);

  const stale = isStale(
    { link, synchronised, feedAgeMs, feedSilenceLimitMs, receivedAt },
    now,
  );
  const label = statusLabel(link, stale);
  const tone = link !== "open" ? "bad" : stale ? "warn" : "live";

  return (
    <span
      className="link"
      data-testid="link-status"
      data-status={label}
      data-stale={stale ? "true" : "false"}
      title={readyReasons.join("; ")}
    >
      <span className={`dot ${tone}`} />
      {label}
      {gaps > 0 ? (
        <span className="muted" data-testid="gap-count">
          {" "}
          · {gaps} gap{gaps === 1 ? "" : "s"}
        </span>
      ) : null}
    </span>
  );
}

export function StaleBanner() {
  const link = useEngine((store) => store.link);
  const synchronised = useEngine((store) => store.synchronised);
  const feedAgeMs = useEngine((store) => store.feedAgeMs);
  const feedSilenceLimitMs = useEngine((store) => store.feedSilenceLimitMs);
  const receivedAt = useEngine((store) => store.receivedAt);
  const readyReasons = useEngine((store) => store.readyReasons);
  const error = useEngine((store) => store.error);
  const now = useNow(500);

  const stale = isStale(
    { link, synchronised, feedAgeMs, feedSilenceLimitMs, receivedAt },
    now,
  );

  if (!stale && !error) {
    return null;
  }

  return (
    <div className="err" data-testid="stale-banner">
      <strong>{statusLabel(link, stale)}</strong> — the values below are not live and
      must not be read as the current market.
      {readyReasons.length > 0 ? <div>{readyReasons.join("; ")}</div> : null}
      {error ? <div>{error}</div> : null}
    </div>
  );
}
