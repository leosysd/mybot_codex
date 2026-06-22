use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer as _;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use polymarket_client_sdk_v2::auth::state::Authenticated;
use polymarket_client_sdk_v2::auth::Normal;
use polymarket_client_sdk_v2::clob::types::{Amount, OrderType, Side, SignatureType};
use polymarket_client_sdk_v2::clob::{Client, Config as ClobConfig};
use polymarket_client_sdk_v2::types::{Address, Decimal as SdkDecimal, U256};
use polymarket_client_sdk_v2::POLYGON;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let env_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".env".to_string());
    if Path::new(&env_path).exists() {
        dotenvy::from_path(&env_path).with_context(|| format!("load env {env_path}"))?;
    }

    let cfg = Config::from_env()?;
    fs::create_dir_all(&cfg.data_dir).await?;
    if let Some(parent) = cfg.signal_file.parent() {
        fs::create_dir_all(parent).await?;
    }

    info!(
        "mybot-codex dry_run={} strategy=t1_late target_qty={} poll={}ms",
        cfg.dry_run, cfg.target_qty, cfg.poll_ms
    );

    let cache = new_book_cache();
    let ws = MarketWs::new(cfg.market_ws_url.clone(), cache.clone());
    let _ws_task = ws.clone().run();
    let executor = Arc::new(OrderExecutor::new(&cfg).await?);
    let mut bot = Bot::new(cfg, cache, ws, executor).await?;
    let poll = tokio::time::Duration::from_millis(bot.cfg.poll_ms);

    loop {
        let run = tokio::time::timeout(tokio::time::Duration::from_secs(2), bot.run_once()).await;
        match run {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error!("run_once error: {e:#}"),
            Err(_) => error!("run_once timeout"),
        }

        tokio::select! {
            _ = bot.ws.wait_book_update() => {}
            _ = tokio::time::sleep(poll) => {}
        }
    }
}

#[derive(Clone, Debug)]
struct Config {
    dry_run: bool,
    private_key: Option<String>,
    deposit_wallet: Option<String>,
    signature_type: u8,
    clob_api_url: String,
    clob_v2_api_url: String,
    gamma_api_url: String,
    market_ws_url: String,
    market_slug_prefix: String,
    data_dir: PathBuf,
    signal_file: PathBuf,
    state_file: PathBuf,
    poll_ms: u64,
    confirm1_secs: i64,
    confirm2_secs: i64,
    entry_secs: i64,
    confirm_min_ask: f64,
    entry_min_ask: f64,
    opp_max_ask: f64,
    target_qty: f64,
    start_equity: f64,
    risk_fraction: f64,
    max_deploy_usdc: f64,
    rest_fallback_timeout_ms: u64,
}

impl Config {
    fn from_env() -> Result<Self> {
        let data_dir = PathBuf::from(env("DATA_DIR", "data"));
        Ok(Self {
            dry_run: env_bool("DRY_RUN", true),
            private_key: env_opt("PRIVATE_KEY"),
            deposit_wallet: env_opt("DEPOSIT_WALLET_ADDRESS"),
            signature_type: env_u64("SIGNATURE_TYPE", 3) as u8,
            clob_api_url: env("CLOB_API_URL", "https://clob.polymarket.com"),
            clob_v2_api_url: env("CLOB_V2_API_URL", "https://clob.polymarket.com"),
            gamma_api_url: env("GAMMA_API_URL", "https://gamma-api.polymarket.com"),
            market_ws_url: env(
                "MARKET_WS_URL",
                "wss://ws-subscriptions-clob.polymarket.com/ws/market",
            ),
            market_slug_prefix: env("MARKET_SLUG_PREFIX", "btc-updown-5m"),
            signal_file: PathBuf::from(env("SIGNAL_FILE", "data/t1_late_signals.jsonl")),
            state_file: PathBuf::from(env("STATE_FILE", "data/t1_late_state.json")),
            data_dir,
            poll_ms: env_u64("POLL_MS", 200),
            confirm1_secs: env_i64("T1_LATE_CONFIRM1_SECS", 10),
            confirm2_secs: env_i64("T1_LATE_CONFIRM2_SECS", 8),
            entry_secs: env_i64("T1_LATE_ENTRY_SECS", 1),
            confirm_min_ask: env_f64("T1_LATE_CONFIRM_MIN_ASK", 0.98),
            entry_min_ask: env_f64("T1_LATE_ENTRY_MIN_ASK", 0.75),
            opp_max_ask: env_f64("T1_LATE_OPP_MAX_ASK", 0.30),
            target_qty: env_f64("T1_LATE_TARGET_QTY", 2000.0),
            start_equity: env_f64("T1_LATE_START_EQUITY", 300.0),
            risk_fraction: env_f64("T1_LATE_RISK_FRACTION", 1.0),
            max_deploy_usdc: env_f64("T1_LATE_MAX_DEPLOY_USDC", 0.0),
            rest_fallback_timeout_ms: env_u64("REST_FALLBACK_TIMEOUT_MS", 700),
        })
    }
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

#[derive(Debug, Clone)]
struct Market {
    slug: String,
    start_ts: i64,
    end_ts: i64,
    outcomes: Vec<String>,
    token_ids: Vec<String>,
}

impl Market {
    fn seconds_left(&self) -> i64 {
        (self.end_ts - Utc::now().timestamp()).max(0)
    }

