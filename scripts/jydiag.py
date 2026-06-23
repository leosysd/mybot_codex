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


def fmt_money(value: Any) -> str:
    try:
        return f"{float(value):+.2f}"
    except (TypeError, ValueError):
        return "-"


def pct(num: int, den: int) -> str:
    return f"{num / den * 100:.1f}%" if den else "0.0%"


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
    for trade in trades:
        strategy = str(trade.get("strategy") or "unknown")
        tier = str(trade.get("tier") or "-")
        by_strategy[strategy] += 1
        by_tier[f"{strategy}:{tier}"] += 1
        if trade.get("pnl") is not None:
            tier_pnl[f"{strategy}:{tier}"] += float(trade.get("pnl") or 0)
    print_counter("按策略/层级笔数", by_tier)
    if tier_pnl:
        print("\n按层级已结算 PnL")
        for key, value in sorted(tier_pnl.items(), key=lambda item: item[1]):
            print(f"  {key.ljust(32)} {value:+.2f}")


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
            intents_by_tier[str(row.get("tier") or "-")] += 1
            if row.get("btc_price_age_ms") is not None:
                try:
                    btc_age_values.append(float(row["btc_price_age_ms"]))
                except (TypeError, ValueError):
                    pass
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
    args = ap.parse_args()

    env_path = Path(args.env)
    env = read_env(env_path)
    state_path = resolve_path(env_path, env, "STATE_FILE", "/opt/mybot-codex/data/t1_late_state.json")
    signal_path = resolve_path(
        env_path, env, "SIGNAL_FILE", "/opt/mybot-codex/data/t1_late_signals.jsonl"
    )
    trades = load_state(state_path)
    signals = since_filter(iter_jsonl(signal_path, args.max_lines), args.hours)

    print("mybot-codex 诊断")
    print(f"  env={env_path}")
    print(f"  strategy={env.get('STRATEGY', '-')}")
    print(f"  dry_run={env.get('DRY_RUN', '-')}")
    print(f"  state={state_path}")
    print(f"  signals={signal_path}")
    print(f"  signal_rows={len(signals)} hours={args.hours:g}")
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
