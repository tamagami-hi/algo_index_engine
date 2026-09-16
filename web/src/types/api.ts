export type Phase =
  | "starting"
  | "authenticating"
  | "auth_failed"
  | "loading_instruments"
  | "instruments_failed"
  | "ready"
  | "feed_connected"
  | "feed_disconnected"
  | "shutting_down";

export interface FeedView {
  connected: boolean;
  connects: number;
  disconnects: number;
  frames: number;
  bytes: number;
  last_frame_at_ms: number | null;
  subscribed: number;
  packets: number;
  index_packets: number;
  ticker_packets: number;
  quote_packets: number;
  full_packets: number;
  oi_packets: number;
  prev_close_packets: number;
  unknown_packets: number;
  undecodable_frames: number;
  two_sided_packets: number;
  chains: number;
  applied: number;
  unmatched: number;
  indices: Record<string, number>;
}

export interface ChainSummaryView {
  symbol: string;
  expiry: string;
  spot_price: number;
  spot_atm: number | null;
  market_atm: number | null;
  max_pain: number | null;
  atm_straddle: number | null;
  pcr_oi: number;
  quoted_strikes: number;
  strikes: number;
}

export interface CatalogChain {
  segment: string;
  symbol: string;
  expiry: string;
  contracts: number;
  excluded: boolean;
}

export interface CatalogView {
  spot_instruments: number;
  spot_messages: number;
  option_instruments: number;
  option_messages: number;
  total_instruments: number;
  total_messages: number;
  spare_capacity: number;
  index_underlyings: number;
  spot_index: number;
  spot_index_future: number;
  missing_extra_spots: string[];
  chains: CatalogChain[];
}

export interface Snapshot {
  version: string;
  phase: Phase;
  phase_label: string;
  detail: string;
  started_at_ms: number;
  updated_at_ms: number;
  uptime_seconds: number;
  as_of: string | null;
  catalog: CatalogView | null;
  feed: FeedView;
  chains: ChainSummaryView[];
}

export interface ChainMetrics {
  segment: string;
  symbol: string;
  expiry: string;
  lot_size: number;
  strikes: number;
  quoted_strikes: number;
  strike_step: number;
  spot_price: number;
  spot_atm: number | null;
  market_atm: number | null;
  max_pain: number | null;
  atm_straddle: number | null;
  synthetic_future: number | null;
  atm_imbalance: number | null;
  total_call_oi: number;
  total_put_oi: number;
  total_combined_oi: number;
  call_volume: number;
  put_volume: number;
  pcr_oi: number;
  pcr_volume: number;
}

export interface SideColumns {
  ltp: (number | null)[];
  bid: (number | null)[];
  bid_quantity: (number | null)[];
  ask: (number | null)[];
  ask_quantity: (number | null)[];
  oi: (number | null)[];
  change_in_oi: (number | null)[];
  volume: (number | null)[];
  change: (number | null)[];
  quoted: boolean[];
}

export interface ChainColumns extends ChainMetrics {
  market_atm_row: number | null;
  strike: number[];
  call: SideColumns;
  put: SideColumns;
}

export interface StreamFrame {
  chain?: ChainColumns;
  state: Snapshot;
}

export type RiskMethod = "percent" | "points";

export interface Threshold {
  method: RiskMethod;
  value: number;
}

export interface TrailingRule {
  arm_at: Threshold;
  give_back: Threshold;
  start_after_minutes?: number;
}

export type Moneyness = "atm" | "itm" | "otm";

export type StrikeCriteria =
  | { type: "relative"; moneyness: Moneyness; steps: number }
  | { type: "closest_premium"; target: number }
  | { type: "premium_range"; lower: number; upper: number }
  | { type: "premium_at_least"; threshold: number }
  | { type: "premium_at_most"; threshold: number }
  | { type: "straddle_width"; multiplier: number; away_from_atm: boolean };

export interface LegDefinition {
  side: "call" | "put";
  action: "sell" | "buy";
  lots: number;
  strike: StrikeCriteria;
  stop_loss?: Threshold;
  target?: Threshold;
  trailing?: TrailingRule;
}

export type LossCoverage = "none" | "breakeven" | "recover_stopped_leg_loss";

export type EntryCondition =
  | { type: "always" }
  | { type: "reference_above"; reference: string; value: number }
  | { type: "reference_at_most"; reference: string; value: number };

export interface OverallRisk {
  stop_loss?: Threshold;
  target?: Threshold;
  trailing?: TrailingRule;
  daily_loss_limit?: number;
  daily_profit_target?: number;
}

export const MAX_DTE = 6;
export const DTE_CHOICES: number[] = Array.from({ length: MAX_DTE + 1 }, (_, day) => day);

export const MAX_PREMIUM_PERCENT = 100;

export type DteSelection = number[];

export interface Strategy {
  id: string;
  name: string;
  notes?: string;
  underlying: string;
  entry_time: string;
  exit_time: string;
  dte: DteSelection;
  entry_condition: EntryCondition;
  legs: LegDefinition[];
  overall: OverallRisk;
  loss_coverage: LossCoverage;
}

export interface Unreadable {
  file: string;
  problem: string;
}

export interface Listing {
  strategies: Strategy[];
  unreadable: Unreadable[];
}

export interface ResolvedLeg {
  leg: number;
  side: "call" | "put";
  strike: number;
  row: number;
  security_id: string;
  ltp: number;
  bid: number;
  ask: number;
  action: "sell" | "buy";
  lots: number;
  quantity: number;
  stop_price: number | null;
  target_price: number | null;
}

export interface LegProblem {
  leg: number;
  side: string;
  reason: string;
  wanted?: string;
  strike?: string;
}

export interface Resolution {
  id: string;
  underlying: string;
  expiry: string;
  spot_price: number;
  spot_atm: number | null;
  entry_condition_met: boolean;
  entry_window_open: boolean;
  entry_closes_at: string;
  before_hard_exit: boolean;
  minutes_until_exit: number;
  days_to_expiry: number | null;
  expiry_gate_met: boolean;
  dte_selection: string;
  lot_size: number;
  legs: ResolvedLeg[];
  problems: LegProblem[];
  sizing?: { problem: string; underlying: string };
  would_enter_now: boolean;
}
