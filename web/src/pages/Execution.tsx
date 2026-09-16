import { useEffect, useState } from "react";
import { useEngine } from "../stores/engine";
import { api } from "../api/client";
import { useStrategies } from "../stores/strategies";
import { Panel, num } from "../components/ui";
import { StrategyFlow, StrategyNotes } from "../components/StrategyFlow";
import { StrategyEditor } from "../components/StrategyEditor";
import type { Resolution, Strategy } from "../types/api";

function legSummary(strategy: Strategy): string {
  return strategy.legs
    .map((leg) => {
      const side = leg.side === "call" ? "CE" : "PE";
      const strike =
        leg.strike.type === "relative"
          ? leg.strike.moneyness === "atm"
            ? "ATM"
            : `${leg.strike.moneyness.toUpperCase()}${leg.strike.steps}`
          : leg.strike.type;
      const stop = leg.stop_loss
        ? ` SL ${leg.stop_loss.value}${leg.stop_loss.method === "percent" ? "%" : "pt"}`
        : "";
      return `${leg.action === "sell" ? "-" : "+"}${leg.lots} ${strike} ${side}${stop}`;
    })
    .join("   ");
}

function condition(strategy: Strategy): string {
  const gate = strategy.entry_condition;
  if (gate.type === "always") return "unconditional";
  const operator = gate.type === "reference_above" ? ">" : "≤";
  return `${gate.reference} ${operator} ${gate.value}`;
}

