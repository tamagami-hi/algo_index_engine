import { useEffect, useState } from "react";
import { useEngine } from "./stores/engine";
import { Telemetry } from "./pages/Telemetry";
import { Chains } from "./pages/Chains";
import { Execution } from "./pages/Execution";

type Route = "telemetry" | "chains" | "execution";

const ROUTES: { id: Route; label: string }[] = [
  { id: "telemetry", label: "telemetry" },
  { id: "chains", label: "option chains" },
  { id: "execution", label: "execution" },
];

function currentRoute(): Route {
  const hash = window.location.hash.replace(/^#\/?/, "");
  return ROUTES.some((route) => route.id === hash) ? (hash as Route) : "telemetry";
}

export function App() {
  const [route, setRoute] = useState<Route>(currentRoute);
  const connect = useEngine((store) => store.connect);
  const link = useEngine((store) => store.link);
  const snapshot = useEngine((store) => store.snapshot);
  const selected = useEngine((store) => store.selected);

  useEffect(() => {
    connect();
  }, [connect]);

  useEffect(() => {
    const onHash = () => setRoute(currentRoute());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  const phase = snapshot?.phase;
  const dotClass =
    link !== "open"
      ? "dot bad"
      : phase === "feed_connected"
        ? "dot live"
        : phase && phase.endsWith("failed")
          ? "dot bad"
          : "dot warn";

  return (
    <div className="shell">
      <header className="topbar">
        <span className="brand">algo index engine</span>
        <nav>
          {ROUTES.map((item) => (
            <a
              key={item.id}
              href={`#/${item.id}`}
              aria-current={route === item.id ? "page" : undefined}
            >
              {item.label}
            </a>
          ))}
        </nav>
        <span className="spacer" />
        <span className="link">
          <span className={dotClass} />
          {link === "open" ? (snapshot?.phase_label ?? "connected") : link}
        </span>
        <span className="link">{selected}</span>
        {snapshot ? <span className="link">v{snapshot.version}</span> : null}
      </header>

      <main>
        {route === "telemetry" ? <Telemetry /> : null}
        {route === "chains" ? <Chains /> : null}
        {route === "execution" ? <Execution /> : null}
      </main>
    </div>
  );
}
