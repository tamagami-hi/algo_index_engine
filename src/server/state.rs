use std::sync::Arc;

use serde::Serialize;
use tokio::sync::watch;

use crate::dhan_api::feed::Packet;
use crate::dhan_api::instruments::Catalog;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Starting,
    Authenticating,
    AuthFailed,
    LoadingInstruments,
    InstrumentsFailed,
    Ready,
    FeedConnected,
    FeedDisconnected,
    ShuttingDown,
}

impl Phase {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Authenticating => "authenticating",
            Self::AuthFailed => "auth failed",
            Self::LoadingInstruments => "loading instruments",
            Self::InstrumentsFailed => "instruments failed",
            Self::Ready => "ready",
            Self::FeedConnected => "feed connected",
            Self::FeedDisconnected => "feed disconnected",
            Self::ShuttingDown => "shutting down",
        }
    }

    pub(crate) const fn is_healthy(self) -> bool {
        matches!(self, Self::Ready | Self::FeedConnected)
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ChainView {
    pub(crate) segment: String,
    pub(crate) symbol: String,
    pub(crate) expiry: String,
    pub(crate) contracts: usize,
    pub(crate) excluded: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CatalogView {
    pub(crate) spot_instruments: usize,
    pub(crate) spot_messages: usize,
    pub(crate) option_instruments: usize,
    pub(crate) option_messages: usize,
    pub(crate) total_instruments: usize,
    pub(crate) total_messages: usize,
    pub(crate) spare_capacity: usize,
    pub(crate) index_underlyings: usize,
    pub(crate) spot_index: usize,
    pub(crate) spot_index_future: usize,
    pub(crate) missing_extra_spots: Vec<String>,
    pub(crate) chains: Vec<ChainView>,
}

impl CatalogView {
    fn from_catalog(catalog: &Catalog) -> Self {
        let report = &catalog.report;
        let mut chains: Vec<ChainView> = report
            .index_chains
            .iter()
            .map(|chain| ChainView {
                segment: chain.underlying.segment.as_str().to_owned(),
                symbol: chain.underlying.symbol.clone(),
                expiry: chain.expiry.clone(),
                contracts: chain.contracts,
                excluded: false,
            })
            .collect();
        chains.extend(report.excluded_index_chains.iter().map(|chain| ChainView {
            segment: chain.underlying.segment.as_str().to_owned(),
            symbol: chain.underlying.symbol.clone(),
            expiry: chain.expiry.clone(),
            contracts: chain.contracts,
            excluded: true,
        }));

        Self {
            spot_instruments: catalog.spot.len(),
            spot_messages: catalog.spot.message_count(),
            option_instruments: catalog.index_options.len(),
            option_messages: catalog.index_options.message_count(),
            total_instruments: catalog.len(),
            total_messages: catalog.message_count(),
            spare_capacity: catalog.spare_capacity(),
            index_underlyings: report.index_underlyings,
            spot_index: report.spot_index,
            spot_index_future: report.spot_index_future,
            missing_extra_spots: report.missing_extra_spots.clone(),
            chains,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct FeedView {
    pub(crate) connected: bool,
    pub(crate) connects: u64,
    pub(crate) disconnects: u64,
    pub(crate) frames: u64,
    pub(crate) bytes: u64,
    pub(crate) last_frame_at: Option<i64>,
    pub(crate) subscribed: usize,
    pub(crate) packets: u64,
    pub(crate) index_packets: u64,
    pub(crate) ticker_packets: u64,
    pub(crate) quote_packets: u64,
    pub(crate) full_packets: u64,
    pub(crate) oi_packets: u64,
    pub(crate) prev_close_packets: u64,
    pub(crate) unknown_packets: u64,
    pub(crate) undecodable_frames: u64,
    pub(crate) two_sided_packets: u64,
}


#[derive(Clone, Debug, Serialize)]
pub(crate) struct Snapshot {
    pub(crate) version: &'static str,
    pub(crate) phase: Phase,
    pub(crate) phase_label: &'static str,
    pub(crate) detail: String,
    pub(crate) started_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) uptime_seconds: i64,
    pub(crate) as_of: Option<String>,
    pub(crate) catalog: Option<CatalogView>,
    pub(crate) feed: FeedView,
}

impl Snapshot {
    fn new(now: i64) -> Self {
        Self {
            version: VERSION,
            phase: Phase::Starting,
            phase_label: Phase::Starting.label(),
            detail: String::new(),
            started_at: now,
            updated_at: now,
            uptime_seconds: 0,
            as_of: None,
            catalog: None,
            feed: FeedView::default(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct EngineState {
    sender: Arc<watch::Sender<Snapshot>>,
}

impl EngineState {
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(Snapshot::new(now_unix()));
        Self {
            sender: Arc::new(sender),
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.sender.subscribe()
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.sender.borrow().clone();
        snapshot.uptime_seconds = now_unix() - snapshot.started_at;
        snapshot
    }

    fn update(&self, apply: impl FnOnce(&mut Snapshot)) {
        self.sender.send_modify(|snapshot| {
            apply(snapshot);
            let now = now_unix();
            snapshot.updated_at = now;
            snapshot.uptime_seconds = now - snapshot.started_at;
            snapshot.phase_label = snapshot.phase.label();
        });
    }

    pub(crate) fn set_phase(&self, phase: Phase, detail: impl Into<String>) {
        self.update(|snapshot| {
            snapshot.phase = phase;
            snapshot.detail = detail.into();
        });
    }

    pub(crate) fn set_catalog(&self, as_of: &str, catalog: &Catalog) {
        let view = CatalogView::from_catalog(catalog);
        self.update(|snapshot| {
            snapshot.as_of = Some(as_of.to_owned());
            snapshot.catalog = Some(view);
        });
    }


    pub(crate) fn feed_connected(&self) {
        self.update(|snapshot| {
            snapshot.phase = Phase::FeedConnected;
            snapshot.detail = String::new();
            snapshot.feed.connected = true;
            snapshot.feed.connects += 1;
        });
    }

    pub(crate) fn feed_disconnected(&self, detail: impl Into<String>) {
        self.update(|snapshot| {
            snapshot.phase = Phase::FeedDisconnected;
            snapshot.detail = detail.into();
            snapshot.feed.connected = false;
            snapshot.feed.disconnects += 1;
        });
    }

    pub(crate) fn feed_frame(&self, bytes: usize) {
        self.update(|snapshot| {
            snapshot.feed.frames += 1;
            snapshot.feed.bytes += bytes as u64;
            snapshot.feed.last_frame_at = Some(now_unix());
        });
    }

    pub(crate) fn feed_subscribed(&self, instruments: usize) {
        self.update(|snapshot| {
            snapshot.feed.subscribed = instruments;
        });
    }

    pub(crate) fn feed_undecodable(&self, _bytes: usize) {
        self.update(|snapshot| {
            snapshot.feed.undecodable_frames += 1;
        });
    }

    pub(crate) fn feed_packet(&self, message: &crate::dhan_api::feed::Message) {
        self.update(|snapshot| {
            snapshot.feed.packets += 1;
            let feed = &mut snapshot.feed;
            match message.packet {
                Packet::Index { .. } => feed.index_packets += 1,
                Packet::Ticker { .. } => feed.ticker_packets += 1,
                Packet::Quote { .. } => feed.quote_packets += 1,
                Packet::Full(full) => {
                    feed.full_packets += 1;
                    if full.best_bid().is_some() && full.best_ask().is_some() {
                        feed.two_sided_packets += 1;
                    }
                }
                Packet::OpenInterest { .. } => feed.oi_packets += 1,
                Packet::PrevClose { .. } => feed.prev_close_packets += 1,
                Packet::Unknown { .. } => feed.unknown_packets += 1,
                Packet::Disconnect { .. } => {}
            }
        });
    }
}

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}
