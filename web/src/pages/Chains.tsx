import { useEngine } from "../stores/engine";
import { Big, Panel, compact, num } from "../components/ui";
import type { SideColumns } from "../types/api";

function cell(side: SideColumns, row: number, field: keyof SideColumns): number {
  const column = side[field] as number[];
  return column[row] ?? 0;
}

export function Chains() {
  const chain = useEngine((store) => store.chain);
  const symbols = useEngine((store) => store.symbols);
  const selected = useEngine((store) => store.selected);
  const select = useEngine((store) => store.select);

  const picker = (
    <select value={selected} onChange={(event) => select(event.target.value)}>
      {(symbols.length > 0 ? symbols : [selected]).map((symbol) => (
        <option key={symbol} value={symbol}>
          {symbol}
        </option>
      ))}
    </select>
  );

  if (!chain) {
    return (
      <Panel title="option chain" right={picker}>
        <span className="muted">waiting for {selected} to arrive on the stream…</span>
      </Panel>
    );
  }

  const atmRow = chain.market_atm_row;

  return (
    <>
      <Panel title={`${chain.symbol} · ${chain.expiry}`} right={picker}>
        <div className="big">
          <Big k="spot" v={num(chain.spot_price)} />
          <Big k="spot atm" v={num(chain.spot_atm, 0)} />
          <Big k="market atm" v={num(chain.market_atm, 0)} tone="up" />
          <Big k="max pain" v={num(chain.max_pain, 0)} tone="warnfg" />
          <Big k="atm straddle" v={num(chain.atm_straddle)} />
          <Big k="synthetic fut" v={num(chain.synthetic_future)} />
          <Big k="pcr oi" v={num(chain.pcr_oi)} />
        </div>
        <div className="grid" style={{ marginTop: "0.5rem" }}>
          <div className="stat">
            <span className="k">lot size</span>
            <span className="v">{chain.lot_size}</span>
          </div>
          <div className="stat">
            <span className="k">strike step</span>
            <span className="v">{num(chain.strike_step, 0)}</span>
          </div>
          <div className="stat">
            <span className="k">strikes quoted</span>
            <span className="v">
              {compact(chain.quoted_strikes)} / {compact(chain.strikes)}
            </span>
          </div>
          <div className="stat">
            <span className="k">call oi</span>
            <span className="v">{compact(chain.total_call_oi)}</span>
          </div>
          <div className="stat">
            <span className="k">put oi</span>
            <span className="v">{compact(chain.total_put_oi)}</span>
          </div>
          <div className="stat">
            <span className="k">atm imbalance</span>
            <span
              className={
                chain.atm_imbalance === null
                  ? "v muted"
                  : chain.atm_imbalance > 0
                    ? "v up"
                    : "v down"
              }
            >
              {chain.atm_imbalance === null
                ? "—"
                : `${(chain.atm_imbalance * 100).toFixed(1)}%`}
            </span>
          </div>
        </div>
      </Panel>

      <section className="panel">
        <h2>
          calls · strike · puts
          <span style={{ float: "right" }} className="note">
            {chain.strikes} strikes · ATM highlighted
          </span>
        </h2>
        <div className="chainwrap">
          <table>
            <thead>
              <tr>
                <th className="callside">oi</th>
                <th className="callside">chg oi</th>
                <th className="callside">vol</th>
                <th className="callside">bid</th>
                <th className="callside">ask</th>
                <th className="callside">ltp</th>
                <th className="callside">chg</th>
                <th style={{ textAlign: "center" }}>strike</th>
                <th className="putside">chg</th>
                <th className="putside">ltp</th>
                <th className="putside">bid</th>
                <th className="putside">ask</th>
                <th className="putside">vol</th>
                <th className="putside">chg oi</th>
                <th className="putside">oi</th>
              </tr>
            </thead>
            <tbody>
              {chain.strike.map((strike, row) => {
                const isAtm = row === atmRow;
                // A call is in the money below the ATM strike, a put above it.
                const callItm = atmRow !== null && row < atmRow ? "itm" : undefined;
                const putItm = atmRow !== null && row > atmRow ? "itm" : undefined;
                const callQuoted = chain.call.quoted[row] ?? false;
                const putQuoted = chain.put.quoted[row] ?? false;
                const callChange = cell(chain.call, row, "change");
                const putChange = cell(chain.put, row, "change");

                return (
                  <tr key={strike} className={isAtm ? "atmrow" : undefined}>
                    <td className={callItm}>{compact(cell(chain.call, row, "oi"))}</td>
                    <td className={callItm}>{compact(cell(chain.call, row, "change_in_oi"))}</td>
                    <td className={callItm}>{compact(cell(chain.call, row, "volume"))}</td>
                    <td className={callItm}>{num(cell(chain.call, row, "bid"))}</td>
                    <td className={callItm}>{num(cell(chain.call, row, "ask"))}</td>
                    <td className={callQuoted ? callItm : "stale"}>
                      {num(cell(chain.call, row, "ltp"))}
                    </td>
                    <td className={callChange > 0 ? "up" : callChange < 0 ? "down" : "muted"}>
                      {num(callChange)}
                    </td>
                    <td className="strike">{num(strike, 0)}</td>
                    <td className={putChange > 0 ? "up" : putChange < 0 ? "down" : "muted"}>
                      {num(putChange)}
                    </td>
                    <td className={putQuoted ? putItm : "stale"}>
                      {num(cell(chain.put, row, "ltp"))}
                    </td>
                    <td className={putItm}>{num(cell(chain.put, row, "bid"))}</td>
                    <td className={putItm}>{num(cell(chain.put, row, "ask"))}</td>
                    <td className={putItm}>{compact(cell(chain.put, row, "volume"))}</td>
                    <td className={putItm}>{compact(cell(chain.put, row, "change_in_oi"))}</td>
                    <td className={putItm}>{compact(cell(chain.put, row, "oi"))}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}
