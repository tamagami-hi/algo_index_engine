import type {
  ChainColumns,
  Resolution,
  SideColumns,
  Snapshot,
  StreamFrame,
  Strategy,
} from "../types/api";

export function side(overrides: Partial<SideColumns> = {}): SideColumns {
  return {
    ltp: [129.3, 87.45],
    bid: [128.8, 87.0],
    bid_quantity: [750, 600],
    ask: [129.8, 87.9],
    ask_quantity: [900, 800],
    oi: [120000, 95000],
    change_in_oi: [0, 0],
    volume: [4200, 3100],
    change: [1.5, -0.75],
    quoted: [true, true],
    ...overrides,
  };
}

export function chain(overrides: Partial<ChainColumns> = {}): ChainColumns {
  return {
    segment: "NSE_FNO",
    symbol: "NIFTY",
    expiry: "2026-09-22",
    lot_size: 65,
    strikes: 2,
    quoted_strikes: 2,
    strike_step: 50,
    spot_price: 25050,
    spot_atm: 25050,
    market_atm: 25050,
    max_pain: 25050,
    atm_straddle: 216.75,
    synthetic_future: 25091.85,
    atm_imbalance: 0.1,
    total_call_oi: 120000,
    total_put_oi: 95000,
    total_combined_oi: 215000,
    call_volume: 4200,
    put_volume: 3100,
    pcr_oi: 0.79,
    pcr_volume: 0.74,
    market_atm_row: 0,
    strike: [25000, 25050],
    call: side(),
    put: side(),
    ...overrides,
  };
}

export function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    version: "0.1.0",
    sequence: 1,
    phase: "feed_connected",
    phase_label: "feed connected",
    detail: "",
    started_at_ms: 1_700_000_000_000,
    updated_at_ms: 1_700_000_000_500,
    uptime_seconds: 1,
    as_of: "2026-09-16",
    catalog: null,
    feed: {
      connected: true,
      connects: 1,
      disconnects: 0,
      frames: 10,
      bytes: 1620,
      last_frame_at_ms: 1_700_000_000_500,
      subscribed: 3688,
      packets: 100,
      index_packets: 8,
      ticker_packets: 0,
      quote_packets: 0,
      full_packets: 92,
      oi_packets: 0,
      prev_close_packets: 0,
      unknown_packets: 0,
      undecodable_frames: 0,
      two_sided_packets: 90,
      chains: 7,
      applied: 100,
      unmatched: 0,
      indices: { NIFTY: 25050 },
    },
    chains: [],
    ...overrides,
  };
}

export function frame(overrides: Partial<StreamFrame> = {}): StreamFrame {
  return {
    sequence: 1,
    published_at_ms: 1_700_000_000_500,
    feed_age_ms: 40,
    feed_silence_limit_ms: 5000,
    publish_interval_ms: 50,
    chain: chain(),
    state: snapshot(),
    ...overrides,
  };
}

export function strategy(overrides: Partial<Strategy> = {}): Strategy {
  return {
    id: "nifty-atm-straddle",
    name: "NIFTY ATM short straddle",
    underlying: "NIFTY",
    entry_time: "09:16",
    exit_time: "14:59",
    dte: [0, 1],
    entry_condition: { type: "always" },
    legs: [
      {
        side: "call",
        action: "sell",
        lots: 1,
        strike: { type: "relative", moneyness: "atm", steps: 0 },
        stop_loss: { method: "percent", value: 75 },
      },
      {
        side: "put",
        action: "sell",
        lots: 1,
        strike: { type: "relative", moneyness: "atm", steps: 0 },
        stop_loss: { method: "percent", value: 75 },
      },
    ],
    overall: {},
    loss_coverage: "breakeven",
    ...overrides,
  };
}

export function resolution(overrides: Partial<Resolution> = {}): Resolution {
  return {
    id: "nifty-atm-straddle",
    underlying: "NIFTY",
    expiry: "2026-09-22",
    spot_price: 25050,
    spot_atm: 25050,
    spot_age_ms: 120,
    entry_condition_met: true,
    entry_window_open: false,
    entry_closes_at: "09:17",
    before_hard_exit: true,
    minutes_until_exit: 180,
    days_to_expiry: 0,
    expiry_gate_met: true,
    dte_selection: "0DTE, 1DTE",
    lot_size: 65,
    legs: [],
    problems: [],
    blockers: [],
    blocked_because: [],
    would_enter_now: false,
    ...overrides,
  };
}
