import { useEffect, useState } from "react";
import { useEngine } from "../stores/engine";
import { Big, Panel, Stat, bytes, compact, duration, int, num } from "../components/ui";

function useTick(ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(timer);
  }, [ms]);
  return now;
}

export function Telemetry() {
  const snapshot = useEngine((store) => store.snapshot);
  const link = useEngine((store) => store.link);
  const receivedAt = useEngine((store) => store.receivedAt);
  const now = useTick(250);

  if (!snapshot) {
    return (
      <Panel title="engine">
        <span className="muted">
          {link === "open" ? "waiting for the first frame…" : "connecting to the engine…"}
        </span>
      </Panel>
    );
  }

  const feed = snapshot.feed;
  const frameAge = feed.last_frame_at_ms === null ? null : now - feed.last_frame_at_ms;
  const streamAge = receivedAt === null ? null : now - receivedAt;

  const feedTone =
    frameAge === null ? "muted" : frameAge < 2_000 ? "up" : frameAge < 10_000 ? "warnfg" : "down";

  const decoded =
    feed.packets > 0
      ? ((feed.packets - feed.unknown_packets) / feed.packets) * 100
      : null;
  const matched =
    feed.applied + feed.unmatched > 0
      ? (feed.applied / (feed.applied + feed.unmatched)) * 100
      : null;
  const twoSided =
    feed.full_packets > 0 ? (feed.two_sided_packets / feed.full_packets) * 100 : null;

  return (
    <>
      <Panel title="engine">
        <div className="big">
          <Big k="phase" v={snapshot.phase_label} tone={snapshot.phase === "feed_connected" ? "up" : snapshot.phase.endsWith("failed") ? "down" : "warnfg"} />
          <Big k="feed age" v={frameAge === null ? "no frames" : `${(frameAge / 1000).toFixed(1)}s`} tone={feedTone} />
          <Big k="stream age" v={streamAge === null ? "—" : `${(streamAge / 1000).toFixed(1)}s`} />
          <Big k="uptime" v={duration(snapshot.uptime_seconds)} />
          <Big k="subscribed" v={int(feed.subscribed)} />
        </div>
        {snapshot.detail ? <div className="err">{snapshot.detail}</div> : null}
        <div className="grid" style={{ marginTop: "0.5rem" }}>
          <Stat k="version" v={snapshot.version} />
          <Stat k="trading day" v={snapshot.as_of ?? "—"} />
          <Stat k="chains" v={int(feed.chains)} />
          <Stat k="connects" v={int(feed.connects)} />
          <Stat k="disconnects" v={int(feed.disconnects)} tone={feed.disconnects > 0 ? "warnfg" : undefined} />
        </div>
      </Panel>

      <Panel title="websocket telemetry">
        <div className="grid">
          <Stat k="link" v={link} tone={link === "open" ? "up" : "warnfg"} />
          <Stat k="socket" v={feed.connected ? "connected" : "down"} tone={feed.connected ? "up" : "down"} />
          <Stat k="frames" v={int(feed.frames)} />
          <Stat k="bytes" v={bytes(feed.bytes)} />
          <Stat k="packets" v={int(feed.packets)} />
          <Stat k="decoded" v={decoded === null ? "—" : `${decoded.toFixed(2)}%`} tone={decoded !== null && decoded < 100 ? "warnfg" : "up"} />
          <Stat k="undecodable frames" v={int(feed.undecodable_frames)} tone={feed.undecodable_frames > 0 ? "down" : undefined} />
          <Stat k="unknown packets" v={int(feed.unknown_packets)} tone={feed.unknown_packets > 0 ? "warnfg" : undefined} />
        </div>
      </Panel>

      <Panel title="packet mix">
        <div className="grid">
          <Stat k="full" v={int(feed.full_packets)} />
          <Stat k="index" v={int(feed.index_packets)} />
          <Stat k="ticker" v={int(feed.ticker_packets)} />
          <Stat k="quote" v={int(feed.quote_packets)} />
          <Stat k="open interest" v={int(feed.oi_packets)} />
          <Stat k="prev close" v={int(feed.prev_close_packets)} />
          <Stat k="two-sided" v={twoSided === null ? "—" : `${twoSided.toFixed(1)}%`} tone={twoSided !== null && twoSided < 90 ? "warnfg" : "up"} />
        </div>
      </Panel>

      <Panel title="routing">
        <div className="grid">
          <Stat k="applied" v={int(feed.applied)} />
          <Stat k="unmatched" v={int(feed.unmatched)} tone={feed.unmatched > 0 ? "warnfg" : undefined} />
          <Stat k="matched" v={matched === null ? "—" : `${matched.toFixed(2)}%`} tone={matched !== null && matched < 100 ? "warnfg" : "up"} />
        </div>
      </Panel>

      <Panel title="live index levels">
        {Object.keys(feed.indices).length === 0 ? (
          <span className="muted">no index prices received yet</span>
        ) : (
          <div className="grid">
            {Object.entries(feed.indices)
              .sort(([left], [right]) => left.localeCompare(right))
              .map(([label, price]) => (
                <Stat key={label} k={label} v={num(price)} />
              ))}
          </div>
        )}
      </Panel>

      {snapshot.catalog ? (
        <Panel title="subscription catalog">
          <div className="grid">
            <Stat k="spot instruments" v={int(snapshot.catalog.spot_instruments)} />
            <Stat k="option instruments" v={int(snapshot.catalog.option_instruments)} />
            <Stat k="total" v={int(snapshot.catalog.total_instruments)} />
            <Stat k="messages" v={int(snapshot.catalog.total_messages)} />
            <Stat k="spare capacity" v={int(snapshot.catalog.spare_capacity)} tone={snapshot.catalog.spare_capacity < 200 ? "warnfg" : undefined} />
            <Stat k="index spots" v={int(snapshot.catalog.spot_index)} />
            <Stat k="index futures" v={int(snapshot.catalog.spot_index_future)} />
          </div>
          {snapshot.catalog.missing_extra_spots.length > 0 ? (
            <div className="err">
              missing reference indices: {snapshot.catalog.missing_extra_spots.join(", ")}
            </div>
          ) : null}
          <div className="chainwrap" style={{ maxHeight: "18rem", marginTop: "0.6rem" }}>
            <table>
              <thead>
                <tr>
                  <th style={{ textAlign: "left" }}>segment</th>
                  <th style={{ textAlign: "left" }}>underlying</th>
                  <th style={{ textAlign: "left" }}>expiry</th>
                  <th>contracts</th>
                  <th style={{ textAlign: "left" }}>state</th>
                </tr>
              </thead>
              <tbody>
                {snapshot.catalog.chains.map((chain) => (
                  <tr key={`${chain.segment}-${chain.symbol}`}>
                    <td style={{ textAlign: "left" }} className="muted">{chain.segment}</td>
                    <td style={{ textAlign: "left" }}>{chain.symbol}</td>
                    <td style={{ textAlign: "left" }} className="muted">{chain.expiry}</td>
                    <td>{int(chain.contracts)}</td>
                    <td style={{ textAlign: "left" }}>
                      <span className={chain.excluded ? "tag" : "tag live"}>
                        {chain.excluded ? "excluded" : "subscribed"}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Panel>
      ) : null}

      <Panel title="market data by chain">
        {snapshot.chains.length === 0 ? (
          <span className="muted">no chains assembled yet</span>
        ) : (
          <div className="chainwrap" style={{ maxHeight: "20rem" }}>
            <table>
              <thead>
                <tr>
                  <th style={{ textAlign: "left" }}>index</th>
                  <th style={{ textAlign: "left" }}>expiry</th>
                  <th>spot</th>
                  <th>spot atm</th>
                  <th>market atm</th>
                  <th>max pain</th>
                  <th>atm straddle</th>
                  <th>pcr oi</th>
                  <th>quoted</th>
                </tr>
              </thead>
              <tbody>
                {snapshot.chains.map((chain) => (
                  <tr key={chain.symbol}>
                    <td style={{ textAlign: "left" }}>{chain.symbol}</td>
                    <td style={{ textAlign: "left" }} className="muted">{chain.expiry}</td>
                    <td>{num(chain.spot_price)}</td>
                    <td>{num(chain.spot_atm, 0)}</td>
                    <td>{num(chain.market_atm, 0)}</td>
                    <td>{num(chain.max_pain, 0)}</td>
                    <td>{num(chain.atm_straddle)}</td>
                    <td>{num(chain.pcr_oi)}</td>
                    <td className={chain.quoted_strikes === 0 ? "stale" : undefined}>
                      {compact(chain.quoted_strikes)}/{compact(chain.strikes)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>
    </>
  );
}
