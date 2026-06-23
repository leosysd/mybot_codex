#!/usr/bin/env bash
# Install mybot-codex as a systemd dry-run service.
set -euo pipefail

DATA_DIR="${1:-/opt/mybot-codex}"
SERVICE="${2:-mybot-codex}"
BIN="${3:-/usr/local/bin/mybot-codex}"
ENV_FILE="$DATA_DIR/.env"

if [ ! -x "$BIN" ]; then
  echo "missing executable: $BIN" >&2
  exit 1
fi

mkdir -p "$DATA_DIR/data"
chmod 700 "$DATA_DIR"

if [ ! -f "$ENV_FILE" ]; then
  cat > "$ENV_FILE" <<'ENVEOF'
DRY_RUN=1
STRATEGY=btc_distance_ladder
PRIVATE_KEY=
DEPOSIT_WALLET_ADDRESS=
SIGNATURE_TYPE=3
CLOB_API_URL=https://clob.polymarket.com
CLOB_V2_API_URL=https://clob.polymarket.com
GAMMA_API_URL=https://gamma-api.polymarket.com
MARKET_WS_URL=wss://ws-subscriptions-clob.polymarket.com/ws/market
MARKET_SLUG_PREFIX=btc-updown-5m
BTC_PRICE_WS_URL=wss://ws-live-data.polymarket.com
BTC_PRICE_TOPIC=crypto_prices_chainlink
BTC_PRICE_TYPE=*
BTC_PRICE_FILTERS={"symbol":"btc/usd"}
BTC_PRICE_SYMBOL=btc/usd
BTC_PRICE_MAX_AGE_MS=3000
DATA_DIR=/opt/mybot-codex/data
SIGNAL_FILE=/opt/mybot-codex/data/t1_late_signals.jsonl
STATE_FILE=/opt/mybot-codex/data/t1_late_state.json
POLL_MS=200
T1_LATE_CONFIRM1_SECS=10
T1_LATE_CONFIRM2_SECS=8
T1_LATE_ENTRY_SECS=1
T1_LATE_CONFIRM_MIN_ASK=0.98
T1_LATE_ENTRY_MIN_ASK=0.75
T1_LATE_OPP_MAX_ASK=0.30
T1_LATE_TARGET_QTY=5000
T1_LATE_START_EQUITY=300
T1_LATE_RISK_FRACTION=0.2
T1_LATE_MAX_DEPLOY_USDC=500
BTC_DISTANCE_TAIL_MAX_SECS=2
BTC_DISTANCE_START_CAPTURE_MIN_SECS=295
BTC_DISTANCE_MAX_ASK=0.99
BTC_DISTANCE_MAX_SPREAD=0.10
BTC_DISTANCE_EXCLUDE_ASK_LOW=0.85
BTC_DISTANCE_EXCLUDE_ASK_HIGH=0.90
BTC_DISTANCE_MIN_ABS_BPS=0
BTC_LADDER_EARLY_SECS=60
BTC_LADDER_EARLY_BUDGET_FRAC=0.05
BTC_LADDER_EARLY_MIN_ABS_BPS=10
BTC_LADDER_EARLY_MIN_SCORE=2.5
BTC_LADDER_EARLY_MAX_ASK=0.95
BTC_LADDER_EARLY_MAX_SPREAD=0.10
BTC_LADDER_MID_SECS=5
BTC_LADDER_MID_BUDGET_FRAC=0.40
BTC_LADDER_MID_MIN_ABS_BPS=1
BTC_LADDER_MID_MIN_SCORE=0.5
BTC_LADDER_MID_MAX_ASK=0.99
BTC_LADDER_MID_MAX_SPREAD=0.10
BTC_LADDER_MID_EXCLUDE_ASK_LOW=0.85
BTC_LADDER_MID_EXCLUDE_ASK_HIGH=0.90
BTC_LADDER_TAIL_SECS=2
BTC_LADDER_TAIL_BUDGET_FRAC=1.0
BTC_LADDER_TAIL_MIN_ABS_BPS=0
BTC_LADDER_TAIL_MIN_SCORE=0
BTC_LADDER_TAIL_MAX_ASK=0.98
BTC_LADDER_TAIL_MAX_SPREAD=0.10
BTC_LADDER_TAIL_EXCLUDE_ASK_LOW=0.85
BTC_LADDER_TAIL_EXCLUDE_ASK_HIGH=0.90
REST_FALLBACK_TIMEOUT_MS=700
ENVEOF
  chmod 600 "$ENV_FILE"
