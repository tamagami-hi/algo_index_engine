function Node({
  kind,
  title,
  lines,
}: {
  kind: "start" | "gate" | "order" | "exit";
  title: string;
  lines: string[];
}) {
  return (
    <div className={`node ${kind}`}>
      <div className="title">{title}</div>
      {lines.map((line) => (
        <div className="line" key={line}>
          {line}
        </div>
      ))}
    </div>
  );
}

const Stem = () => <div className="stem" />;

/**
 * The reference flowchart, drawn rather than embedded, so the rules stay
 * readable and can be kept in step with the engine.
 */
export function StrategyFlow({
  vixThreshold = 12,
  entry = "09:16",
  exit = "14:59",
  highVixStop = 75,
  lowVixStop = 50,
  itmSteps = 3,
}: {
  vixThreshold?: number;
  entry?: string;
  exit?: string;
  highVixStop?: number;
  lowVixStop?: number;
  itmSteps?: number;
}) {
  return (
    <div className="flow">
      <Node kind="start" title="start" lines={["0DTE – 1DTE", `at ${entry} IST`]} />
      <Stem />
      <Node kind="gate" title="if" lines={[`NSE:INDIA VIX > ${vixThreshold}`]} />
      <Stem />

      <div className="branches">
        <div className="branch t">
          <div className="label">true</div>
          <Node
            kind="order"
            title="sell"
            lines={["ATM CE · current expiry", "1 lot", `stop-loss ${highVixStop}%`]}
          />
          <Stem />
          <Node
            kind="order"
            title="sell"
            lines={["ATM PE · current expiry", "1 lot", `stop-loss ${highVixStop}%`]}
          />
        </div>

        <div className="branch f">
          <div className="label">false</div>
          <Node
            kind="order"
            title="sell"
            lines={[
              `ITM${itmSteps} CE · current expiry`,
              "1 lot",
              `stop-loss ${lowVixStop}%`,
            ]}
          />
          <Stem />
          <Node
            kind="order"
            title="sell"
            lines={[
              `ITM${itmSteps} PE · current expiry`,
              "1 lot",
              `stop-loss ${lowVixStop}%`,
            ]}
          />
        </div>
      </div>

      <Stem />
      <Node kind="start" title="wait" lines={[`hold until ${exit}`]} />
      <Stem />
      <Node kind="exit" title="square off" lines={["exit all open legs", "profit or loss"]} />
    </div>
  );
}

export function StrategyNotes() {
  return (
    <div className="note" style={{ display: "grid", gap: "0.4rem" }}>
      <p style={{ margin: 0 }}>
        A short-premium intraday strategy on the current expiry. Both branches sell,
        so the credit is taken at entry and the risk is a rise in premium.
      </p>
      <p style={{ margin: 0 }}>
        India VIX picks the branch. Above the threshold the ATM straddle is sold, both
        legs at the same strike. At or below it, the ITM3 pair is sold — the call three
        strikes below the money and the put three above — which puts the call strike
        beneath the put strike, a short guts. Its stop is tighter because ITM legs carry
        intrinsic value and track spot closely.
      </p>
      <p style={{ margin: 0 }}>
        Each stop is per leg, measured against that leg&apos;s own entry fill. A leg
        hitting its stop exits alone; the surviving leg keeps running, protected by
        whichever loss-coverage rule the strategy sets.
      </p>
      <p style={{ margin: 0 }}>
        The exit time is a hard gate: it is evaluated before every other rule and
        cannot be skipped, so nothing is carried overnight.
      </p>
    </div>
  );
}
