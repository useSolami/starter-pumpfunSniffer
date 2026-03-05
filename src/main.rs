mod app;
mod pumpfun;
mod position;
mod send_transaction;
mod trader;
mod ui;

use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterTransactions,
};

use crate::app::{Action, App, Config, FeedItem, FeedKind};
use crate::pumpfun::{PumpFunEvent, PUMPFUN};
use crate::position::{load_positions, new_position_store};
use crate::send_transaction::SolamiSender;

fn load_config() -> Config {
    let rpc_base = env::var("RPC_URL").unwrap_or("https://ams.rpc.solami.fast/sol".into());
    let rpc_key = env::var("RPC_KEY").expect("RPC_KEY is required");
    let rpc_url = format!("{}?api_key={}", rpc_base, rpc_key);

    let pk_str = env::var("PRIVATE_KEY").expect("PRIVATE_KEY is required");
    let pk_bytes = bs58::decode(&pk_str)
        .into_vec()
        .expect("invalid base58 PRIVATE_KEY");
    let payer = Keypair::try_from(pk_bytes.as_slice()).expect("invalid keypair bytes");

    let buy_sol: f64 = env::var("BUY_AMOUNT_SOL")
        .unwrap_or("0.01".into())
        .parse()
        .expect("invalid BUY_AMOUNT_SOL");

    let tip_sol: f64 = env::var("TIP_AMOUNT")
        .unwrap_or("0.001".into())
        .parse::<f64>()
        .expect("invalid TIP_AMOUNT")
        .max(0.001);

    Config {
        grpc_endpoint: env::var("GRPC_ENDPOINT")
            .unwrap_or("https://grpc-ams.solami.fast".into()),
        grpc_x_token: env::var("GRPC_X_TOKEN").expect("GRPC_X_TOKEN is required"),
        rpc_url,
        swqos_key: env::var("SWQOS_KEY").ok(),
        payer: Arc::new(payer),
        buy_amount_lamports: (buy_sol * 1_000_000_000.0) as u64,
        slippage_bps: env::var("SLIPPAGE_BPS")
            .unwrap_or("500".into())
            .parse()
            .expect("invalid SLIPPAGE_BPS"),
        priority_fee_lamports: env::var("PRIORITY_FEE_LAMPORTS")
            .unwrap_or("100000".into())
            .parse()
            .expect("invalid PRIORITY_FEE_LAMPORTS"),
        tip_amount_lamports: (tip_sol * 1_000_000_000.0) as u64,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("solami-sniffer.log")
        .ok();

    if let Some(file) = log_file {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(file)
            .with_ansi(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let config = Arc::new(load_config());
    let positions = new_position_store();
    {
        let loaded = load_positions();
        if !loaded.is_empty() {
            info!(count = loaded.len(), "restored positions from disk");
            *positions.write().await = loaded;
        }
    }

    info!(wallet = %config.payer.pubkey(), "solami sniffer starting");

    let sender: Option<Arc<SolamiSender>> = match &config.swqos_key {
        Some(key) => match SolamiSender::new(key).await {
            Ok(s) => {
                info!("solami QUIC sender ready");
                Some(Arc::new(s))
            }
            Err(e) => {
                warn!(error = %e, "failed to connect to solami SWQoS, falling back to RPC");
                None
            }
        },
        None => {
            info!("SWQOS_KEY not set, using RPC to send transactions");
            None
        }
    };

    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();
    let (feed_tx, mut feed_rx) = mpsc::unbounded_channel::<FeedItem>();

    let mut app = App::new(
        Arc::clone(&config),
        positions.clone(),
        sender.clone(),
        action_tx.clone(),
    );

    app.refresh_sol_balance();

    {
        let config = Arc::clone(&config);
        let feed_tx = feed_tx.clone();
        let action_tx = action_tx.clone();
        tokio::spawn(async move {
            let mut retry_count: u32 = 0;
            loop {
                let _ = action_tx.send(Action::GrpcConnected(false));
                match subscribe_feed(&config, &feed_tx, &action_tx).await {
                    Ok(()) => {
                        warn!("gRPC stream ended, reconnecting...");
                        retry_count = 0;
                    }
                    Err(e) => {
                        error!(error = %e, retry_count, "gRPC connection failed");
                    }
                }
                let _ = action_tx.send(Action::GrpcConnected(false));
                retry_count += 1;
                let delay = std::cmp::min(2u64.saturating_pow(retry_count), 60);
                warn!(delay_secs = delay, "gRPC retrying in {delay}s...");
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        });
    }

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    app.refresh_position_views().await;

    let tick_rate = Duration::from_millis(66);
    let mut last_refresh = std::time::Instant::now();

    while app.running {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            app.running = false;
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            app.toggle_pause();
                        }
                        KeyCode::Char('b') | KeyCode::Char('B') | KeyCode::Enter => {
                            if matches!(
                                app.active_panel,
                                app::Panel::Launches | app::Panel::Trades
                            ) {
                                app.handle_buy_selected();
                            }
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            if app.active_panel == app::Panel::Positions {
                                app.handle_sell_selected();
                            }
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            app.handle_refresh_pnl();
                        }
                        KeyCode::Char('o') | KeyCode::Char('O') => {
                            app.open_in_browser();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.move_up();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.move_down();
                        }
                        KeyCode::Tab => {
                            app.next_panel();
                        }
                        _ => {}
                    }
                }
            }
        }

        while let Ok(item) = feed_rx.try_recv() {
            app.push_feed_item(item);
        }

        while let Ok(action) = action_rx.try_recv() {
            match action {
                Action::BuySelected => app.handle_buy_selected(),
                Action::SellSelected => app.handle_sell_selected(),
                Action::RefreshPnl => app.handle_refresh_pnl(),
                Action::Quit => app.running = false,
                Action::NewFeedItem(item) => app.push_feed_item(item),
                Action::GrpcConnected(connected) => {
                    app.stats.grpc_connected = connected;
                }
                Action::StatusMessage(msg) => {
                    app.status_message = msg;
                }
                Action::SolBalanceUpdate(sol) => {
                    app.sol_balance = sol;
                }
                Action::PositionUpdate(pv) => {
                    app.apply_position_update(pv);
                }
                Action::MetadataResolved { mint, meta } => {
                    app.apply_metadata(mint, meta);
                }
            }
        }

        if last_refresh.elapsed() >= Duration::from_secs(10) {
            app.handle_refresh_pnl_silent();
            last_refresh = std::time::Instant::now();
        }

        app.refresh_position_views().await;
    }

    app.on_quit().await;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    println!("Solami Sniffer stopped. Positions saved.");

    Ok(())
}

