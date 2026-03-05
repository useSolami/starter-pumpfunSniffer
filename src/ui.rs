use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use solana_sdk::signer::Signer;
use crate::app::{App, FeedKind, Panel};

pub struct Theme;

impl Theme {
    pub const BG: Color = Color::Rgb(21, 34, 56);
    pub const SURFACE: Color = Color::Rgb(30, 48, 80);
    pub const ACCENT: Color = Color::Rgb(0, 200, 255);
    pub const TEXT: Color = Color::Rgb(220, 230, 245);
    pub const TEXT_DIM: Color = Color::Rgb(100, 120, 150);
    pub const GREEN: Color = Color::Rgb(0, 255, 136);
    pub const RED: Color = Color::Rgb(255, 80, 80);
    pub const YELLOW: Color = Color::Rgb(255, 200, 0);
    pub const SELECTION: Color = Color::Rgb(40, 70, 120);
}

const LOGO: &str = r#" ███████  ██████  ██       █████  ███    ███ ██
 ██      ██    ██ ██      ██   ██ ████  ████ ██
 ███████ ██    ██ ██      ███████ ██ ████ ██ ██
      ██ ██    ██ ██      ██   ██ ██  ██  ██ ██
 ███████  ██████  ███████ ██   ██ ██      ██ ██"#;