    fn token_for(&self, outcome: &str) -> Option<&str> {
        self.outcomes
            .iter()
            .position(|o| o == outcome)
            .and_then(|i| self.token_ids.get(i))
            .map(|s| s.as_str())
    }
}

#[derive(Debug, Clone, Default)]
struct OrderBook {
    asks: Vec<(f64, f64)>,
    bids: Vec<(f64, f64)>,
}

impl OrderBook {
    fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|(p, _)| *p)
    }

    fn best_ask_size(&self) -> Option<f64> {
        self.asks.first().map(|(_, s)| *s)
    }
}

type BookCache = Arc<RwLock<HashMap<String, OrderBook>>>;

fn new_book_cache() -> BookCache {
    Arc::new(RwLock::new(HashMap::new()))
}

#[derive(Clone)]
struct ClobClient {
    http: reqwest::Client,
    clob_api: String,
    gamma_api: String,
    slug_prefix: String,
}

impl ClobClient {
    fn new(cfg: &Config) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()?,
            clob_api: cfg.clob_api_url.clone(),
            gamma_api: cfg.gamma_api_url.clone(),
            slug_prefix: cfg.market_slug_prefix.clone(),
        })
    }

    async fn find_current_market(&self) -> Option<Market> {
        let now = Utc::now().timestamp();
        let slot = (now / 300) * 300;
        for candidate in [slot, slot - 300, slot + 300] {
            if let Some(m) = self.fetch_market(candidate).await {
                if m.start_ts <= now && now < m.end_ts {
                    return Some(m);
                }
            }
        }
        None
    }

    async fn fetch_market(&self, start_ts: i64) -> Option<Market> {
        let slug = format!("{}-{}", self.slug_prefix, start_ts);
        let url = format!("{}/events/slug/{}", self.gamma_api, slug);
        let resp: serde_json::Value = self.http.get(url).send().await.ok()?.json().await.ok()?;
        let market = resp.get("markets")?.as_array()?.first()?;
        if market.get("acceptingOrders")?.as_bool() != Some(true) {
            return None;
        }
        let outcomes = parse_json_list(market.get("outcomes")?)?;
        let token_ids = parse_json_list(market.get("clobTokenIds")?)?;
        if !outcomes.contains(&"Up".to_string()) || !outcomes.contains(&"Down".to_string()) {
            return None;
        }
        let event_start = market
            .get("eventStartTime")
            .or_else(|| resp.get("startTime"))
            .and_then(|v| v.as_str())?;
        let end_date = market
            .get("endDate")
            .or_else(|| resp.get("endDate"))
            .and_then(|v| v.as_str())?;

        Some(Market {
            slug,
            start_ts: iso_to_ts(event_start)?,
            end_ts: iso_to_ts(end_date)?,
            outcomes,
            token_ids,
        })
    }

    async fn fetch_book(&self, token_id: &str) -> Result<OrderBook> {
        let url = format!("{}/book?token_id={}", self.clob_api, token_id);
        let resp: serde_json::Value = self.http.get(url).send().await?.json().await?;
        if resp.get("error").is_some() {
            bail!("book error for {token_id}: {resp}");
        }
        Ok(parse_book(&resp))
    }

    async fn fetch_winner(&self, slug: &str) -> Option<String> {
        let url = format!("{}/events/slug/{}", self.gamma_api, slug);
        let resp: serde_json::Value = self.http.get(url).send().await.ok()?.json().await.ok()?;
        let market = resp.get("markets")?.as_array()?.first()?;
        if market.get("closed")?.as_bool() != Some(true) {
            return None;
        }
        let outcomes = parse_json_list(market.get("outcomes")?)?;
        let prices = parse_json_list(market.get("outcomePrices")?)?;
        for (outcome, price) in outcomes.into_iter().zip(prices.into_iter()) {
            if price.parse::<f64>().ok()? >= 0.99 {
                return Some(outcome);
            }
        }
        None
    }
}

