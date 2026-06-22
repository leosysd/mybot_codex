# mybot-codex

Minimal Polymarket BTC 5-minute dry-run bot for one strategy only: `t1_late`.

It runs:

- T-10 and T-8 strong-side confirmation.
- T-1 FAK-style dry-run buy of the same strong side.
- Taker fee model: `0.07 * price * (1 - price)`.
- Top-of-book size cap.
- Dry-run bankroll tracking from `T1_LATE_START_EQUITY`.
- REST `/book` fallback near the tail if the WebSocket cache is missing.
- One execution path for dry-run and live FAK buys.

It intentionally does not include accum, sniper, zscore, maker quoting, or model training.

## Run

```bash
cp .env.example .env
cargo run --release -- .env
```

Signals are written to `data/t1_late_signals.jsonl`.

On the VPS, `scripts/deploy-vps.sh` also installs:

```bash
jytd
```

`jytd` prints the same style trading stats table as the old bot, reading
`/opt/mybot-codex/data/t1_late_state.json` by default.

The default is `DRY_RUN=1`. With `DRY_RUN=0`, the bot uses the official Polymarket CLOB V2 SDK to authenticate and send FAK buy orders through the same `buy_fak` path used by dry-run simulation. Do not switch this on until dry-run tail evidence is stable.

## Current Historical Context

This project is intended to dry-run the historical candidate from the one-month Telonex BTC 5-minute backtest:

- T-10/T-8 strong side ask >= 0.98 and same side.
- T-1 same side, strong ask >= 0.75 and opposite ask <= 0.30.
- Target 2000 shares.
- `T1_LATE_RISK_FRACTION=1`, `T1_LATE_MAX_DEPLOY_USDC=0` reproduces the high-risk dry-run bankroll setting.

Historical backtest is not proof of future profit. Keep this in dry-run until the live orderbook log proves the same behavior for at least a full day.
