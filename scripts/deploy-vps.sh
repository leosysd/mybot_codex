#!/usr/bin/env bash
# Deploy locally built mybot-codex to the VPS as a separate service.
# This script only installs/restarts mybot-codex and leaves existing bots alone.
#
# Usage:
#   scripts/deploy-vps.sh [ssh-host]
set -euo pipefail

SSH_HOST="${1:-dubolin-vps}"
SSH_CONFIG="${SSH_CONFIG:-/mnt/c/Users/Tedy/.ssh/config}"
LOCAL_BIN="${LOCAL_BIN:-target/release/mybot-codex}"
SERVICE="mybot-codex"
DATA_DIR="/opt/mybot-codex"
REMOTE_BIN="/usr/local/bin/mybot-codex"
CONTROL_PATH="${CONTROL_PATH:-/tmp/mybot-codex-ssh-%r@%h:%p}"
SSH_OPTS=(-F "$SSH_CONFIG" -o BatchMode=yes -o ConnectTimeout=20 -o ControlMaster=auto -o ControlPersist=90 -o ControlPath="$CONTROL_PATH")

if [ ! -x "$LOCAL_BIN" ]; then
  echo "missing executable binary: $LOCAL_BIN" >&2
  echo "run: cargo build --release" >&2
  exit 1
fi

local_hash="$(sha256sum "$LOCAL_BIN" | awk '{print $1}')"
remote_tmp="/tmp/mybot-codex-${local_hash:0:12}"

echo "[1/5] local binary"
echo "  path: $LOCAL_BIN"
echo "  sha256: $local_hash"

echo "[2/5] upload binary and installer"
ssh "${SSH_OPTS[@]}" -MNf "$SSH_HOST" 2>/dev/null || true
scp "${SSH_OPTS[@]}" "$LOCAL_BIN" "$SSH_HOST:$remote_tmp"
scp "${SSH_OPTS[@]}" scripts/install-systemd.sh "$SSH_HOST:/tmp/install-mybot-codex.sh"
scp "${SSH_OPTS[@]}" scripts/jytd.py "$SSH_HOST:/tmp/jytd.py"
scp "${SSH_OPTS[@]}" scripts/jydiag.py "$SSH_HOST:/tmp/jydiag.py"

echo "[3/5] install/restart mybot-codex service"
ssh "${SSH_OPTS[@]}" "$SSH_HOST" \
  "REMOTE_TMP='$remote_tmp' REMOTE_BIN='$REMOTE_BIN' DATA_DIR='$DATA_DIR' SERVICE='$SERVICE' EXPECTED_HASH='$local_hash' bash -s" <<'REMOTE'
set -euo pipefail

actual_hash="$(sha256sum "$REMOTE_TMP" | awk '{print $1}')"
if [ "$actual_hash" != "$EXPECTED_HASH" ]; then
  echo "hash mismatch after upload" >&2
  echo "expected: $EXPECTED_HASH" >&2
  echo "actual:   $actual_hash" >&2
  exit 1
fi

stamp="$(date -u +%Y%m%d-%H%M%S)"
if [ -f "$REMOTE_BIN" ]; then
  cp -a "$REMOTE_BIN" "${REMOTE_BIN}.bak-${stamp}"
fi
install -m 755 "$REMOTE_TMP" "$REMOTE_BIN"
install -m 755 /tmp/jytd.py /usr/local/bin/jytd
install -m 755 /tmp/jydiag.py /usr/local/bin/jydiag
chmod +x /tmp/install-mybot-codex.sh
/tmp/install-mybot-codex.sh "$DATA_DIR" "$SERVICE" "$REMOTE_BIN"

set_env() {
  local key="$1"
  local value="$2"
  if grep -q "^${key}=" "$DATA_DIR/.env"; then
    sed -i "s|^${key}=.*|${key}=${value}|" "$DATA_DIR/.env"
  else
    printf '%s=%s\n' "$key" "$value" >> "$DATA_DIR/.env"
  fi
}

