import type { ReactNode } from "react";

export function Panel({
  title,
  children,
  right,
}: {
  title: string;
  children: ReactNode;
  right?: ReactNode;
}) {
  return (
    <section className="panel">
      <h2>
        {title}
        {right ? <span style={{ float: "right" }}>{right}</span> : null}
      </h2>
      <div className="body">{children}</div>
    </section>
  );
}

export function Stat({ k, v, tone }: { k: string; v: ReactNode; tone?: string }) {
  return (
    <div className="stat">
      <span className="k">{k}</span>
      <span className={tone ? `v ${tone}` : "v"}>{v}</span>
    </div>
  );
}

export function Big({ k, v, tone }: { k: string; v: ReactNode; tone?: string }) {
  return (
    <div className="cell">
      <div className="k">{k}</div>
      <div className={tone ? `v ${tone}` : "v"}>{v}</div>
    </div>
  );
}

export const num = (value: number | null | undefined, digits = 2): string =>
  value === null || value === undefined || !Number.isFinite(value)
    ? "—"
    : value.toLocaleString("en-IN", {
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
      });

export const int = (value: number | null | undefined): string =>
  value === null || value === undefined || !Number.isFinite(value)
    ? "—"
    : Math.round(value).toLocaleString("en-IN");

export function compact(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "—";
  }
  const abs = Math.abs(value);
  if (abs >= 1e7) return `${(value / 1e7).toFixed(2)}Cr`;
  if (abs >= 1e5) return `${(value / 1e5).toFixed(2)}L`;
  if (abs >= 1e3) return `${(value / 1e3).toFixed(1)}k`;
  return int(value);
}

export function bytes(value: number): string {
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(2)} GiB`;
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(2)} MiB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${value} B`;
}

export function duration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = Math.floor(seconds % 60);
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m ${rest}s`;
  if (minutes > 0) return `${minutes}m ${rest}s`;
  return `${rest}s`;
}
