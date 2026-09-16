import { useState } from "react";
import { useStrategies } from "../stores/strategies";
import { DTE_CHOICES } from "../types/api";
import type {
  EntryCondition,
  LegDefinition,
  LossCoverage,
  Moneyness,
  Strategy,
  Threshold,
} from "../types/api";

const slug = (text: string): string =>
  text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="field">
      <label>{label}</label>
      {children}
    </div>
  );
}

function ThresholdInput({
  value,
  onChange,
}: {
  value: Threshold | undefined;
  onChange: (next: Threshold | undefined) => void;
}) {
  return (
    <div style={{ display: "flex", gap: "0.3rem" }}>
      <input
        type="number"
        step="0.5"
        style={{ width: "5rem" }}
        value={value?.value ?? ""}
        placeholder="off"
        onChange={(event) => {
          const raw = event.target.value;
          if (raw === "") {
            onChange(undefined);
            return;
          }
          onChange({ method: value?.method ?? "percent", value: Number(raw) });
        }}
      />
      <select
        value={value?.method ?? "percent"}
        disabled={!value}
        onChange={(event) =>
          value
            ? onChange({
                method: event.target.value as Threshold["method"],
                value: value.value,
              })
            : undefined
        }
      >
        <option value="percent">%</option>
        <option value="points">pts</option>
      </select>
    </div>
  );
}

function LegRow({
  leg,
  index,
  onChange,
  onRemove,
}: {
  leg: LegDefinition;
  index: number;
  onChange: (next: LegDefinition) => void;
  onRemove: () => void;
}) {
  const relative = leg.strike.type === "relative" ? leg.strike : null;

  return (
    <fieldset>
      <legend>
        leg {index + 1} · {leg.side === "call" ? "CE" : "PE"}
      </legend>
      <div className="fields">
        <Field label="side">
          <select
            value={leg.side}
            onChange={(event) =>
              onChange({ ...leg, side: event.target.value as LegDefinition["side"] })
            }
          >
            <option value="call">call (CE)</option>
            <option value="put">put (PE)</option>
          </select>
        </Field>
        <Field label="action">
          <select
            value={leg.action}
            onChange={(event) =>
              onChange({ ...leg, action: event.target.value as LegDefinition["action"] })
            }
          >
            <option value="sell">sell</option>
            <option value="buy">buy</option>
          </select>
        </Field>
        <Field label="lots">
          <input
            type="number"
            min="1"
            value={leg.lots}
            onChange={(event) => onChange({ ...leg, lots: Number(event.target.value) })}
          />
        </Field>
        <Field label="moneyness">
          <select
            value={relative?.moneyness ?? "atm"}
            onChange={(event) =>
              onChange({
                ...leg,
                strike: {
                  type: "relative",
                  moneyness: event.target.value as Moneyness,
                  steps: relative?.steps ?? 0,
                },
              })
            }
          >
            <option value="atm">ATM</option>
            <option value="itm">ITM</option>
            <option value="otm">OTM</option>
          </select>
        </Field>
        <Field label="steps">
          <input
            type="number"
            min="0"
            disabled={(relative?.moneyness ?? "atm") === "atm"}
            value={relative?.steps ?? 0}
            onChange={(event) =>
              onChange({
                ...leg,
                strike: {
                  type: "relative",
                  moneyness: relative?.moneyness ?? "atm",
                  steps: Number(event.target.value),
                },
              })
            }
          />
        </Field>
        <Field label="stop loss">
          <ThresholdInput
            value={leg.stop_loss}
            onChange={(next) => onChange({ ...leg, stop_loss: next })}
          />
        </Field>
        <Field label="target">
          <ThresholdInput
            value={leg.target}
            onChange={(next) => onChange({ ...leg, target: next })}
          />
        </Field>
        <Field label="trail arm at">
          <ThresholdInput
            value={leg.trailing?.arm_at}
            onChange={(next) =>
              onChange({
                ...leg,
                trailing: next
                  ? {
                      arm_at: next,
                      give_back: leg.trailing?.give_back ?? { method: "percent", value: 10 },
                      ...(leg.trailing?.start_after_minutes !== undefined
                        ? { start_after_minutes: leg.trailing.start_after_minutes }
                        : {}),
                    }
                  : undefined,
              })
            }
          />
        </Field>
        <Field label="trail give back">
          <ThresholdInput
            value={leg.trailing?.give_back}
            onChange={(next) =>
              leg.trailing && next
                ? onChange({ ...leg, trailing: { ...leg.trailing, give_back: next } })
                : undefined
            }
          />
        </Field>
      </div>
      <div className="rowline">
        <span className="spacer" />
        <button className="danger" onClick={onRemove}>
          remove leg
        </button>
      </div>
    </fieldset>
  );
}