fn parse_json_list(v: &serde_json::Value) -> Option<Vec<String>> {
    match v {
        serde_json::Value::Array(arr) => Some(
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
        ),
        serde_json::Value::String(s) => serde_json::from_str::<Vec<String>>(s).ok(),
        _ => None,
    }
}

fn iso_to_ts(s: &str) -> Option<i64> {
    let fixed = s.replace('Z', "+00:00");
    DateTime::parse_from_rfc3339(&fixed)
        .ok()
        .map(|dt| dt.timestamp())
}

fn parse_book(v: &serde_json::Value) -> OrderBook {
    let parse_side = |key: &str| -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = v
            .get(key)
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let p = l.get("price")?.as_str()?.parse().ok()?;
                let s = l.get("size")?.as_str()?.parse().ok()?;
                Some((p, s))
            })
            .collect();
        if key == "asks" {
            out.sort_by(|a, b| a.0.total_cmp(&b.0));
        } else {
            out.sort_by(|a, b| b.0.total_cmp(&a.0));
        }
        out
    };
    OrderBook {
        asks: parse_side("asks"),
        bids: parse_side("bids"),
    }
}

#[derive(Clone)]
struct MarketWs {
    inner: Arc<WsInner>,
}

struct WsInner {
    url: String,
    subscribed: Mutex<HashSet<String>>,
    cache: BookCache,
    reconnect: Notify,
    book_updated: Notify,
}

impl MarketWs {
    fn new(url: String, cache: BookCache) -> Self {
        Self {
            inner: Arc::new(WsInner {
                url,
                subscribed: Mutex::new(HashSet::new()),
                cache,
                reconnect: Notify::new(),
                book_updated: Notify::new(),
            }),
        }
    }

    async fn wait_book_update(&self) {
        self.inner.book_updated.notified().await;
    }

    async fn ensure_subscribed(&self, token_ids: &[String]) {
        let new_set: HashSet<String> = token_ids.iter().cloned().collect();
        let mut sub = self.inner.subscribed.lock().await;
        if *sub != new_set {
            *sub = new_set;
            drop(sub);
            self.inner.reconnect.notify_one();
        }
    }

    fn run(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match self.connect_once().await {
                    Ok(true) => info!("ws reconnect requested"),
                    Ok(false) => info!("ws closed"),
                    Err(e) => warn!("ws error: {e:#}"),
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        })
    }

    async fn connect_once(&self) -> Result<bool> {
        let (ws_stream, _) = connect_async(&self.inner.url).await?;
        info!("ws connected {}", self.inner.url);
        let (mut write, mut read) = ws_stream.split();
        let assets: Vec<String> = self.inner.subscribed.lock().await.iter().cloned().collect();
        if !assets.is_empty() {
            let msg = json!({ "assets_ids": assets, "type": "market" });
            write.send(Message::Text(msg.to_string().into())).await?;
            info!("ws subscribed {}", assets.len());
        }

        loop {
            tokio::select! {
                _ = self.inner.reconnect.notified() => return Ok(true),
                msg = read.next() => {
                    let Some(msg) = msg else { return Ok(false); };
                    match msg? {
                        Message::Text(text) => self.handle_message(&text).await,
                        Message::Ping(data) => { let _ = write.send(Message::Pong(data)).await; }
                        Message::Close(_) => return Ok(false),
                        _ => {}
                    }
                }
            }
        }
    }

    async fn handle_message(&self, text: &str) {
        let Ok(data): Result<serde_json::Value, _> = serde_json::from_str(text) else {
            return;
        };
        let events: &[serde_json::Value] = match &data {
            serde_json::Value::Array(arr) => arr.as_slice(),
            serde_json::Value::Object(_) => std::slice::from_ref(&data),
            _ => return,
        };

        let mut touched = false;
        {
            let mut cache = self.inner.cache.write().await;
            for event in events {
                let ev_type = event
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let Some(asset_id) = event.get("asset_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                if ev_type == "book" {
                    cache.insert(asset_id.to_string(), parse_book(event));
                    touched = true;
                } else if ev_type == "price_change" {
                    if let Some(book) = cache.get_mut(asset_id) {
                        apply_price_change(book, event);
                        touched = true;
                    }
                }
            }
        }
        if touched {
            self.inner.book_updated.notify_one();
        }
    }
}

fn apply_price_change(book: &mut OrderBook, event: &serde_json::Value) {
    let updates = |key: &str| -> Vec<(f64, f64)> {
        event
            .get(key)
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let p = l.get("price")?.as_str()?.parse().ok()?;
                let s = l.get("size")?.as_str()?.parse().ok()?;
                Some((p, s))
            })
            .collect()
    };

    for (price, size) in updates("asks") {
        book.asks.retain(|(p, _)| *p != price);
        if size > 0.0 {
            book.asks.push((price, size));
        }
    }
    book.asks.sort_by(|a, b| a.0.total_cmp(&b.0));

    for (price, size) in updates("bids") {
        book.bids.retain(|(p, _)| *p != price);
        if size > 0.0 {
            book.bids.push((price, size));
        }
    }
    book.bids.sort_by(|a, b| b.0.total_cmp(&a.0));
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    trades: Vec<Trade>,
}

