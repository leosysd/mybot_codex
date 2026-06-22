#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import sys
import unicodedata
from pathlib import Path
from typing import Any


DEFAULT_ENV = "/opt/mybot-codex/.env"


def display_width(text: str) -> int:
    width = 0
    for ch in str(text):
        if ch == "\x1b":
            continue
        width += 2 if unicodedata.east_asian_width(ch) in {"F", "W"} else 1
    return width


def pad(text: Any, width: int) -> str:
    s = str(text)
    return s + " " * max(0, width - display_width(s))


def bj_hm(ts: int) -> str:
    return dt.datetime.fromtimestamp(ts, dt.timezone.utc).astimezone(
        dt.timezone(dt.timedelta(hours=8))
    ).strftime("%H:%M")


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


def resolve_state_path(env_path: Path, override: str | None) -> Path | str:
    if override:
        return override if override == "-" else Path(override)
    env = read_env(env_path)
    raw = env.get("STATE_FILE", "/opt/mybot-codex/data/t1_late_state.json")
    path = Path(raw)
    if path.is_absolute():
        return path
    return env_path.parent / path


def load_state(path: Path | str) -> list[dict[str, Any]]:
    if path == "-":
        text = sys.stdin.read()
    else:
        p = Path(path)
        if not p.exists():
            return []
        text = p.read_text(encoding="utf-8", errors="replace")
    if not text.strip():
        return []
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        print(f"✖ 解析失败: {exc}")
        return []
    if isinstance(data, dict) and isinstance(data.get("trades"), list):
        return data["trades"]
    if isinstance(data, list):
        return data
    print("✖ 状态文件格式不认识")
    return []


def fmt_num(value: Any, decimals: int = 2, signed: bool = False) -> str:
    try:
        num = float(value)
    except (TypeError, ValueError):
        return "-"
    sign = "+" if signed else ""
    return f"{num:{sign}.{decimals}f}"


def build_rows(trades: list[dict[str, Any]]) -> tuple[list[list[str]], dict[str, Any]]:
    trades = sorted(trades, key=lambda t: (int(t.get("end_ts") or 0), int(t.get("ts") or 0)))
    rows: list[list[str]] = []
    stats = {
        "settled": 0,
        "wins": 0,
        "losses": 0,
        "holding": 0,
        "locked": 0,
        "net": 0.0,
    }
    grouped: dict[str, list[dict[str, Any]]] = {}
    for trade in trades:
        grouped.setdefault(str(trade.get("market") or ""), []).append(trade)

    for market_trades in grouped.values():
        market_trades.sort(key=lambda t: int(t.get("ts") or 0))
        if not market_trades:
            continue
        end_ts = int(market_trades[0].get("end_ts") or 0)
        start_ts = end_ts - 300
        label = f"{bj_hm(start_ts)}~{bj_hm(end_ts)}" if end_ts else "-"
        n = len(market_trades)
        last = market_trades[-1]
        pnl = last.get("pnl")
        winner = last.get("winner")
        if pnl is None:
            stats["holding"] += 1
        else:
            stats["settled"] += 1
            pnl_f = float(pnl)
            stats["net"] += pnl_f
            if pnl_f >= 0:
                stats["wins"] += 1
            else:
                stats["losses"] += 1

        for i, trade in enumerate(market_trades):
            is_last = i == n - 1
            result = ""
            pnl_text = ""
            if is_last:
                if pnl is None:
                    result = "持仓中"
                    pnl_text = "-"
                else:
                    result = str(winner or "-")
                    pnl_text = fmt_num(pnl, signed=True)
            ts = int(trade.get("ts") or start_ts)
            rows.append(
                [
                    label if i == 0 else "",
                    str(max(0, ts - start_ts)) if end_ts else "-",
                    str(trade.get("side") or trade.get("direction") or "-").lower(),
                    fmt_num(trade.get("price"), 3),
                    fmt_num(trade.get("shares"), 0),
                    fmt_num(trade.get("cost") or trade.get("total_cost"), 2),
                    "T1尾盘",
                    result,
                    pnl_text,
                ]
            )
    return rows, stats


def render_table(headers: list[str], rows: list[list[str]]) -> None:
    widths = [display_width(h) for h in headers]
    for row in rows:
        for idx, cell in enumerate(row):
            widths[idx] = max(widths[idx], display_width(cell))

    def border(left: str, mid: str, right: str) -> str:
        return left + mid.join("─" * (w + 2) for w in widths) + right

    print(border("┌", "┬", "┐"))
    print("│ " + " │ ".join(pad(h, widths[i]) for i, h in enumerate(headers)) + " │")
    print(border("├", "┼", "┤"))
    last_label = None
    for idx, row in enumerate(rows):
        label = row[0] or last_label
        if idx > 0 and row[0] and last_label is not None and label != last_label:
            print(border("├", "┼", "┤"))
        print("│ " + " │ ".join(pad(c, widths[i]) for i, c in enumerate(row)) + " │")
        if row[0]:
            last_label = row[0]
    print(border("└", "┴", "┘"))


def main() -> int:
    ap = argparse.ArgumentParser(description="Show mybot-codex trading stats table.")
    ap.add_argument("--env", default=os.environ.get("MYBOT_CODEX_ENV", DEFAULT_ENV))
    ap.add_argument("--state", default="")
    args = ap.parse_args()

    env_path = Path(args.env)
    state_path = resolve_state_path(env_path, args.state or None)
    trades = load_state(state_path)
    if not trades:
        if state_path != "-" and not Path(state_path).exists():
            print(f"ℹ 暂无数据文件 {state_path}")
        else:
            print("ℹ 暂无交易记录")
        return 0

    rows, stats = build_rows(trades)
    headers = ["盘口时间", "秒", "方向", "价格", "份额", "成本", "阶段", "结果", "盈亏"]
    render_table(headers, rows)

    settled = stats["settled"]
    wins = stats["wins"]
    losses = stats["losses"]
    win_rate = wins / settled * 100.0 if settled else 0.0
    print(
        f"\n  已结算 {settled} 盘  胜 {wins} / 负 {losses}  胜率 {win_rate:.1f}%"
        f"   锁定中 {stats['locked']}  持仓中 {stats['holding']}"
    )
    print(f"  已实现净盈亏: ${stats['net']:+.2f}")
    print(f"  状态文件: {state_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