function ResolutionView({ resolution }: { resolution: Resolution }) {
  return (
    <div style={{ marginTop: "0.5rem" }}>
      <div className="rowline">
        <span className={resolution.entry_condition_met ? "tag live" : "tag"}>
          gate {resolution.entry_condition_met ? "met" : "not met"}
        </span>
        <span className={resolution.within_trading_window ? "tag live" : "tag"}>
          {resolution.within_trading_window ? "in window" : "out of window"}
        </span>
        <span className={resolution.expiry_gate_met ? "tag live" : "tag"}>
          {resolution.days_to_expiry === null
            ? "expiry unknown"
            : `${resolution.days_to_expiry}DTE`}
        </span>
        <span className={resolution.would_enter_now ? "tag live" : "tag"}>
          {resolution.would_enter_now ? "would enter now" : "would not enter"}
        </span>
        <span className="note">
          spot {num(resolution.spot_price)} · atm {num(resolution.spot_atm, 0)} · lot{" "}
          {resolution.lot_size} · {resolution.minutes_until_exit} min to exit
        </span>
      </div>

      {resolution.legs.length > 0 ? (
        <table style={{ marginTop: "0.3rem" }}>
          <thead>
            <tr>
              <th style={{ textAlign: "left" }}>leg</th>
              <th>strike</th>
              <th>qty</th>
              <th>ltp</th>
              <th>bid</th>
              <th>ask</th>
              <th>stop</th>
              <th>target</th>
              <th style={{ textAlign: "left" }}>security</th>
            </tr>
          </thead>
          <tbody>
            {resolution.legs.map((leg) => (
              <tr key={leg.leg}>
                <td style={{ textAlign: "left" }}>
                  <span className={leg.action === "sell" ? "tag sell" : "tag buy"}>
                    {leg.action}
                  </span>{" "}
                  {leg.side === "call" ? "CE" : "PE"}
                </td>
                <td>{num(leg.strike, 0)}</td>
                <td>{leg.quantity}</td>
                <td>{num(leg.ltp)}</td>
                <td>{num(leg.bid)}</td>
                <td>{num(leg.ask)}</td>
                <td className="down">{num(leg.stop_price)}</td>
                <td className="up">{num(leg.target_price)}</td>
                <td style={{ textAlign: "left" }} className="muted">
                  {leg.security_id}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}

      {resolution.sizing ? (
        <div className="err">
          {resolution.sizing.underlying}: {resolution.sizing.problem} — cannot size an
          order, entry is blocked
        </div>
      ) : null}

      {resolution.problems.length > 0 ? (
        <div className="err">
          {resolution.problems.map((problem) => (
            <div key={`${problem.leg}-${problem.reason}`}>
              leg {problem.leg} ({problem.side}): {problem.reason}
              {problem.wanted ? ` — wanted ${problem.wanted}` : ""}
              {problem.strike ? ` — at ${problem.strike}` : ""}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function Execution() {
  const strategies = useStrategies((store) => store.strategies);
  const active = useStrategies((store) => store.active);
  const resolutions = useStrategies((store) => store.resolutions);
  const busy = useStrategies((store) => store.busy);
  const error = useStrategies((store) => store.error);
  const load = useStrategies((store) => store.load);
  const toggle = useStrategies((store) => store.toggle);
  const remove = useStrategies((store) => store.remove);
  const resolve = useStrategies((store) => store.resolve);

  const symbols = useEngine((store) => store.symbols);
  const [editing, setEditing] = useState<Strategy | null>(null);

  useEffect(() => {
    void load();
  }, [load]);

  // Keep the entry preview fresh while the market moves.
  useEffect(() => {
    if (active.length === 0) {
      return;
    }
    const refresh = () => active.forEach((id) => void resolve(id));
    refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => window.clearInterval(timer);
  }, [active, resolve]);

  return (
    <>
      <Panel title="strategy">
        <details>
          <summary>description and flow</summary>
          <div style={{ paddingTop: "0.5rem" }}>
            <StrategyNotes />
            <StrategyFlow />
          </div>
        </details>
      </Panel>

      {error ? <div className="err">{error}</div> : null}

      <Panel title="paper trading">
        <div className="rowline">
          <span className="note">
            {active.length} of {strategies.length} saved strategies armed. Several may run at
            once, across different indices.
          </span>
          <span className="spacer" />
          <button onClick={() => void load()}>refresh</button>
          <button
            onClick={() => {
              setEditing(null);
              void api
                .strategyTemplate(symbols[0] ?? "NIFTY")
                .then(setEditing);
            }}
          >
            new strategy
          </button>
        </div>

        {strategies.length === 0 ? (
          <span className="muted">
            nothing saved yet — start from a template with “new strategy”.
          </span>
        ) : (
          strategies.map((strategy) => {
            const on = active.includes(strategy.id);
            const resolution = resolutions[strategy.id];
            return (
              <div
                key={strategy.id}
                className={on ? "strategy armed" : "strategy"}
              >
                <div className="rowline">
                  <h3 style={{ margin: 0 }}>{strategy.name}</h3>
                  <span className="tag">{strategy.underlying}</span>
                  <span className="tag">{condition(strategy)}</span>
                  <span className="tag">{strategy.loss_coverage}</span>
                  <span className="tag">
                    {strategy.entry_time} → {strategy.exit_time}
                  </span>
                  <span className="spacer" />
                  <button
                    className={on ? "on" : undefined}
                    disabled={busy === strategy.id}
                    onClick={() => void toggle(strategy.id, !on)}
                  >
                    {on ? "deactivate" : "activate"}
                  </button>
                  <button onClick={() => setEditing(strategy)}>edit</button>
                  <button
                    className="danger"
                    disabled={busy === strategy.id}
                    onClick={() => void remove(strategy.id)}
                  >
                    delete
                  </button>
                </div>
                <div className="note">{legSummary(strategy)}</div>
                {strategy.notes ? <div className="note">{strategy.notes}</div> : null}
                {on && resolution ? <ResolutionView resolution={resolution} /> : null}
              </div>
            );
          })
        )}
      </Panel>

      <Panel title="live trading">
        <span className="muted">
          Disabled. Live order routing is deliberately not wired up — paper results come
          first.
        </span>
      </Panel>

      {editing ? (
        <StrategyEditor
          initial={editing}
          symbols={symbols}
          onClose={() => setEditing(null)}
        />
      ) : null}
    </>
  );
}