impl State {
    async fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path).await {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    async fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?).await?;
        Ok(())
    }

    fn realized_pnl(&self) -> f64 {
        self.trades.iter().filter_map(|t| t.pnl).sum()
    }

    fn has_trade(&self, slug: &str) -> bool {
        self.trades.iter().any(|t| t.market == slug)
    }

    fn pending_markets(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.trades {
            if t.winner.is_none() && !out.contains(&t.market) {
                out.push(t.market.clone());
            }
        }
        out
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Trade {
    market: String,
    end_ts: i64,
    side: String,
    price: f64,
    shares: f64,
    cost: f64,
    ts: i64,
    winner: Option<String>,
    pnl: Option<f64>,
}

#[derive(Clone, Default)]
struct T1State {
    confirm1_side: String,
    confirm1_ask: f64,
    confirm1_seen_at: i64,
    confirm2_side: String,
    confirm2_ask: f64,
    confirm2_seen_at: i64,
    locked: bool,
}

struct Bot {
    cfg: Config,
    client: ClobClient,
    executor: Arc<OrderExecutor>,
    cache: BookCache,
    ws: MarketWs,
    market: Option<Market>,
    state: State,
    t1: HashMap<String, T1State>,
    last_settlement_check: i64,
    last_tail_log_ts: i64,
}

impl Bot {
    async fn new(
        cfg: Config,
        cache: BookCache,
        ws: MarketWs,
        executor: Arc<OrderExecutor>,
    ) -> Result<Self> {
        let client = ClobClient::new(&cfg)?;
        let state = State::load(&cfg.state_file).await?;
        Ok(Self {
            cfg,
            client,
            executor,
            cache,
            ws,
            market: None,
            state,
            t1: HashMap::new(),
            last_settlement_check: 0,
            last_tail_log_ts: 0,
        })
    }

    async fn run_once(&mut self) -> Result<()> {
        let now = Utc::now().timestamp();
        if now - self.last_settlement_check >= 10 {
            self.last_settlement_check = now;
            self.check_settlements().await?;
        }

        let Some(market) = self.current_market().await else {
            return Ok(());
        };
        let seconds_left = market.seconds_left();
        if seconds_left <= 0 {
            return Ok(());
        }

        let up_idx = market
            .outcomes
            .iter()
            .position(|o| o == "Up")
            .ok_or_else(|| anyhow!("market has no Up outcome"))?;
        let dn_idx = market
            .outcomes
            .iter()
            .position(|o| o == "Down")
            .ok_or_else(|| anyhow!("market has no Down outcome"))?;
        let up_token = &market.token_ids[up_idx];
        let dn_token = &market.token_ids[dn_idx];
        let tail_window = seconds_left <= self.cfg.confirm1_secs + 20;

        let (up_ask, up_size, up_source) = self.top_ask(up_token, tail_window).await;
        let (dn_ask, dn_size, dn_source) = self.top_ask(dn_token, tail_window).await;
        let (Some(up_ask), Some(dn_ask)) = (up_ask, dn_ask) else {
            if tail_window && now > self.last_tail_log_ts {
                self.last_tail_log_ts = now;
                self.signal(json!({
                    "phase": "t1_late_book_missing",
                    "market": market.slug,
                    "seconds_left": seconds_left,
                    "up_has_ask": up_ask.is_some(),
                    "dn_has_ask": dn_ask.is_some(),
                    "up_source": up_source,
                    "dn_source": dn_source,
                    "ts": now,
                }))
                .await?;
            }
            return Ok(());
        };

        if tail_window && now > self.last_tail_log_ts {
            self.last_tail_log_ts = now;
            self.signal(json!({
                "phase": "t1_late_tail",
                "market": market.slug,
                "seconds_left": seconds_left,
                "up_ask": up_ask,
                "dn_ask": dn_ask,
                "up_size": up_size,
                "dn_size": dn_size,
                "up_source": up_source,
                "dn_source": dn_source,
                "ts": now,
            }))
            .await?;
        }

        self.decide_t1_late(
            &market,
            up_ask,
            up_size.unwrap_or(0.0),
            dn_ask,
            dn_size.unwrap_or(0.0),
            seconds_left,
        )
        .await
    }

    async fn current_market(&mut self) -> Option<Market> {
        let now = Utc::now().timestamp();
        if let Some(m) = &self.market {
            if now < m.end_ts {
                return Some(m.clone());
            }
        }
        let market = self.client.find_current_market().await?;
        let is_new = self
            .market
            .as_ref()
            .map(|m| m.slug != market.slug)
            .unwrap_or(true);
        if is_new {
            self.ws.ensure_subscribed(&market.token_ids).await;
            info!("new market {}", market.slug);
            self.signal(json!({
                "phase": "market",
                "market": market.slug,
                "start_ts": market.start_ts,
                "end_ts": market.end_ts,
                "ts": now,
            }))
            .await
            .ok();
        }
        self.market = Some(market.clone());
        Some(market)
    }

    async fn top_ask(
        &self,
        token_id: &str,
        allow_rest: bool,
    ) -> (Option<f64>, Option<f64>, &'static str) {
        {
            let cache = self.cache.read().await;
            if let Some(book) = cache.get(token_id) {
                if let Some(ask) = book.best_ask() {
                    return (Some(ask), book.best_ask_size(), "ws");
                }
            }
        }
        if !allow_rest {
            return (None, None, "missing");
        }
        let fetch = tokio::time::timeout(
            tokio::time::Duration::from_millis(self.cfg.rest_fallback_timeout_ms),
            self.client.fetch_book(token_id),
        )
        .await;
        let Ok(Ok(book)) = fetch else {
            return (None, None, "rest_failed");
        };
        let ask = book.best_ask();
        let size = book.best_ask_size();
        {
            let mut cache = self.cache.write().await;
            cache.insert(token_id.to_string(), book);
        }
        (ask, size, "rest")
    }

    async fn decide_t1_late(
        &mut self,
        market: &Market,
        up_ask: f64,
        up_size: f64,
        dn_ask: f64,
        dn_size: f64,
        seconds_left: i64,
    ) -> Result<()> {
        if self.state.has_trade(&market.slug) {
            self.t1.entry(market.slug.clone()).or_default().locked = true;
            return Ok(());
        }
        let now = Utc::now().timestamp();
        let (side, main_ask, opp_side, opp_ask, ask_size) =
            strong_side(up_ask, up_size, dn_ask, dn_size);

        let mut signal = None;
        {
            let st = self.t1.entry(market.slug.clone()).or_default();
            if st.locked {
                return Ok(());
            }
            if st.confirm1_side.is_empty()
                && seconds_left <= self.cfg.confirm1_secs
                && seconds_left > self.cfg.confirm2_secs
            {
                if main_ask < self.cfg.confirm_min_ask {
                    st.locked = true;
                    signal = Some(json!({
                        "phase": "t1_late_block",
                        "reason": "confirm1_below_min",
                        "market": market.slug,
                        "seconds_left": seconds_left,
                        "side": side,
                        "main_ask": main_ask,
                        "confirm_min": self.cfg.confirm_min_ask,
                        "up_ask": up_ask,
                        "dn_ask": dn_ask,
                        "ts": now,
                    }));
                } else {
                    st.confirm1_side = side.to_string();
                    st.confirm1_ask = main_ask;
                    st.confirm1_seen_at = seconds_left;
                    signal = Some(json!({
                        "phase": "t1_late_confirm1",
                        "market": market.slug,
                        "seconds_left": seconds_left,
                        "side": side,
                        "main_ask": main_ask,
                        "up_ask": up_ask,
                        "dn_ask": dn_ask,
                        "ts": now,
                    }));
                }
            }
        }
        if let Some(v) = signal.take() {
            self.signal(v).await?;
        }

        let mut signal = None;
        {
            let st = self.t1.entry(market.slug.clone()).or_default();
            if st.locked {
                return Ok(());
            }
            if st.confirm2_side.is_empty()
                && seconds_left <= self.cfg.confirm2_secs
                && seconds_left > self.cfg.entry_secs
            {
                if st.confirm1_side.is_empty() {
                    st.locked = true;
                    signal = Some(json!({
                        "phase": "t1_late_block",
                        "reason": "missing_confirm1",
                        "market": market.slug,
                        "seconds_left": seconds_left,
                        "up_ask": up_ask,
                        "dn_ask": dn_ask,
                        "ts": now,
                    }));
                } else if side != st.confirm1_side {
                    st.locked = true;
                    signal = Some(json!({
                        "phase": "t1_late_block",
                        "reason": "confirm2_side_changed",
                        "market": market.slug,
                        "confirm1_side": st.confirm1_side,
                        "confirm2_side": side,
                        "seconds_left": seconds_left,
                        "up_ask": up_ask,
                        "dn_ask": dn_ask,
                        "ts": now,
                    }));
                } else if main_ask < self.cfg.confirm_min_ask {
                    st.locked = true;
                    signal = Some(json!({
                        "phase": "t1_late_block",
                        "reason": "confirm2_below_min",
                        "market": market.slug,
                        "side": side,
                        "main_ask": main_ask,
                        "confirm_min": self.cfg.confirm_min_ask,
                        "seconds_left": seconds_left,
                        "up_ask": up_ask,
                        "dn_ask": dn_ask,
                        "ts": now,
                    }));
                } else {
                    st.confirm2_side = side.to_string();
                    st.confirm2_ask = main_ask;
                    st.confirm2_seen_at = seconds_left;
                    signal = Some(json!({
                        "phase": "t1_late_confirm2",
                        "market": market.slug,
                        "seconds_left": seconds_left,
                        "side": side,
                        "main_ask": main_ask,
                        "confirm1_ask": st.confirm1_ask,
                        "up_ask": up_ask,
                        "dn_ask": dn_ask,
                        "ts": now,
                    }));
                }
            }
        }
        if let Some(v) = signal.take() {
            self.signal(v).await?;
        }

        if seconds_left > self.cfg.entry_secs {
            return Ok(());
        }

        let st = self.t1.get(&market.slug).cloned().unwrap_or_default();
        if st.locked {
            return Ok(());
        }
        self.t1.entry(market.slug.clone()).or_default().locked = true;

        if st.confirm1_side.is_empty() || st.confirm2_side.is_empty() {
            self.signal(json!({
                "phase": "t1_late_block",
                "reason": "missing_confirm_before_entry",
                "market": market.slug,
                "seconds_left": seconds_left,
                "up_ask": up_ask,
                "dn_ask": dn_ask,
                "ts": now,
            }))
            .await?;
            return Ok(());
        }
        if side != st.confirm1_side || side != st.confirm2_side {
            self.signal(json!({
                "phase": "t1_late_block",
                "reason": "entry_side_changed",
                "market": market.slug,
                "entry_side": side,
                "confirm1_side": st.confirm1_side,
                "confirm2_side": st.confirm2_side,
                "seconds_left": seconds_left,
                "up_ask": up_ask,
                "dn_ask": dn_ask,
                "ts": now,
            }))
            .await?;
            return Ok(());
        }
        if main_ask < self.cfg.entry_min_ask || opp_ask > self.cfg.opp_max_ask {
            self.signal(json!({
                "phase": "t1_late_block",
                "reason": "entry_filter",
                "market": market.slug,
                "entry_side": side,
                "entry_ask": main_ask,
                "opp_side": opp_side,
                "opp_ask": opp_ask,
                "seconds_left": seconds_left,
                "ts": now,
            }))
            .await?;
            return Ok(());
        }

        let equity = (self.cfg.start_equity + self.state.realized_pnl()).max(0.0);
        let mut max_deploy = f64::INFINITY;
        if self.cfg.risk_fraction > 0.0 {
            max_deploy = max_deploy.min(equity * self.cfg.risk_fraction);
        }
        if self.cfg.max_deploy_usdc > 0.0 {
            max_deploy = max_deploy.min(self.cfg.max_deploy_usdc);
        }
        if !max_deploy.is_finite() {
            max_deploy = self.cfg.target_qty * full_cost_per_share(main_ask);
        }

        let cost_per_share = full_cost_per_share(main_ask);
        let planned = self
            .cfg
            .target_qty
            .min(ask_size.floor())
            .min((max_deploy / cost_per_share).floor());
        let planned_cost = planned * cost_per_share;
        if planned < 1.0 || planned_cost < 1.0 {
            self.signal(json!({
                "phase": "t1_late_block",
                "reason": "planned_order_too_small",
                "market": market.slug,
                "entry_side": side,
                "entry_ask": main_ask,
                "ask_size": ask_size,
                "equity": equity,
                "max_deploy": max_deploy,
                "planned_shares": planned,
                "planned_cost": planned_cost,
                "seconds_left": seconds_left,
                "ts": now,
            }))
            .await?;
            return Ok(());
        }

        self.signal(json!({
            "phase": "intent",
            "label": "t1_late_entry",
            "market": market.slug,
            "direction": side,
            "price": main_ask,
            "shares": planned,
            "ask_size": ask_size,
            "equity": equity,
            "max_deploy": max_deploy,
            "planned_cost": planned_cost,
            "confirm1_seen_at": st.confirm1_seen_at,
            "confirm1_ask": st.confirm1_ask,
            "confirm2_seen_at": st.confirm2_seen_at,
            "confirm2_ask": st.confirm2_ask,
            "seconds_left": seconds_left,
            "mode": "dry_run_fak",
            "ts": now,
        }))
        .await?;

        let token = market
            .token_for(side)
            .ok_or_else(|| anyhow!("missing token for side {side}"))?;
        let fill = self
            .executor
            .buy_fak(token, main_ask, planned, Some(main_ask))
            .await?;
        self.signal(json!({
            "phase": "submit",
            "label": "t1_late_entry",
            "market": market.slug,
            "direction": side,
            "order_id": fill.order_id,
            "status": fill.status,
            "success": fill.success,
            "simulated": fill.simulated,
            "filled_price": fill.filled_price,
            "filled_shares": fill.filled_shares,
            "ts": Utc::now().timestamp(),
        }))
        .await?;

        if !fill.success || fill.filled_shares <= 0.0 {
            warn!("T1 FAK produced no fill for {}", market.slug);
            return Ok(());
        }

        let filled_price = fill.filled_price;
        let filled_shares = fill.filled_shares;
        let filled_cost_per_share = full_cost_per_share(filled_price);
        let filled_cost = filled_shares * filled_cost_per_share;
        self.state.trades.push(Trade {
            market: market.slug.clone(),
            end_ts: market.end_ts,
            side: side.to_string(),
            price: filled_price,
            shares: filled_shares,
            cost: filled_cost,
            ts: Utc::now().timestamp(),
            winner: None,
            pnl: None,
        });
        self.state.save(&self.cfg.state_file).await?;
        self.signal(json!({
            "phase": "t1_late_entry",
            "market": market.slug,
            "direction": side,
            "price": filled_price,
            "shares": filled_shares,
            "full_cost": filled_cost_per_share,
            "total_cost": filled_cost,
            "dry_run": self.cfg.dry_run,
            "ts": Utc::now().timestamp(),
        }))
        .await?;
        info!(
            "T1 entry {} {} @ {:.3} x {:.0} cost {:.2} dry_run={}",
            market.slug, side, filled_price, filled_shares, filled_cost, self.cfg.dry_run
        );
        Ok(())
    }

    async fn check_settlements(&mut self) -> Result<()> {
        let pending = self.state.pending_markets();
        for slug in pending {
            let Some(winner) = self.client.fetch_winner(&slug).await else {
                continue;
            };
            let mut changed = false;
            let mut settled = Vec::new();
            for trade in self
                .state
                .trades
                .iter_mut()
                .filter(|t| t.market == slug && t.pnl.is_none())
            {
                let pnl = if trade.side == winner {
                    trade.shares - trade.cost
                } else {
                    -trade.cost
                };
                trade.winner = Some(winner.clone());
                trade.pnl = Some(pnl);
                changed = true;
                settled.push(json!({
                    "phase": "settled",
                    "market": slug,
                    "winner": winner,
                    "side": trade.side,
                    "shares": trade.shares,
                    "cost": trade.cost,
                    "pnl": pnl,
                    "ts": Utc::now().timestamp(),
                }));
            }
            if changed {
                let equity = self.cfg.start_equity + self.state.realized_pnl();
                for mut rec in settled {
                    if let Some(obj) = rec.as_object_mut() {
                        obj.insert("equity".to_string(), json!(equity));
                    }
                    self.signal(rec).await?;
                }
                self.state.save(&self.cfg.state_file).await?;
            }
        }
        Ok(())
    }

    async fn signal(&self, v: serde_json::Value) -> Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cfg.signal_file)
            .await?;
        f.write_all((serde_json::to_string(&v)? + "\n").as_bytes())
            .await?;
        Ok(())
    }
}

