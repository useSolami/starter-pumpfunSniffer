use std::collections::HashMap;
use std::sync::Arc;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::position::{Position, PositionStore, save_positions};
use crate::pumpfun::{self, TokenMeta};
use crate::send_transaction::SolamiSender;
use crate::{send_transaction, trader};

#[derive(Clone, Debug)]
pub enum FeedKind {
    Create,
    Buy,
    Sell,
}

#[derive(Clone, Debug)]
pub struct FeedItem {
    pub timestamp: i64,
    pub kind: FeedKind,
    pub symbol: String,
    pub name: String,
    pub mint: Pubkey,
    pub creator: Pubkey,
    pub bonding_curve: Pubkey,
    pub sol_amount: f64,
    pub user: Pubkey,
    pub token_supply: u64,
    pub is_mayhem_mode: bool,
}

#[derive(Clone, Debug)]
pub struct PositionView {
    pub mint: Pubkey,
    pub symbol: String,
    pub name: String,
    pub lamports_invested: u64,
    pub token_amount: u64,
    pub entry_timestamp: i64,
    pub pnl_pct: Option<f64>,
    pub current_value_sol: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Launches,
    Trades,
    Positions,
}

pub enum Action {
    NewFeedItem(FeedItem),
    GrpcConnected(bool),
    StatusMessage(String),
    SolBalanceUpdate(f64),
    PositionUpdate(PositionView),
    MetadataResolved { mint: Pubkey, meta: TokenMeta },
    BuySelected,
    SellSelected,
    RefreshPnl,
    Quit,
}

pub struct Stats {
    pub tokens_seen: usize,
    pub trades_seen: usize,
    pub grpc_connected: bool,
}
pub struct Config {
    pub grpc_endpoint: String,
    pub grpc_x_token: String,
    pub rpc_url: String,
    pub swqos_key: Option<String>,
    pub payer: Arc<Keypair>,
    pub buy_amount_lamports: u64,
    pub slippage_bps: u64,
    pub priority_fee_lamports: u64,
    pub tip_amount_lamports: u64,
}

const MAX_FEED_ITEMS: usize = 500;

pub struct App {
    pub running: bool,
    pub active_panel: Panel,
    pub paused: bool,
    pub launches: Vec<FeedItem>,
    pub trades: Vec<FeedItem>,
    pub selected_launch: usize,
    pub selected_trade: usize,
    pub positions: PositionStore,
    pub position_views: Vec<PositionView>,
    pub selected_position: usize,
    pub status_message: String,
    pub stats: Stats,
    pub sol_balance: f64,
    pub config: Arc<Config>,
    pub sender: Option<Arc<SolamiSender>>,
    pub action_tx: mpsc::UnboundedSender<Action>,
    pub meta_cache: HashMap<Pubkey, TokenMeta>,
    meta_fetching: std::collections::HashSet<Pubkey>,
}