async fn subscribe_feed(
    config: &Arc<Config>,
    feed_tx: &mpsc::UnboundedSender<FeedItem>,
    action_tx: &mpsc::UnboundedSender<Action>,
) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = &config.grpc_endpoint;
    info!(%endpoint, "connecting to geyser backend");

    let mut builder =
        GeyserGrpcClient::build_from_shared(endpoint.clone()).expect("invalid endpoint");

    if endpoint.starts_with("https") {
        builder = builder
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .expect("tls config failed");
    }

    let mut client = builder
        .x_token(Some(config.grpc_x_token.clone()))?
        .connect_timeout(Duration::from_secs(10))
        .connect()
        .await?;

    let mut transactions = HashMap::new();
    transactions.insert(
        "pumpfun_feed".to_owned(),
        SubscribeRequestFilterTransactions {
            vote: Some(false),
            failed: Some(false),
            account_include: vec![PUMPFUN.to_string()],
            account_exclude: vec![],
            account_required: vec![],
            signature: None,
        },
    );

    let request = SubscribeRequest {
        accounts: HashMap::default(),
        slots: HashMap::default(),
        transactions,
        transactions_status: HashMap::default(),
        blocks: HashMap::default(),
        blocks_meta: HashMap::default(),
        entry: HashMap::default(),
        commitment: Some(CommitmentLevel::Processed as i32),
        accounts_data_slice: vec![],
        ping: None,
        from_slot: None,
    };

    let (_sink, mut stream) = client.subscribe_with_request(Some(request)).await?;

    let _ = action_tx.send(Action::GrpcConnected(true));
    info!("subscribed to PumpFun feed, streaming all events...");

    while let Some(msg) = stream.next().await {
        match msg {
            Ok(update) => {
                if let Some(ref inner) = update.update_oneof {
                    match inner {
                        yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Transaction(tx_update) => {
                            let logs: Vec<String> = tx_update
                                .transaction
                                .as_ref()
                                .and_then(|t| t.meta.as_ref())
                                .map(|m| m.log_messages.clone())
                                .unwrap_or_default();

                            let events = pumpfun::decode_events_from_logs(&logs);
                            for ev in events {
                                let feed_item = match ev {
                                    PumpFunEvent::Create(c) => FeedItem {
                                        timestamp: c.timestamp,
                                        kind: FeedKind::Create,
                                        symbol: c.symbol.clone(),
                                        name: c.name.clone(),
                                        mint: c.mint,
                                        creator: c.creator,
                                        bonding_curve: c.bonding_curve,
                                        sol_amount: 0.0,
                                        user: c.user,
                                        token_supply: c.token_total_supply,
                                        is_mayhem_mode: c.is_mayhem_mode,
                                    },
                                    PumpFunEvent::Trade(t) => {
                                        let bc = pumpfun::get_bonding_curve_pda(&t.mint)
                                            .unwrap_or_default();
                                        FeedItem {
                                            timestamp: t.timestamp,
                                            kind: if t.is_buy {
                                                FeedKind::Buy
                                            } else {
                                                FeedKind::Sell
                                            },
                                            symbol: String::new(),
                                            name: String::new(),
                                            mint: t.mint,
                                            creator: t.creator,
                                            bonding_curve: bc,
                                            sol_amount: t.sol_amount as f64 / 1_000_000_000.0,
                                            user: t.user,
                                            token_supply: 0,
                                            is_mayhem_mode: t.mayhem_mode,
                                        }
                                    }
                                };
                                let _ = feed_tx.send(feed_item);
                            }
                        }
                        yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Ping(_) => {
                            debug!("ping");
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                return Err(e.into());
            }
        }
    }

    Ok(())
}