export function StrategyEditor({
  initial,
  symbols,
  onClose,
}: {
  initial: Strategy;
  symbols: string[];
  onClose: () => void;
}) {
  const save = useStrategies((store) => store.save);
  const [draft, setDraft] = useState<Strategy>(initial);
  const [problem, setProblem] = useState<string | null>(null);

  const gate = draft.entry_condition;
  const gateValue = gate.type === "always" ? "" : String(gate.value);
  const gateReference = gate.type === "always" ? "INDIA VIX" : gate.reference;
  const allDte = DTE_CHOICES.every((day) => draft.dte.includes(day));

  const setGate = (next: EntryCondition) => setDraft({ ...draft, entry_condition: next });

  const commit = async () => {
    const id = draft.id.trim() === "" ? slug(draft.name) : slug(draft.id);
    if (id === "") {
      setProblem("give the strategy a name first");
      return;
    }
    const ok = await save({ ...draft, id });
    if (ok) {
      onClose();
    } else {
      setProblem("the engine rejected this strategy — see the error above");
    }
  };

  return (
    <section className="panel">
      <h2>
        {initial.id ? `edit · ${initial.id}` : "new strategy"}
        <span style={{ float: "right" }}>
          <button onClick={onClose}>cancel</button>
        </span>
      </h2>
      <div className="body">
        {problem ? <div className="err">{problem}</div> : null}

        <fieldset>
          <legend>identity</legend>
          <div className="fields">
            <Field label="name">
              <input
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              />
            </Field>
            <Field label="id (slug)">
              <input
                value={draft.id}
                placeholder={slug(draft.name) || "auto"}
                onChange={(event) => setDraft({ ...draft, id: event.target.value })}
              />
            </Field>
            <Field label="index">
              <select
                value={draft.underlying}
                onChange={(event) => setDraft({ ...draft, underlying: event.target.value })}
              >
                {(symbols.length > 0 ? symbols : [draft.underlying]).map((symbol) => (
                  <option key={symbol} value={symbol}>
                    {symbol}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="entry (IST)">
              <input
                value={draft.entry_time}
                onChange={(event) => setDraft({ ...draft, entry_time: event.target.value })}
              />
            </Field>
            <Field label="hard exit (IST)">
              <input
                value={draft.exit_time}
                onChange={(event) => setDraft({ ...draft, exit_time: event.target.value })}
              />
            </Field>
          </div>
        </fieldset>

        <fieldset>
          <legend>days to expiry</legend>
          <span className="note">
            Tick the DTEs this strategy may enter on. On any other day it stays idle even
            when armed.
          </span>
          <div className="rowline" style={{ marginTop: "0.4rem", flexWrap: "wrap" }}>
            <label className="tag" style={{ cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={allDte}
                style={{ marginRight: "0.3rem" }}
                onChange={() =>
                  setDraft({ ...draft, dte: allDte ? [] : [...DTE_CHOICES] })
                }
              />
              all DTE
            </label>
            {DTE_CHOICES.map((day) => {
              const chosen = draft.dte.includes(day);
              return (
                <label
                  key={day}
                  className={chosen ? "tag live" : "tag"}
                  style={{ cursor: "pointer" }}
                >
                  <input
                    type="checkbox"
                    checked={chosen}
                    style={{ marginRight: "0.3rem" }}
                    onChange={() =>
                      setDraft({
                        ...draft,
                        dte: chosen
                          ? draft.dte.filter((value) => value !== day)
                          : [...draft.dte, day].sort((a, b) => a - b),
                      })
                    }
                  />
                  {day} DTE
                </label>
              );
            })}
          </div>
          {draft.dte.length === 0 ? (
            <div className="err">select at least one DTE or the strategy can never run</div>
          ) : null}
        </fieldset>

        <fieldset>
          <legend>entry gate</legend>
          <div className="fields">
            <Field label="condition">
              <select
                value={gate.type}
                onChange={(event) => {
                  const type = event.target.value as EntryCondition["type"];
                  if (type === "always") {
                    setGate({ type: "always" });
                  } else {
                    setGate({
                      type,
                      reference: gateReference,
                      value: gateValue === "" ? 12 : Number(gateValue),
                    });
                  }
                }}
              >
                <option value="always">always</option>
                <option value="reference_above">reference above</option>
                <option value="reference_at_most">reference at most</option>
              </select>
            </Field>
            <Field label="reference">
              <input
                disabled={gate.type === "always"}
                value={gateReference}
                onChange={(event) =>
                  gate.type !== "always"
                    ? setGate({ ...gate, reference: event.target.value })
                    : undefined
                }
              />
            </Field>
            <Field label="value">
              <input
                type="number"
                step="0.5"
                disabled={gate.type === "always"}
                value={gateValue}
                onChange={(event) =>
                  gate.type !== "always"
                    ? setGate({ ...gate, value: Number(event.target.value) })
                    : undefined
                }
              />
            </Field>
            <Field label="loss coverage">
              <select
                value={draft.loss_coverage}
                onChange={(event) =>
                  setDraft({ ...draft, loss_coverage: event.target.value as LossCoverage })
                }
              >
                <option value="none">none</option>
                <option value="breakeven">breakeven</option>
                <option value="recover_stopped_leg_loss">recover stopped leg loss</option>
              </select>
            </Field>
          </div>
        </fieldset>

        {draft.legs.map((leg, index) => (
          <LegRow
            key={index}
            leg={leg}
            index={index}
            onChange={(next) =>
              setDraft({
                ...draft,
                legs: draft.legs.map((current, at) => (at === index ? next : current)),
              })
            }
            onRemove={() =>
              setDraft({
                ...draft,
                legs: draft.legs.filter((_, at) => at !== index),
              })
            }
          />
        ))}

        <fieldset>
          <legend>overall risk</legend>
          <div className="fields">
            <Field label="stop loss">
              <ThresholdInput
                value={draft.overall.stop_loss}
                onChange={(next) =>
                  setDraft({ ...draft, overall: { ...draft.overall, stop_loss: next } })
                }
              />
            </Field>
            <Field label="target">
              <ThresholdInput
                value={draft.overall.target}
                onChange={(next) =>
                  setDraft({ ...draft, overall: { ...draft.overall, target: next } })
                }
              />
            </Field>
            <Field label="daily loss limit">
              <input
                type="number"
                value={draft.overall.daily_loss_limit ?? ""}
                placeholder="off"
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    overall: {
                      ...draft.overall,
                      daily_loss_limit:
                        event.target.value === "" ? undefined : Number(event.target.value),
                    },
                  })
                }
              />
            </Field>
            <Field label="daily profit target">
              <input
                type="number"
                value={draft.overall.daily_profit_target ?? ""}
                placeholder="off"
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    overall: {
                      ...draft.overall,
                      daily_profit_target:
                        event.target.value === "" ? undefined : Number(event.target.value),
                    },
                  })
                }
              />
            </Field>
          </div>
        </fieldset>

        <div className="rowline">
          <button
            onClick={() =>
              setDraft({
                ...draft,
                legs: [
                  ...draft.legs,
                  {
                    side: "call",
                    action: "sell",
                    lots: 1,
                    strike: { type: "relative", moneyness: "atm", steps: 0 },
                    stop_loss: { method: "percent", value: 75 },
                  },
                ],
              })
            }
          >
            add leg
          </button>
          <span className="spacer" />
          <button className="on" onClick={() => void commit()}>
            save strategy
          </button>
        </div>
      </div>
    </section>
  );
}
