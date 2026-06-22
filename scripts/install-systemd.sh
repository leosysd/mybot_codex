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
PRIVATE_KEY=
DEPOSIT_WALLET_ADDRESS=
SIGNATURE_TYPE=3
CLOB_API_URL=https://clob.polymarket.com
CLOB_V2_API_URL=https://clob.polymarket.com
GAMMA_API_URL=https://gamma-api.polymarket.com
MARKET_WS_URL=wss://ws-subscriptions-clob.polymarket.com/ws/market
MARKET_SLUG_PREFIX=btc-updown-5m
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
T1_LATE_TARGET_QTY=2000
T1_LATE_START_EQUITY=300
T1_LATE_RISK_FRACTION=1
T1_LATE_MAX_DEPLOY_USDC=0
REST_FALLBACK_TIMEOUT_MS=700
ENVEOF
  chmod 600 "$ENV_FILE"
fi

cat > "/etc/systemd/system/${SERVICE}.service" <<EOF
[Unit]
Description=mybot-codex single-strategy T1 late dry-run bot
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