fn strong_side(
    up_ask: f64,
    up_size: f64,
    dn_ask: f64,
    dn_size: f64,
) -> (&'static str, f64, &'static str, f64, f64) {
    if up_ask >= dn_ask {
        ("Up", up_ask, "Down", dn_ask, up_size)
    } else {
        ("Down", dn_ask, "Up", up_ask, dn_size)
    }
}

fn taker_fee(price: f64) -> f64 {
    0.07 * price * (1.0 - price)
}

fn full_cost_per_share(price: f64) -> f64 {
    price + taker_fee(price)
}

#[derive(Debug, Clone)]
struct Fill {
    order_id: String,
    status: String,
    success: bool,
    simulated: bool,
    filled_price: f64,
    filled_shares: f64,
}

impl Fill {
    fn simulated(price: f64, shares: f64) -> Self {
        Self {
            order_id: "DRY_RUN".to_string(),
            status: "simulated".to_string(),
            success: true,
            simulated: true,
            filled_price: price,
            filled_shares: shares,
        }
    }
}

enum OrderExecutor {
    DryRun,
    Live {
        client: Box<Client<Authenticated<Normal>>>,
        signer: PrivateKeySigner,
    },
}

impl OrderExecutor {
    async fn new(cfg: &Config) -> Result<Self> {
        if cfg.dry_run {
            info!("executor DRY_RUN=1; no real orders will be sent");
            return Ok(Self::DryRun);
        }

        let pk = cfg
            .private_key
            .as_ref()
            .context("DRY_RUN=0 requires PRIVATE_KEY")?;
        let signer = PrivateKeySigner::from_str(pk.trim())
            .context("PRIVATE_KEY parse failed")?
            .with_chain_id(Some(POLYGON));
        let builder = Client::new(&cfg.clob_v2_api_url, ClobConfig::default())
            .context("create CLOB V2 client failed")?
            .authentication_builder(&signer);

        let client = if let Some(dw) = &cfg.deposit_wallet {
            let funder =
                Address::from_str(dw.trim()).context("DEPOSIT_WALLET_ADDRESS parse failed")?;
            builder
                .funder(funder)
                .signature_type(map_sig_type(cfg.signature_type))
                .authenticate()
                .await
                .context("CLOB authentication failed")?
        } else {
            builder
                .authenticate()
                .await
                .context("CLOB authentication failed")?
        };
        info!("executor LIVE ready; CLOB API creds authenticated");
        let _ = client.version().await;
        Ok(Self::Live {
            client: Box::new(client),
            signer,
        })
    }