impl App {
    pub fn new(
        config: Arc<Config>,
        positions: PositionStore,
        sender: Option<Arc<SolamiSender>>,
        action_tx: mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            running: true,
            active_panel: Panel::Launches,
            paused: false,
            launches: Vec::with_capacity(MAX_FEED_ITEMS),
            trades: Vec::with_capacity(MAX_FEED_ITEMS),
            selected_launch: 0,
            selected_trade: 0,
            positions,
            position_views: Vec::new(),
            selected_position: 0,
            status_message: "Ready".into(),
            stats: Stats {
                tokens_seen: 0,
                trades_seen: 0,
                grpc_connected: false,
            },
            sol_balance: 0.0,
            config,
            sender,
            action_tx,
            meta_cache: HashMap::new(),
            meta_fetching: std::collections::HashSet::new(),
        }
    }

    pub fn symbol_for(&self, mint: &Pubkey) -> String {
        if let Some(meta) = self.meta_cache.get(mint) {
            if !meta.symbol.is_empty() {
                return meta.symbol.clone();
            }
        }
        let s = mint.to_string();
        format!("{}...{}", &s[..4], &s[s.len() - 2..])
    }

    pub fn name_for(&self, mint: &Pubkey) -> String {
        self.meta_cache
            .get(mint)
            .map(|m| m.name.clone())
            .unwrap_or_default()
    }

    fn cache_meta_from_item(&mut self, item: &FeedItem) {
        if !item.symbol.is_empty() || !item.name.is_empty() {
            self.meta_cache.insert(
                item.mint,
                TokenMeta {
                    name: item.name.clone(),
                    symbol: item.symbol.clone(),
                },
            );
        }
    }

    fn ensure_metadata(&mut self, mint: Pubkey) {
        if self.meta_cache.contains_key(&mint) {
            return;
        }
        if !self.meta_fetching.insert(mint) {
            return; 
        }
        let config = Arc::clone(&self.config);
        let action_tx = self.action_tx.clone();
        tokio::spawn(async move {
            let rpc = RpcClient::new(config.rpc_url.clone());
            match pumpfun::fetch_token_metadata(&rpc, &mint).await {
                Ok(meta) => {
                    let _ = action_tx.send(Action::MetadataResolved { mint, meta });
                }
                Err(e) => {
                    warn!(error = %e, %mint, "failed to fetch token metadata");
                }
            }
        });
    }

    pub fn apply_metadata(&mut self, mint: Pubkey, meta: TokenMeta) {
        self.meta_cache.insert(mint, meta.clone());
        self.meta_fetching.remove(&mint);
        for item in &mut self.trades {
            if item.mint == mint && item.symbol.is_empty() {
                item.symbol = meta.symbol.clone();
                item.name = meta.name.clone();
            }
        }
        for pv in &mut self.position_views {
            if pv.mint == mint {
                pv.symbol = meta.symbol.clone();
                pv.name = meta.name.clone();
            }
        }
        let positions = self.positions.clone();
        let meta2 = meta.clone();
        tokio::spawn(async move {
            let mut store = positions.write().await;
            if let Some(p) = store.get_mut(&mint) {
                p.symbol = meta2.symbol;
                p.name = meta2.name;
            }
        });
    }


    pub fn refresh_sol_balance(&self) {
        let config = Arc::clone(&self.config);
        let action_tx = self.action_tx.clone();
        tokio::spawn(async move {
            let rpc = RpcClient::new(config.rpc_url.clone());
            match rpc.get_balance(&config.payer.pubkey()).await {
                Ok(lamports) => {
                    let sol = lamports as f64 / 1_000_000_000.0;
                    let _ = action_tx.send(Action::SolBalanceUpdate(sol));
                }
                Err(e) => {
                    warn!(error = %e, "failed to fetch SOL balance");
                }
            }
        });
    }

    pub fn push_feed_item(&mut self, item: FeedItem) {
        if self.paused {
            return;
        }
        self.cache_meta_from_item(&item);
        match item.kind {
            FeedKind::Create => {
                self.stats.tokens_seen += 1;
                self.launches.insert(0, item);
                if self.launches.len() > MAX_FEED_ITEMS {
                    self.launches.pop();
                }
            }
            FeedKind::Buy | FeedKind::Sell => {
                self.stats.trades_seen += 1;
                let mint = item.mint;
                if item.symbol.is_empty() {
                    if let Some(meta) = self.meta_cache.get(&mint) {
                        let mut item = item;
                        item.symbol = meta.symbol.clone();
                        item.name = meta.name.clone();
                        self.trades.insert(0, item);
                    } else {
                        self.ensure_metadata(mint);
                        self.trades.insert(0, item);
                    }
                } else {
                    self.trades.insert(0, item);
                }
                if self.trades.len() > MAX_FEED_ITEMS {
                    self.trades.pop();
                }
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.status_message = if self.paused {
            "PAUSED - feed frozen".into()
        } else {
            "Resumed".into()
        };
    }

    pub fn move_up(&mut self) {
        match self.active_panel {
            Panel::Launches => {
                if self.selected_launch > 0 {
                    self.selected_launch -= 1;
                }
            }
            Panel::Trades => {
                if self.selected_trade > 0 {
                    self.selected_trade -= 1;
                }
            }
            Panel::Positions => {
                if self.selected_position > 0 {
                    self.selected_position -= 1;
                }
            }
        }
    }

    pub fn move_down(&mut self) {
        match self.active_panel {
            Panel::Launches => {
                if !self.launches.is_empty() {
                    self.selected_launch =
                        (self.selected_launch + 1).min(self.launches.len().saturating_sub(1));
                }
            }
            Panel::Trades => {
                if !self.trades.is_empty() {
                    self.selected_trade =
                        (self.selected_trade + 1).min(self.trades.len().saturating_sub(1));
                }
            }
            Panel::Positions => {
                if !self.position_views.is_empty() {
                    self.selected_position = (self.selected_position + 1)
                        .min(self.position_views.len().saturating_sub(1));
                }
            }
        }
    }

    pub fn next_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Launches => Panel::Trades,
            Panel::Trades => Panel::Positions,
            Panel::Positions => Panel::Launches,
        };
    }

    pub async fn refresh_position_views(&mut self) {
        let positions_snapshot: Vec<Position> = {
            let store = self.positions.read().await;
            store.values().cloned().collect()
        };

        for p in &positions_snapshot {
            if !p.symbol.is_empty() && !self.meta_cache.contains_key(&p.mint) {
                self.meta_cache.insert(
                    p.mint,
                    TokenMeta {
                        name: p.name.clone(),
                        symbol: p.symbol.clone(),
                    },
                );
            }
        }

        let old_views: HashMap<Pubkey, &PositionView> =
            self.position_views.iter().map(|v| (v.mint, v)).collect();

        self.position_views = positions_snapshot
            .iter()
            .map(|p| {
                let old = old_views.get(&p.mint);
                let symbol = if let Some(meta) = self.meta_cache.get(&p.mint) {
                    if !meta.symbol.is_empty() {
                        meta.symbol.clone()
                    } else if !p.symbol.is_empty() {
                        p.symbol.clone()
                    } else {
                        let s = p.mint.to_string();
                        format!("{}...{}", &s[..4], &s[s.len() - 2..])
                    }
                } else if !p.symbol.is_empty() {
                    p.symbol.clone()
                } else {
                    let s = p.mint.to_string();
                    format!("{}...{}", &s[..4], &s[s.len() - 2..])
                };
                let name = self
                    .meta_cache
                    .get(&p.mint)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| p.name.clone());

                PositionView {
                    mint: p.mint,
                    symbol,
                    name,
                    lamports_invested: p.lamports_invested,
                    token_amount: p.token_amount,
                    entry_timestamp: p.entry_timestamp,
                    pnl_pct: old.and_then(|o| o.pnl_pct),
                    current_value_sol: old.and_then(|o| o.current_value_sol),
                }
            })
            .collect();
        let mints_needing_meta: Vec<Pubkey> = self
            .position_views
            .iter()
            .filter(|pv| !self.meta_cache.contains_key(&pv.mint))
            .map(|pv| pv.mint)
            .collect();
        for mint in mints_needing_meta {
            self.ensure_metadata(mint);
        }

        self.selected_position = self
            .selected_position
            .min(self.position_views.len().saturating_sub(1));
    }

    pub fn apply_position_update(&mut self, update: PositionView) {
        if let Some(pv) = self.position_views.iter_mut().find(|v| v.mint == update.mint) {
            pv.token_amount = update.token_amount;
            pv.pnl_pct = update.pnl_pct;
            pv.current_value_sol = update.current_value_sol;
        }
    }
    fn selected_feed_item(&self) -> Option<&FeedItem> {
        match self.active_panel {
            Panel::Launches => self.launches.get(self.selected_launch),
            Panel::Trades => self.trades.get(self.selected_trade),
            Panel::Positions => None,
        }
    }

    pub fn handle_buy_selected(&mut self) {
        let item = match self.selected_feed_item() {
            Some(item) => item,
            None => {
                self.status_message = "Select a token in Launches or Trades panel".into();
                return;
            }
        };
        let mint = item.mint;
        let creator = item.creator;
        let bonding_curve = item.bonding_curve;
        let symbol = self.symbol_for(&mint);
        let name = self.name_for(&mint);

        self.status_message = format!("Buying {}...", symbol);

        let positions = self.positions.clone();
        let config = Arc::clone(&self.config);
        let sender = self.sender.clone();
        let action_tx = self.action_tx.clone();
        let sym = symbol.clone();
        let nm = name.clone();

        tokio::spawn(async move {
            {
                let mut store = positions.write().await;
                if store.contains_key(&mint) {
                    let _ = action_tx.send(Action::StatusMessage(format!(
                        "Already holding {}",
                        sym
                    )));
                    return;
                }
                store.insert(
                    mint,
                    Position {
                        mint,
                        bonding_curve,
                        creator,
                        lamports_invested: config.buy_amount_lamports,
                        token_amount: 0,
                        entry_timestamp: chrono::Utc::now().timestamp(),
                        symbol: sym.clone(),
                        name: nm.clone(),
                    },
                );
            }

            let rpc = RpcClient::new(config.rpc_url.clone());
            match trader::buy_token(
                &rpc,
                sender.as_deref(),
                &config.payer,
                &mint,
                &creator,
                config.buy_amount_lamports,
                config.slippage_bps,
                config.priority_fee_lamports,
                config.tip_amount_lamports,
            )
            .await
            {
                Ok((sig, token_amount)) => {
                    info!(%sig, %mint, token_amount, "buy tx sent");
                    let _ = action_tx.send(Action::StatusMessage(format!(
                        "Buy {} sent: {}",
                        sym,
                        &sig.to_string()[..8]
                    )));
                    {
                        let mut store = positions.write().await;
                        let entry_timestamp = store
                            .get(&mint)
                            .map(|p| p.entry_timestamp)
                            .unwrap_or_else(|| chrono::Utc::now().timestamp());
                        store.insert(
                            mint,
                            Position {
                                mint,
                                bonding_curve,
                                creator,
                                lamports_invested: config.buy_amount_lamports,
                                token_amount,
                                entry_timestamp,
                                symbol: sym.clone(),
                                name: nm.clone(),
                            },
                        );
                        save_positions(&store);
                    }

                    let positions_poll = positions.clone();
                    let action_tx2 = action_tx.clone();
                    let sym2 = sym.clone();
                    let config2 = Arc::clone(&config);
                    tokio::spawn(async move {
                        match send_transaction::poll_confirmation(
                            &rpc,
                            sig,
                            std::time::Duration::from_secs(30),
                        )
                        .await
                        {
                            Ok(()) => {
                                info!(%sig, %mint, "buy confirmed");
                                let _ = action_tx2.send(Action::StatusMessage(format!(
                                    "Buy {} confirmed!",
                                    sym2
                                )));
                                refresh_sol_balance_task(&config2, &action_tx2).await;
                            }
                            Err(e) => {
                                warn!(error = %e, %sig, %mint, "buy confirmation poll failed (tx may still land)");
                                let _ = action_tx2.send(Action::StatusMessage(format!(
                                    "Buy {} unconfirmed: {} (press R to check)",
                                    sym2, e
                                )));
                                refresh_sol_balance_task(&config2, &action_tx2).await;
                            }
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, %mint, "buy failed");
                    let _ = action_tx.send(Action::StatusMessage(format!(
                        "Buy {} failed: {}",
                        sym, e
                    )));
                    let mut store = positions.write().await;
                    store.remove(&mint);
                    save_positions(&store);
                }
            }
        });
    }

    pub fn handle_sell_selected(&mut self) {
        if self.position_views.is_empty() {
            self.status_message = "No position selected".into();
            return;
        }
        let view = &self.position_views[self.selected_position];
        let mint = view.mint;
        let symbol = view.symbol.clone();

        self.status_message = format!("Selling {}...", symbol);

        let positions = self.positions.clone();
        let config = Arc::clone(&self.config);
        let sender = self.sender.clone();
        let action_tx = self.action_tx.clone();

        tokio::spawn(async move {
            let position = {
                let store = positions.read().await;
                match store.get(&mint) {
                    Some(p) if p.token_amount > 0 => p.clone(),
                    _ => {
                        let _ = action_tx.send(Action::StatusMessage(format!(
                            "Sell {} skipped: buy still pending",
                            symbol
                        )));
                        return;
                    }
                }
            };

            let rpc = RpcClient::new(config.rpc_url.clone());
            match trader::sell_token(
                &rpc,
                sender.as_deref(),
                &config.payer,
                &mint,
                &position.creator,
                position.token_amount,
                config.slippage_bps,
                config.priority_fee_lamports,
                config.tip_amount_lamports,
            )
            .await
            {
                Ok(sig) => {
                    info!(%sig, %mint, "sell successful");
                    let _ = action_tx.send(Action::StatusMessage(format!(
                        "Sold {}: {}",
                        symbol,
                        &sig.to_string()[..8]
                    )));
                    {
                        let mut store = positions.write().await;
                        store.remove(&mint);
                        save_positions(&store);
                    }
                    refresh_sol_balance_task(&config, &action_tx).await;
                }
                Err(e) => {
                    error!(error = %e, %mint, "sell failed");
                    let _ = action_tx.send(Action::StatusMessage(format!(
                        "Sell {} failed: {}",
                        symbol, e
                    )));
                }
            }
        });
    }
    pub fn handle_refresh_pnl(&mut self) {
        self.status_message = "Refreshing positions...".into();
        let positions = self.positions.clone();
        let config = Arc::clone(&self.config);
        let action_tx = self.action_tx.clone();
        let meta_cache = self.meta_cache.clone();

        tokio::spawn(async move {
            let rpc = RpcClient::new(config.rpc_url.clone());
            let snapshot: Vec<Position> = {
                let store = positions.read().await;
                store.values().cloned().collect()
            };

            for position in snapshot {
                let mint = position.mint;
                let ata = get_associated_token_address_with_program_id(
                    &config.payer.pubkey(),
                    &mint,
                    &pumpfun::TOKEN_PROGRAM_2022,
                );
                let real_token_amount: u64 = match rpc.get_token_account_balance(&ata).await {
                    Ok(balance) => balance.amount.parse().unwrap_or(0),
                    Err(_) => 0,
                };
                if real_token_amount != position.token_amount && real_token_amount > 0 {
                    let mut store = positions.write().await;
                    if let Some(p) = store.get_mut(&mint) {
                        p.token_amount = real_token_amount;
                    }
                }
                if real_token_amount == 0 && position.token_amount > 0 {
                    info!(%mint, "no tokens held on-chain, removing position");
                    let mut store = positions.write().await;
                    store.remove(&mint);
                    save_positions(&store);
                    continue;
                }

                if real_token_amount == 0 {
                    continue;
                }
                let (pnl_pct, current_value_sol) =
                    match pumpfun::fetch_bonding_curve(&rpc, &mint).await {
                        Ok(curve) => match curve.get_sell_price(real_token_amount) {
                            Ok(sell_value) => {
                                let pnl = ((sell_value as f64)
                                    - (position.lamports_invested as f64))
                                    / (position.lamports_invested as f64)
                                    * 100.0;
                                let val = sell_value as f64 / 1_000_000_000.0;
                                (Some(pnl), Some(val))
                            }
                            Err(_) => (None, None),
                        },
                        Err(e) => {
                            warn!(error = %e, %mint, "failed to fetch bonding curve for P&L");
                            (None, None)
                        }
                    };
                if !meta_cache.contains_key(&mint) {
                    if let Ok(meta) = pumpfun::fetch_token_metadata(&rpc, &mint).await {
                        let _ =
                            action_tx.send(Action::MetadataResolved { mint, meta: meta.clone() });
                    }
                }

                let symbol = meta_cache
                    .get(&mint)
                    .map(|m| m.symbol.clone())
                    .unwrap_or_else(|| {
                        let s = mint.to_string();
                        format!("{}...{}", &s[..4], &s[s.len() - 2..])
                    });
                let name = meta_cache
                    .get(&mint)
                    .map(|m| m.name.clone())
                    .unwrap_or_default();

                let _ = action_tx.send(Action::PositionUpdate(PositionView {
                    mint,
                    symbol,
                    name,
                    lamports_invested: position.lamports_invested,
                    token_amount: real_token_amount,
                    entry_timestamp: position.entry_timestamp,
                    pnl_pct,
                    current_value_sol,
                }));
            }

            let _ = action_tx.send(Action::StatusMessage("Positions refreshed".into()));
            refresh_sol_balance_task(&config, &action_tx).await;
        });
    }

    pub async fn on_quit(&self) {
        let store = self.positions.read().await;
        save_positions(&store);
        info!(count = store.len(), "positions saved");
    }
}

async fn refresh_sol_balance_task(
    config: &Arc<Config>,
    action_tx: &mpsc::UnboundedSender<Action>,
) {
    let rpc = RpcClient::new(config.rpc_url.clone());
    match rpc.get_balance(&config.payer.pubkey()).await {
        Ok(lamports) => {
            let sol = lamports as f64 / 1_000_000_000.0;
            let _ = action_tx.send(Action::SolBalanceUpdate(sol));
        }
        Err(e) => {
            warn!(error = %e, "failed to fetch SOL balance");
        }
    }
}