pub fn draw(frame: &mut Frame, app: &App) {
    let size = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Theme::BG)),
        size,
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(size);

    draw_header(frame, app, chunks[0]);
    draw_body(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(50), Constraint::Min(30)])
        .split(area);

    let logo_lines: Vec<Line> = LOGO
        .lines()
        .map(|l| Line::from(Span::styled(l, Style::default().fg(Theme::ACCENT))))
        .collect();
    let logo = Paragraph::new(logo_lines).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(Theme::BG)),
    );
    frame.render_widget(logo, chunks[0]);

    let grpc_status = if app.stats.grpc_connected {
        Span::styled("connected", Style::default().fg(Theme::GREEN))
    } else {
        Span::styled("disconnected", Style::default().fg(Theme::RED))
    };

    let wallet_full = Signer::pubkey(app.config.payer.as_ref()).to_string();
    let buy_size = format!(
        "{:.4} SOL",
        app.config.buy_amount_lamports as f64 / 1_000_000_000.0
    );

    let pause_indicator = if app.paused {
        vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "|| PAUSED",
                Style::default()
                    .fg(Theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else {
        vec![]
    };

    let mut title_spans = vec![Span::styled(
        "  SNIFFER",
        Style::default()
            .fg(Theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    )];
    title_spans.extend(pause_indicator);

    let balance_color = if app.sol_balance < 0.01 { Theme::RED } else { Theme::GREEN };

    let info_lines = vec![
        Line::from(title_spans),
        Line::from(vec![
            Span::styled("  solami.fast", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" \u{2014} fast gRPC & SWQoS without breaking the bank", Style::default().fg(Theme::TEXT_DIM)),
        ]),
        Line::from(vec![
            Span::styled("  wallet: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                wallet_full,
                Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  SOL: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                format!("{:.4}", app.sol_balance),
                Style::default().fg(balance_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  gRPC: ", Style::default().fg(Theme::TEXT_DIM)),
            grpc_status,
            Span::styled(
                format!(
                    "   Launches: {}  Trades: {}",
                    app.stats.tokens_seen, app.stats.trades_seen
                ),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Buy size: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(buy_size, Style::default().fg(Theme::TEXT)),
        ]),
    ];

    let info = Paragraph::new(info_lines).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(Theme::BG)),
    );
    frame.render_widget(info, chunks[1]);
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Percentage(38),
            Constraint::Percentage(24),
        ])
        .split(area);

    draw_launches(frame, app, chunks[0]);
    draw_trades(frame, app, chunks[1]);
    draw_positions(frame, app, chunks[2]);
}

fn draw_launches(frame: &mut Frame, app: &App, area: Rect) {
    let is_active = app.active_panel == Panel::Launches;
    let border_color = if is_active { Theme::ACCENT } else { Theme::SURFACE };

    let header = Row::new(vec![
        Cell::from("Symbol").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Name").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Supply").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Creator").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Flags").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Age").style(Style::default().fg(Theme::ACCENT)),
    ])
    .height(1);

    let now = chrono::Utc::now().timestamp();

    let rows: Vec<Row> = app
        .launches
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let age = format_age(now - item.timestamp);
            let creator_short = short_pubkey(&item.creator);
            let name = truncate_str(&item.name, 16);

            let supply = if item.token_supply > 0 {
                format_compact_amount(item.token_supply / 1_000_000)
            } else {
                "-".into()
            };

            let mut flags = String::new();
            if item.is_mayhem_mode {
                flags.push_str("MH");
            }

            let style = if i == app.selected_launch && is_active {
                Style::default().bg(Theme::SELECTION).fg(Theme::TEXT)
            } else {
                Style::default().fg(Theme::TEXT)
            };

            Row::new(vec![
                Cell::from(item.symbol.clone()).style(Style::default().fg(Theme::YELLOW)),
                Cell::from(name).style(Style::default().fg(Theme::TEXT)),
                Cell::from(supply).style(Style::default().fg(Theme::TEXT_DIM)),
                Cell::from(creator_short).style(Style::default().fg(Theme::TEXT_DIM)),
                Cell::from(flags).style(Style::default().fg(Theme::YELLOW)),
                Cell::from(age).style(Style::default().fg(Theme::TEXT_DIM)),
            ])
            .style(style)
        })
        .collect();

    let title = format!(" LAUNCHES ({}) ", app.launches.len());
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Min(10),
            Constraint::Length(7),
            Constraint::Length(10),
            Constraint::Length(3),
            Constraint::Length(4),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(Theme::BG)),
    )
    .row_highlight_style(
        Style::default()
            .bg(Theme::SELECTION)
            .add_modifier(Modifier::BOLD),
    );

    let mut state = TableState::default();
    if !app.launches.is_empty() && is_active {
        state.select(Some(app.selected_launch));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_trades(frame: &mut Frame, app: &App, area: Rect) {
    let is_active = app.active_panel == Panel::Trades;
    let border_color = if is_active { Theme::ACCENT } else { Theme::SURFACE };

    let header = Row::new(vec![
        Cell::from("Side").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Token").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("SOL").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Trader").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Age").style(Style::default().fg(Theme::ACCENT)),
    ])
    .height(1);

    let now = chrono::Utc::now().timestamp();

    let rows: Vec<Row> = app
        .trades
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (side, side_color) = match item.kind {
                FeedKind::Buy => ("BUY", Theme::GREEN),
                FeedKind::Sell => ("SELL", Theme::RED),
                _ => ("?", Theme::TEXT_DIM),
            };

            let age = format_age(now - item.timestamp);
            let token_label = if !item.symbol.is_empty() {
                item.symbol.clone()
            } else {
                short_pubkey(&item.mint)
            };
            let user_short = short_pubkey(&item.user);

            let style = if i == app.selected_trade && is_active {
                Style::default().bg(Theme::SELECTION).fg(Theme::TEXT)
            } else {
                Style::default().fg(Theme::TEXT)
            };

            Row::new(vec![
                Cell::from(side).style(Style::default().fg(side_color)),
                Cell::from(token_label).style(Style::default().fg(Theme::YELLOW)),
                Cell::from(format!("{:.3}", item.sol_amount))
                    .style(Style::default().fg(Theme::TEXT)),
                Cell::from(user_short).style(Style::default().fg(Theme::TEXT_DIM)),
                Cell::from(age).style(Style::default().fg(Theme::TEXT_DIM)),
            ])
            .style(style)
        })
        .collect();

    let title = format!(" TRADES ({}) ", app.trades.len());
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(5),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(Theme::BG)),
    )
    .row_highlight_style(
        Style::default()
            .bg(Theme::SELECTION)
            .add_modifier(Modifier::BOLD),
    );

    let mut state = TableState::default();
    if !app.trades.is_empty() && is_active {
        state.select(Some(app.selected_trade));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_positions(frame: &mut Frame, app: &App, area: Rect) {
    let is_active = app.active_panel == Panel::Positions;
    let border_color = if is_active { Theme::ACCENT } else { Theme::SURFACE };

    let header = Row::new(vec![
        Cell::from("Token").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Balance").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("P&L").style(Style::default().fg(Theme::ACCENT)),
        Cell::from("Status").style(Style::default().fg(Theme::ACCENT)),
    ])
    .height(1);

    let rows: Vec<Row> = app
        .position_views
        .iter()
        .enumerate()
        .map(|(i, pv)| {
            let (pnl_str, pnl_color, arrow) = match pv.pnl_pct {
                Some(pnl) if pnl >= 0.0 => (format!("{:+.1}%", pnl), Theme::GREEN, " \u{25b2}"),
                Some(pnl) => (format!("{:+.1}%", pnl), Theme::RED, " \u{25bc}"),
                None => ("--".into(), Theme::TEXT_DIM, ""),
            };

            let balance_str = if pv.token_amount == 0 {
                "...".into()
            } else {
                format_token_amount(pv.token_amount)
            };

            let status = if pv.token_amount == 0 {
                ("pending", Theme::YELLOW)
            } else {
                ("held", Theme::GREEN)
            };

            let style = if i == app.selected_position && is_active {
                Style::default().bg(Theme::SELECTION).fg(Theme::TEXT)
            } else {
                Style::default().fg(Theme::TEXT)
            };

            let token_label = if pv.name.is_empty() {
                pv.symbol.clone()
            } else {
                let name_short = truncate_str(&pv.name, 12);
                format!("{} ({})", pv.symbol, name_short)
            };

            Row::new(vec![
                Cell::from(token_label).style(Style::default().fg(Theme::YELLOW)),
                Cell::from(balance_str).style(Style::default().fg(Theme::TEXT)),
                Cell::from(format!("{}{}", pnl_str, arrow))
                    .style(Style::default().fg(pnl_color)),
                Cell::from(status.0).style(Style::default().fg(status.1)),
            ])
            .style(style)
        })
        .collect();

    let pos_title = format!(" POSITIONS ({}) ", app.position_views.len());
    let table = Table::new(
        rows,
        [
            Constraint::Min(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title(pos_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(Theme::BG)),
    )
    .row_highlight_style(
        Style::default()
            .bg(Theme::SELECTION)
            .add_modifier(Modifier::BOLD),
    );

    let mut state = TableState::default();
    if !app.position_views.is_empty() && is_active {
        state.select(Some(app.selected_position));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(area);

    let keys = Line::from(vec![
        Span::styled(" [P]", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("ause  ", Style::default().fg(Theme::TEXT_DIM)),
        Span::styled("[B]", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("uy  ", Style::default().fg(Theme::TEXT_DIM)),
        Span::styled("[S]", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("ell  ", Style::default().fg(Theme::TEXT_DIM)),
        Span::styled("[R]", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("efresh  ", Style::default().fg(Theme::TEXT_DIM)),
        Span::styled("[Tab]", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(" switch  ", Style::default().fg(Theme::TEXT_DIM)),
        Span::styled("[Q]", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("uit  ", Style::default().fg(Theme::TEXT_DIM)),
        Span::styled("[", Style::default().fg(Theme::ACCENT)),
        Span::styled(
            "\u{2191}\u{2193}",
            Style::default()
                .fg(Theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("]", Style::default().fg(Theme::ACCENT)),
        Span::styled(" navigate", Style::default().fg(Theme::TEXT_DIM)),
    ]);

    let keys_widget = Paragraph::new(keys).style(Style::default().bg(Theme::BG));
    frame.render_widget(keys_widget, chunks[0]);

    let status_color = if app.paused { Theme::YELLOW } else { Theme::GREEN };
    let status = Line::from(vec![
        Span::styled(" Status: ", Style::default().fg(Theme::TEXT_DIM)),
        Span::styled(&app.status_message, Style::default().fg(status_color)),
    ]);
    let status_widget = Paragraph::new(status).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Theme::SURFACE))
            .style(Style::default().bg(Theme::BG)),
    );
    frame.render_widget(status_widget, chunks[1]);
}

fn short_pubkey(pk: &solana_sdk::pubkey::Pubkey) -> String {
    let s = pk.to_string();
    if s.len() > 8 {
        format!("{}..{}", &s[..4], &s[s.len() - 4..])
    } else {
        s
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count > max_chars {
        let truncated: String = s.chars().take(max_chars - 2).collect();
        format!("{}..", truncated)
    } else {
        s.to_string()
    }
}

fn format_age(secs: i64) -> String {
    if secs < 0 {
        "0s".into()
    } else if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn format_token_amount(amount: u64) -> String {
    let whole = amount / 1_000_000;
    if whole >= 1_000_000 {
        format!("{:.1}M", whole as f64 / 1_000_000.0)
    } else if whole >= 1_000 {
        format!("{:.1}K", whole as f64 / 1_000.0)
    } else if whole > 0 {
        format!("{}", whole)
    } else {
        format!("0.{:0>6}", amount)
    }
}

fn format_compact_amount(whole: u64) -> String {
    if whole >= 1_000_000_000 {
        format!("{:.1}B", whole as f64 / 1_000_000_000.0)
    } else if whole >= 1_000_000 {
        format!("{:.0}M", whole as f64 / 1_000_000.0)
    } else if whole >= 1_000 {
        format!("{:.0}K", whole as f64 / 1_000.0)
    } else {
        format!("{}", whole)
    }
}