set_env DRY_RUN 1
set_env STRATEGY btc_oracle_fallback
set_env BTC_PRICE_FILTERS "'{\"symbol\":\"btc/usd\"}'"
set_env T1_LATE_TARGET_QTY 5000
set_env T1_LATE_START_EQUITY 300
set_env T1_LATE_RISK_FRACTION 0.2
set_env T1_LATE_MAX_DEPLOY_USDC 255
set_env BTC_ORACLE_PROFILE "label=t15_momo,sec=15,bps=5,ask=0.98,spread=0.10,frac=1,ret3=0,ret5=0.5;label=e8_strong,sec=8,bps=5,ask=0.95,minask=0.5,spread=none,frac=1;label=e8_normal,sec=8,bps=0.5,ask=0.93,minask=0.5,spread=none,frac=0.75;label=e5_strong,sec=5,bps=1.5,ask=0.98,minask=0.5,spread=0.10,frac=1;label=e5_cheap,sec=5,bps=0.2,ask=0.85,minask=0.5,spread=0.10,frac=0.75;label=e3_final,sec=3,bps=0,ask=0.95,minask=0.5,spread=none,frac=1"
set_env BTC_ORACLE_RISK_FRACTION 0.25
set_env BTC_ORACLE_MAX_DEPLOY_USDC 270
set_env BTC_ORACLE_DAILY_TAKE_PROFIT 800
set_env BTC_ORACLE_BINANCE_MOMENTUM 0
set_env BTC_ORACLE_BINANCE_WS_URL wss://fstream.binance.com/ws/btcusdt@trade
set_env BTC_ORACLE_BINANCE_RET1_MIN_BPS 0
set_env BTC_ORACLE_BINANCE_MAX_AGE_MS 1500
set_env BTC_LADDER_EARLY_SECS 60
set_env BTC_LADDER_EARLY_BUDGET_FRAC 0.05
set_env BTC_LADDER_EARLY_MIN_ABS_BPS 10
set_env BTC_LADDER_EARLY_MIN_SCORE 2.5
set_env BTC_LADDER_EARLY_MAX_ASK 0.95
set_env BTC_LADDER_EARLY_MAX_SPREAD 0.10
set_env BTC_LADDER_MID_SECS 5
set_env BTC_LADDER_MID_BUDGET_FRAC 1.0
set_env BTC_LADDER_MID_MIN_ABS_BPS 1
set_env BTC_LADDER_MID_MIN_SCORE 0.5
set_env BTC_LADDER_MID_MAX_ASK 0.99
set_env BTC_LADDER_MID_MAX_SPREAD 0.10
set_env BTC_LADDER_MID_EXCLUDE_ASK_LOW 0.85
set_env BTC_LADDER_MID_EXCLUDE_ASK_HIGH 0.90
set_env BTC_LADDER_TAIL_SECS 2
set_env BTC_LADDER_TAIL_BUDGET_FRAC 1.0
set_env BTC_LADDER_TAIL_MIN_ABS_BPS 0
set_env BTC_LADDER_TAIL_MIN_SCORE 0
set_env BTC_LADDER_TAIL_MAX_ASK 0.01
set_env BTC_LADDER_TAIL_MAX_SPREAD 0.10
set_env BTC_LADDER_TAIL_EXCLUDE_ASK_LOW 0.85
set_env BTC_LADDER_TAIL_EXCLUDE_ASK_HIGH 0.90

systemctl restart "$SERVICE"
sleep 2

echo "binary:"
sha256sum "$REMOTE_BIN"
echo "jytd:"
ls -l /usr/local/bin/jytd
echo "jydiag:"
ls -l /usr/local/bin/jydiag
echo "service:"
systemctl show "$SERVICE.service" -p ActiveState -p SubState -p MainPID -p MemoryCurrent -p MemoryPeak -p NRestarts
echo "existing jy-bot left untouched:"
systemctl show jy-bot.service -p ActiveState -p SubState -p MainPID 2>/dev/null || true
echo "safe env:"
grep -nE '^(DRY_RUN|STRATEGY|MARKET_SLUG_PREFIX|POLL_MS|BTC_PRICE_[A-Z0-9_]+|BTC_ORACLE_[A-Z0-9_]+|BTC_DISTANCE_[A-Z0-9_]+|BTC_LADDER_[A-Z0-9_]+|T1_LATE_[A-Z0-9_]+|REST_FALLBACK_TIMEOUT_MS)=' "$DATA_DIR/.env"
REMOTE

echo "[4/5] journal"
ssh "${SSH_OPTS[@]}" "$SSH_HOST" \
  "journalctl -u ${SERVICE}.service --since '3 minutes ago' --no-pager | tail -80"

echo "[5/5] recent signals"
ssh "${SSH_OPTS[@]}" "$SSH_HOST" \
  "tail -80 ${DATA_DIR}/data/t1_late_signals.jsonl 2>/dev/null || true"