fi

ensure_env() {
  local key="$1"
  local value="$2"
  if ! grep -q "^${key}=" "$ENV_FILE"; then
    printf '%s=%s\n' "$key" "$value" >> "$ENV_FILE"
  fi
}

ensure_env STRATEGY btc_distance_ladder
ensure_env BTC_PRICE_WS_URL wss://ws-live-data.polymarket.com
ensure_env BTC_PRICE_TOPIC crypto_prices_chainlink
ensure_env BTC_PRICE_TYPE '*'
ensure_env BTC_PRICE_FILTERS '{"symbol":"btc/usd"}'
ensure_env BTC_PRICE_SYMBOL btc/usd
ensure_env BTC_PRICE_MAX_AGE_MS 3000
ensure_env BTC_DISTANCE_TAIL_MAX_SECS 2
ensure_env BTC_DISTANCE_START_CAPTURE_MIN_SECS 295
ensure_env BTC_DISTANCE_MAX_ASK 0.99
ensure_env BTC_DISTANCE_MAX_SPREAD 0.10
ensure_env BTC_DISTANCE_EXCLUDE_ASK_LOW 0.85
ensure_env BTC_DISTANCE_EXCLUDE_ASK_HIGH 0.90
ensure_env BTC_DISTANCE_MIN_ABS_BPS 0
ensure_env BTC_LADDER_EARLY_SECS 60
ensure_env BTC_LADDER_EARLY_BUDGET_FRAC 0.05
ensure_env BTC_LADDER_EARLY_MIN_ABS_BPS 10
ensure_env BTC_LADDER_EARLY_MIN_SCORE 2.5
ensure_env BTC_LADDER_EARLY_MAX_ASK 0.95
ensure_env BTC_LADDER_EARLY_MAX_SPREAD 0.10
ensure_env BTC_LADDER_MID_SECS 5
ensure_env BTC_LADDER_MID_BUDGET_FRAC 0.40
ensure_env BTC_LADDER_MID_MIN_ABS_BPS 1
ensure_env BTC_LADDER_MID_MIN_SCORE 0.5
ensure_env BTC_LADDER_MID_MAX_ASK 0.99
ensure_env BTC_LADDER_MID_MAX_SPREAD 0.10
ensure_env BTC_LADDER_MID_EXCLUDE_ASK_LOW 0.85
ensure_env BTC_LADDER_MID_EXCLUDE_ASK_HIGH 0.90
ensure_env BTC_LADDER_TAIL_SECS 2
ensure_env BTC_LADDER_TAIL_BUDGET_FRAC 1.0
ensure_env BTC_LADDER_TAIL_MIN_ABS_BPS 0
ensure_env BTC_LADDER_TAIL_MIN_SCORE 0
ensure_env BTC_LADDER_TAIL_MAX_ASK 0.98
ensure_env BTC_LADDER_TAIL_MAX_SPREAD 0.10
ensure_env BTC_LADDER_TAIL_EXCLUDE_ASK_LOW 0.85
ensure_env BTC_LADDER_TAIL_EXCLUDE_ASK_HIGH 0.90

cat > "/etc/systemd/system/${SERVICE}.service" <<EOF
[Unit]
Description=mybot-codex BTC distance ladder dry-run bot
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=${DATA_DIR}
ExecStart=${BIN} ${ENV_FILE}
Restart=always
RestartSec=3
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "$SERVICE"
echo "installed $SERVICE with env $ENV_FILE"
