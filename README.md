# Solami Sniffer

PumpFun token sniffer TUI. See every launch and trade in real-time, buy and sell manually from your terminal.

## Setup

1. Go to [solami.fast](https://solami.fast) and register
2. Generate an **RPC key** and a **SWQoS key** (copy the SWQoS key immediately, it is only shown once)
3. Clone this repo and copy the example env:

```
git clone <repo-url>
cd pumpfun-sniper-starter
cp .env.example .env
```

4. Fill in your `.env`:

```
GRPC_X_TOKEN=your-rpc-key
RPC_KEY=your-rpc-key
PRIVATE_KEY=your-wallet-base58-keypair
SWQOS_KEY=your-swqos-base58-keypair
```

5. Run:

```
cargo run
```

## Controls

| Key | Action |
|-----|--------|
| `P` | Pause/resume feed |
| `B` / `Enter` | Buy selected token |
| `S` | Sell selected position |
| `R` | Refresh positions (on-chain balance + P&L) |
| `Tab` | Switch panel (Launches / Trades / Positions) |
| `j/k` or arrows | Navigate |
| `Q` | Quit |

## Notes

- Positions are saved to `~/.solami/starter/positions.json` on quit and after each buy/sell
- Set `NO_SAVE=true` to disable position persistence
- Logs go to `solami-sniffer.log` in the current directory
- SWQoS is optional but recommended for faster transaction landing. Without it, transactions are sent via RPC