    async fn buy_fak(
        &self,
        token_id: &str,
        price: f64,
        shares: f64,
        limit_price: Option<f64>,
    ) -> Result<Fill> {
        let order_shares = normalize_order_shares(shares)?;
        match self {
            Self::DryRun => Ok(Fill::simulated(price, order_shares)),
            Self::Live { client, signer } => {
                let tid = U256::from_str(token_id)
                    .with_context(|| format!("token_id parse failed: {token_id}"))?;
                let s = SdkDecimal::from_str(&format!("{order_shares:.0}"))?;
                let amount = Amount::shares(s).context("share amount conversion failed")?;
                let price_cap = limit_price
                    .map(|x| SdkDecimal::from_str(&clob_price_string(x.clamp(0.01, 0.99))))
                    .transpose()
                    .context("limit price conversion failed")?;

                let t_build = std::time::Instant::now();
                let mut builder = client
                    .market_order()
                    .token_id(tid)
                    .side(Side::Buy)
                    .amount(amount)
                    .order_type(OrderType::FAK);
                if let Some(p) = price_cap {
                    builder = builder.price(p);
                }
                let order = builder.build().await.context("build FAK order failed")?;
                let build_ms = t_build.elapsed().as_millis();

                let t_sign = std::time::Instant::now();
                let signed = client
                    .sign(signer, order)
                    .await
                    .context("sign FAK order failed")?;
                let sign_ms = t_sign.elapsed().as_millis();

                let t_post = std::time::Instant::now();
                let resp_result = client.post_order(signed).await;
                let post_ms = t_post.elapsed().as_millis();
                info!(
                    "order latency build={}ms sign={}ms post={}ms",
                    build_ms, sign_ms, post_ms
                );

                let resp = match resp_result {
                    Ok(r) => r,
                    Err(e) if e.to_string().contains("no orders found to match") => {
                        warn!("FAK unmatched; no fill");
                        return Ok(Fill {
                            order_id: String::new(),
                            status: "unmatched".to_string(),
                            success: false,
                            simulated: false,
                            filled_price: price,
                            filled_shares: 0.0,
                        });
                    }
                    Err(e) => return Err(e).context("post FAK order failed"),
                };

                let making = resp.making_amount.to_string().parse::<f64>().unwrap_or(0.0);
                let taking = resp.taking_amount.to_string().parse::<f64>().unwrap_or(0.0);
                if taking <= 0.0 {
                    return Ok(Fill {
                        order_id: resp.order_id,
                        status: resp.status.to_string(),
                        success: false,
                        simulated: false,
                        filled_price: price,
                        filled_shares: 0.0,
                    });
                }
                Ok(Fill {
                    order_id: resp.order_id,
                    status: resp.status.to_string(),
                    success: true,
                    simulated: false,
                    filled_price: making / taking,
                    filled_shares: taking,
                })
            }
        }
    }
}

fn normalize_order_shares(shares: f64) -> Result<f64> {
    if !shares.is_finite() || shares <= 0.0 {
        bail!("invalid shares: {shares}");
    }
    Ok(shares.round().max(1.0))
}

fn clob_price_string(price: f64) -> String {
    let mut s = format!("{price:.4}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    s
}

fn map_sig_type(t: u8) -> SignatureType {
    match t {
        0 => SignatureType::Eoa,
        1 => SignatureType::Proxy,
        2 => SignatureType::GnosisSafe,
        _ => SignatureType::Poly1271,
    }
}
