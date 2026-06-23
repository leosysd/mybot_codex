#!/usr/bin/env python3
from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import os
import sys
from pathlib import Path
from typing import Any


DEFAULT_ENV = "/opt/mybot-codex/.env"


def read_env(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.exists():
        return values
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, val = line.split("=", 1)
        values[key.strip()] = val.split("#", 1)[0].strip().strip('"').strip("'")
    return values


def resolve_path(env_path: Path, env: dict[str, str], key: str, default: str) -> Path:
    raw = env.get(key, default)
    path = Path(raw)
    if path.is_absolute():
        return path
    return env_path.parent / path


def load_state(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    try:
        data = json.loads(path.read_text(encoding="utf-8", errors="replace") or "{}")
    except json.JSONDecodeError:
        return []
    if isinstance(data, dict) and isinstance(data.get("trades"), list):
        return data["trades"]
    if isinstance(data, list):
        return data
    return []


def iter_jsonl(path: Path, max_lines: int) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    raw_lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    if max_lines > 0:
        raw_lines = raw_lines[-max_lines:]
    rows: list[dict[str, Any]] = []
    for line in raw_lines:
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            rows.append(value)
    return rows


def since_filter(rows: list[dict[str, Any]], hours: float) -> list[dict[str, Any]]:
    if hours <= 0:
        return rows
    cutoff = int(dt.datetime.now(dt.timezone.utc).timestamp() - hours * 3600)
    return [r for r in rows if int(r.get("ts") or 0) >= cutoff]


def latest_service_start_ts(rows: list[dict[str, Any]]) -> int | None:
    starts = [int(r.get("ts") or 0) for r in rows if r.get("phase") == "service_start"]
    if not starts:
        return None
    return max(starts)


def since_ts(rows: list[dict[str, Any]], cutoff: int | None) -> list[dict[str, Any]]:
    if cutoff is None:
        return rows
    return [r for r in rows if int(r.get("ts") or 0) >= cutoff]


def trade_ts(trade: dict[str, Any]) -> int:
    for key in ("ts", "entry_ts", "created_ts"):
        try:
            value = int(trade.get(key) or 0)
        except (TypeError, ValueError):
            value = 0
        if value > 0:
            return value
    return 0


def trades_since_ts(trades: list[dict[str, Any]], cutoff: int | None) -> list[dict[str, Any]]:
    if cutoff is None:
        return trades
    return [trade for trade in trades if trade_ts(trade) >= cutoff]


def fmt_money(value: Any) -> str:
    try:
        return f"{float(value):+.2f}"
    except (TypeError, ValueError):
        return "-"


def pct(num: int, den: int) -> str:
    return f"{num / den * 100:.1f}%" if den else "0.0%"


def as_float(value: Any) -> float | None:
    try:
        num = float(value)
    except (TypeError, ValueError):
        return None
    return num if num == num else None


def quantile(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    idx = int((len(ordered) - 1) * q)
    return ordered[idx]


def print_counter(title: str, counter: collections.Counter[str], limit: int = 20) -> None:
    print(f"\n{title}")
    if not counter:
        print("  -")
        return
    width = max(len(str(k)) for k in counter)
    for key, count in counter.most_common(limit):
        print(f"  {str(key).ljust(width)}  {count}")


def summarize_trades(trades: list[dict[str, Any]]) -> None:
    settled = [t for t in trades if t.get("pnl") is not None]
    holding = [t for t in trades if t.get("pnl") is None]
    wins = [t for t in settled if float(t.get("pnl") or 0) >= 0]
    losses = [t for t in settled if float(t.get("pnl") or 0) < 0]
    net = sum(float(t.get("pnl") or 0) for t in settled)
    print("\n交易状态")
    print(f"  trades={len(trades)} settled={len(settled)} holding={len(holding)}")
    print(f"  wins={len(wins)} losses={len(losses)} win_rate={pct(len(wins), len(settled))}")
    print(f"  realized_pnl={fmt_money(net)}")

    by_strategy: collections.Counter[str] = collections.Counter()
    by_tier: collections.Counter[str] = collections.Counter()
    tier_pnl: collections.defaultdict[str, float] = collections.defaultdict(float)
    tier_bps: collections.defaultdict[str, list[float]] = collections.defaultdict(list)
    for trade in trades:
        strategy = str(trade.get("strategy") or "unknown")
        tier = str(trade.get("tier") or "-")
        key = f"{strategy}:{tier}"
        by_strategy[strategy] += 1
        by_tier[key] += 1
        if trade.get("pnl") is not None:
            tier_pnl[key] += float(trade.get("pnl") or 0)
        bps = as_float(trade.get("btc_from_start_bps"))
        if bps is not None:
            tier_bps[key].append(bps)
    print_counter("按策略/层级笔数", by_tier)
    if tier_pnl:
        print("\n按层级已结算 PnL")
        for key, value in sorted(tier_pnl.items(), key=lambda item: item[1]):
            print(f"  {key.ljust(32)} {value:+.2f}")
    if tier_bps:
        print("\n按层级 BTC 入场涨跌 bp")
        for key, values in sorted(tier_bps.items()):
            abs_values = [abs(v) for v in values]
            avg = sum(values) / len(values)
            print(
                f"  {key.ljust(32)} count={len(values)} "
                f"avg={avg:+.2f} abs_p50={quantile(abs_values, 0.50):.2f} "
                f"abs_p95={quantile(abs_values, 0.95):.2f}"
            )


def summarize_signals(rows: list[dict[str, Any]]) -> None:
    phase_counts = collections.Counter(str(r.get("phase") or "-") for r in rows)
    print_counter("signal phase 计数", phase_counts)

    block_reasons = collections.Counter()
    oracle_rejects = collections.Counter()
    book_missing_by_sec = collections.Counter()
    intents_by_tier = collections.Counter()
    submit_by_status = collections.Counter()
    entries_by_tier = collections.Counter()
    btc_age_values: list[float] = []
    btc_bps_values: list[float] = []
    btc_bps_by_tier: collections.defaultdict[str, list[float]] = collections.defaultdict(list)
    btc_ret_values: dict[str, list[float]] = {"ret3": [], "ret5": [], "ret10": []}
    for row in rows:
        phase = row.get("phase")
        if phase in {"btc_oracle_block", "btc_distance_block", "t1_late_block"}:
            block_reasons[f"{phase}:{row.get('reason') or '-'}"] += 1
        if phase == "btc_oracle_block":
            for reject in row.get("rejects") or []:
                if isinstance(reject, dict):
                    oracle_rejects[
                        f"{reject.get('tier') or '-'}:{reject.get('reason') or '-'}"
                    ] += 1
        if phase in {"btc_oracle_book_missing", "t1_late_book_missing"}:
            book_missing_by_sec[str(row.get("seconds_left") or "-")] += 1
        if phase == "intent" and row.get("label") == "btc_oracle_fallback_entry":
            tier = str(row.get("tier") or "-")
            intents_by_tier[tier] += 1
            if row.get("btc_price_age_ms") is not None:
                age = as_float(row.get("btc_price_age_ms"))
                if age is not None:
                    btc_age_values.append(age)
            bps = as_float(row.get("btc_from_start_bps"))
            if bps is not None:
                btc_bps_values.append(bps)
                btc_bps_by_tier[tier].append(bps)
            for label, field in (
                ("ret3", "btc_ret_3s_bps"),
                ("ret5", "btc_ret_5s_bps"),
                ("ret10", "btc_ret_10s_bps"),
            ):
                value = as_float(row.get(field))
                if value is not None:
                    btc_ret_values[label].append(value)
        if phase == "submit" and row.get("label") == "btc_oracle_fallback_entry":
            submit_by_status[f"{row.get('status') or '-'}:{row.get('success')}"] += 1
        if phase == "btc_oracle_fallback_entry":
            entries_by_tier[str(row.get("tier") or "-")] += 1

    print_counter("block 原因", block_reasons)
    print_counter("oracle tier reject", oracle_rejects)
    print_counter("book missing 秒点", book_missing_by_sec)
    print_counter("oracle intent by tier", intents_by_tier)
    print_counter("oracle submit status", submit_by_status)
    print_counter("oracle entry by tier", entries_by_tier)
    if btc_age_values:
        values = sorted(btc_age_values)
        p50 = values[len(values) // 2]
        p95 = values[int(len(values) * 0.95) if len(values) > 1 else 0]
        print("\nBTC tick age on intent")
        print(f"  count={len(values)} p50={p50:.0f}ms p95={p95:.0f}ms max={max(values):.0f}ms")
    if btc_bps_values:
        abs_values = [abs(v) for v in btc_bps_values]
        print("\nBTC move on oracle intent")
        print(
            f"  count={len(btc_bps_values)} "
            f"min={min(btc_bps_values):+.2f}bp max={max(btc_bps_values):+.2f}bp "
            f"abs_p50={quantile(abs_values, 0.50):.2f}bp "
            f"abs_p95={quantile(abs_values, 0.95):.2f}bp"
        )
        for tier, values in sorted(btc_bps_by_tier.items()):
            abs_tier = [abs(v) for v in values]
            print(
                f"  {tier.ljust(12)} count={len(values)} "
                f"avg={sum(values) / len(values):+.2f}bp "
                f"abs_p50={quantile(abs_tier, 0.50):.2f}bp"
            )
    if any(btc_ret_values.values()):
        print("\nBTC recent move on oracle intent")
        for label, values in btc_ret_values.items():
            if not values:
                continue
            abs_values = [abs(v) for v in values]
            print(
                f"  {label.ljust(5)} count={len(values)} "
                f"min={min(values):+.2f}bp max={max(values):+.2f}bp "
                f"abs_p50={quantile(abs_values, 0.50):.2f}bp"
            )


def print_recent(rows: list[dict[str, Any]], limit: int) -> None:
    if limit <= 0:
        return
    print(f"\n最近 {limit} 条关键信号")
    interesting = [
        r
        for r in rows
        if r.get("phase")
        in {
            "btc_oracle_block",
            "btc_oracle_fallback_entry",
            "intent",
            "submit",
            "settled",
        }
    ]
    for row in interesting[-limit:]:
        print(json.dumps(row, ensure_ascii=False, sort_keys=True))


def main() -> int:
    ap = argparse.ArgumentParser(description="Diagnose mybot-codex dry-run signals.")
    ap.add_argument("--env", default=os.environ.get("MYBOT_CODEX_ENV", DEFAULT_ENV))
    ap.add_argument("--hours", type=float, default=24.0, help="0 means all loaded signals")
    ap.add_argument("--max-lines", type=int, default=200_000)
    ap.add_argument("--recent", type=int, default=12)
    ap.add_argument(
        "--all-starts",
        action="store_true",
        help="include signals before the latest service_start marker",
    )
    args = ap.parse_args()

    env_path = Path(args.env)
    env = read_env(env_path)
    state_path = resolve_path(env_path, env, "STATE_FILE", "/opt/mybot-codex/data/t1_late_state.json")
    signal_path = resolve_path(
        env_path, env, "SIGNAL_FILE", "/opt/mybot-codex/data/t1_late_signals.jsonl"
    )
    trades = load_state(state_path)
    loaded_signals = iter_jsonl(signal_path, args.max_lines)
    service_start = None if args.all_starts else latest_service_start_ts(loaded_signals)
    signals = since_filter(loaded_signals, args.hours)
    signals = since_ts(signals, service_start)
    trades = trades_since_ts(trades, service_start)

    print("mybot-codex 诊断")
    print(f"  env={env_path}")
    print(f"  strategy={env.get('STRATEGY', '-')}")
    print(f"  dry_run={env.get('DRY_RUN', '-')}")
    print(f"  state={state_path}")
    print(f"  signals={signal_path}")
    print(f"  signal_rows={len(signals)} hours={args.hours:g} since_latest_start={not args.all_starts}")
    if service_start is not None:
        started = dt.datetime.fromtimestamp(service_start, dt.timezone.utc).astimezone(
            dt.timezone(dt.timedelta(hours=8))
        )
        print(f"  latest_service_start={started:%Y-%m-%d %H:%M:%S} BJT")
    if env.get("STRATEGY") == "btc_oracle_fallback":
        print("  oracle_profile=" + env.get("BTC_ORACLE_PROFILE", "-"))
        print(
            "  oracle_risk="
            + env.get("BTC_ORACLE_RISK_FRACTION", "-")
            + " max_deploy="
            + env.get("BTC_ORACLE_MAX_DEPLOY_USDC", "-")
            + " daily_tp="
            + env.get("BTC_ORACLE_DAILY_TAKE_PROFIT", "-")
        )

    summarize_trades(trades)
    summarize_signals(signals)
    print_recent(signals, args.recent)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
