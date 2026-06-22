# mybot-codex

`mybot-codex` 是一个独立的 Polymarket BTC 5 分钟 taker dry-run 机器人。

它只跑一条策略: `t1_late`。它不包含老机器人的 accum、maker 做市、zscore、sniper、训练模型等逻辑，也不会读 `/opt/jy-data`。VPS 上的老机器人 `jy-bot.service` 和新机器人 `mybot-codex.service` 是两个服务。

## 策略口径

当前策略是一个尾盘确认 taker 策略:

1. T-10 秒读取 Up/Down 顶档 ask。
2. T-8 秒再次读取 Up/Down 顶档 ask。
3. T-10 和 T-8 的强势边必须相同。
4. T-10 和 T-8 的强势边 ask 必须 `>= T1_LATE_CONFIRM_MIN_ASK`，默认 `0.98`。
5. T-1 秒再次读取盘口。
6. T-1 强势边必须仍然是同一边。
7. T-1 强势边 ask 必须 `>= T1_LATE_ENTRY_MIN_ASK`，默认 `0.75`。
8. T-1 弱势边 ask 必须 `<= T1_LATE_OPP_MAX_ASK`，默认 `0.30`。
9. 满足条件后买强势边，订单类型是 FAK。

手续费模型:

```text
taker fee = 0.07 * price * (1 - price)
full cost per share = price + taker fee
```

仓位模型:

```text
planned shares = min(
  T1_LATE_TARGET_QTY,
  top ask size,
  max_deploy / full_cost_per_share
)
```

`max_deploy` 来自当前 dry-run 权益和配置:

```text
equity = T1_LATE_START_EQUITY + 已实现 dry-run PnL
max_deploy = equity * T1_LATE_RISK_FRACTION
```

如果 `T1_LATE_MAX_DEPLOY_USDC > 0`，还会再受固定 USDC 上限限制。

## 本地安装运行

进入项目目录:

```bash
cd /mnt/d/GPT项目/做市机器人/mybot_codex
```

复制配置:

```bash
cp .env.example .env
```

编译:

```bash
cargo build --release
```

本地 dry-run:

```bash
cargo run --release -- .env
```

或者直接运行编译好的二进制:

```bash
target/release/mybot-codex .env
```

默认 `DRY_RUN=1`，不会真实下单。

## VPS 安装更新

部署脚本会把本地编译好的二进制上传到 VPS，并安装成独立服务:

```bash
cd /mnt/d/GPT项目/做市机器人/mybot_codex
cargo build --release
scripts/deploy-vps.sh dubolin-vps
```

部署到 VPS 后的路径:

```text
二进制: /usr/local/bin/mybot-codex
服务名: mybot-codex.service
配置:   /opt/mybot-codex/.env
数据:   /opt/mybot-codex/data
信号:   /opt/mybot-codex/data/t1_late_signals.jsonl
状态:   /opt/mybot-codex/data/t1_late_state.json
统计:   /usr/local/bin/jytd
```

部署脚本只安装/重启 `mybot-codex.service`，不会停止或修改老的 `jy-bot.service`。

## VPS 常用命令

看新机器人服务状态:

```bash
systemctl status mybot-codex.service
```

看实时日志:

```bash
journalctl -u mybot-codex.service -f
```

看最近日志:

```bash
journalctl -u mybot-codex.service --since '30 minutes ago' --no-pager
```

看信号明细:

```bash
tail -f /opt/mybot-codex/data/t1_late_signals.jsonl
```

看交易统计表:

```bash
jytd
```

`jytd` 输出的是老机器人同风格交易统计表:

```text
盘口时间 | 秒 | 方向 | 价格 | 份额 | 成本 | 阶段 | 结果 | 盈亏
```

重启新机器人:

```bash
systemctl restart mybot-codex.service
```

编辑配置:

```bash
nano /opt/mybot-codex/.env
systemctl restart mybot-codex.service
```

确认老机器人仍在:

```bash
systemctl status jy-bot.service
```

## 配置说明

核心配置在 `.env.example` 或 VPS 的 `/opt/mybot-codex/.env`。

```text
DRY_RUN=1
```

`1` 表示只模拟，不真实下单。`0` 表示允许真实 FAK 下单。没有连续 dry-run 验证前不要改成 `0`。

```text
PRIVATE_KEY=
DEPOSIT_WALLET_ADDRESS=
SIGNATURE_TYPE=3
```

真实下单才需要。dry-run 不需要填。

```text
MARKET_SLUG_PREFIX=btc-updown-5m
```

交易市场前缀。当前只针对 BTC Up/Down 5m。

```text
DATA_DIR=/opt/mybot-codex/data
SIGNAL_FILE=/opt/mybot-codex/data/t1_late_signals.jsonl
STATE_FILE=/opt/mybot-codex/data/t1_late_state.json
```

运行数据路径。`jytd` 默认读取 `STATE_FILE`。

```text
POLL_MS=200
```

主循环等待时间，单位毫秒。

```text
T1_LATE_CONFIRM1_SECS=10
T1_LATE_CONFIRM2_SECS=8
T1_LATE_ENTRY_SECS=1
```

确认和入场秒数。默认是 T-10、T-8 确认，T-1 入场。

```text
T1_LATE_CONFIRM_MIN_ASK=0.98
T1_LATE_ENTRY_MIN_ASK=0.75
T1_LATE_OPP_MAX_ASK=0.30
```

盘口过滤条件。

```text
T1_LATE_TARGET_QTY=2000
```

目标份额。实际成交还会受顶档 ask size 和资金上限限制。

```text
T1_LATE_START_EQUITY=300
T1_LATE_RISK_FRACTION=1
T1_LATE_MAX_DEPLOY_USDC=0
```

dry-run 资金模型。

- `T1_LATE_START_EQUITY=300`: 模拟初始本金 300u。
- `T1_LATE_RISK_FRACTION=1`: 每盘最多用当前模拟权益的 100%。
- `T1_LATE_RISK_FRACTION=0.25`: 每盘最多用当前模拟权益的 25%。
- `T1_LATE_MAX_DEPLOY_USDC=0`: 不设置固定单盘口上限。
- `T1_LATE_MAX_DEPLOY_USDC=300`: 每个盘口最多部署 300u。

```text
REST_FALLBACK_TIMEOUT_MS=700
```

尾盘 WebSocket 没有 ask 时，使用 REST `/book` 补盘口的超时时间。

## 文件作用

```text
src/main.rs
```

机器人主程序。负责读配置、连接 Polymarket、订阅盘口、执行 T-1 策略、记录状态、结算 dry-run PnL。

主要代码块:

- `Config`: 从 `.env` 读取所有配置。
- `ClobClient`: 通过 REST 查当前 BTC 5m 市场、盘口、结算赢家。
- `MarketWs`: 连接 Polymarket market WebSocket，维护顶档盘口缓存。
- `OrderBook`: 保存 Up/Down 的 asks/bids。
- `State` / `Trade`: 保存 dry-run 交易记录和结算结果。
- `Bot::run_once`: 每轮主循环，找市场、取盘口、检查结算、调用策略。
- `Bot::decide_t1_late`: T-10/T-8/T-1 策略核心。
- `OrderExecutor`: dry-run 或真实 FAK 下单的统一入口。

```text
.env.example
```

本地配置模板。VPS 首次安装时也会用同样字段生成 `/opt/mybot-codex/.env`。

```text
scripts/install-systemd.sh
```

在 VPS 上创建 `/opt/mybot-codex`、生成 `.env`、写入 systemd unit，并启用 `mybot-codex.service`。

```text
scripts/deploy-vps.sh
```

本地部署脚本。上传 `target/release/mybot-codex`、安装 `jytd`、重启 `mybot-codex.service`，并打印服务状态和最近信号。

```text
scripts/jytd.py
```

交易统计表命令。部署后安装为 `/usr/local/bin/jytd`。默认读取 `/opt/mybot-codex/data/t1_late_state.json`，输出老机器人同风格表格。

```text
Cargo.toml / Cargo.lock
```

Rust 依赖清单和锁定版本。

```text
data/t1_late_signals.jsonl
```

本地 dry-run 信号日志样例。VPS 上真实路径是 `/opt/mybot-codex/data/t1_late_signals.jsonl`。

## 信号日志 phase 含义

`t1_late_signals.jsonl` 是逐行 JSON。

常见 `phase`:

- `market`: 发现新的 5 分钟盘口。
- `t1_late_tail`: 尾盘盘口快照。
- `t1_late_book_missing`: 尾盘缺少 ask，无法判断。
- `t1_late_confirm1`: T-10 确认成功。
- `t1_late_confirm2`: T-8 确认成功。
- `t1_late_block`: 被条件拦截，不入场。
- `intent`: 准备下单，记录计划份额和计划成本。
- `submit`: FAK 提交或 dry-run 模拟提交结果。
- `t1_late_entry`: 记录实际 dry-run 成交。
- `settled`: 盘口结算后记录 winner 和 PnL。

## 交易统计表

直接运行:

```bash
jytd
```

如果没有交易，会显示:

```text
ℹ 暂无数据文件 /opt/mybot-codex/data/t1_late_state.json
```

有交易后会显示:

```text
┌─────────────┬─────┬──────┬───────┬──────┬────────┬────────┬──────┬───────┐
│ 盘口时间    │ 秒  │ 方向 │ 价格  │ 份额 │ 成本   │ 阶段   │ 结果 │ 盈亏  │
...
└─────────────┴─────┴──────┴───────┴──────┴────────┴────────┴──────┴───────┘

  已结算 N 盘  胜 W / 负 L  胜率 X.X%   锁定中 0  持仓中 H
  已实现净盈亏: $+0.00
```

## 实盘前检查

实盘前至少确认:

1. `DRY_RUN=1` 连续跑满一天。
2. `jytd` 能正常显示模拟交易和结算。
3. `t1_late_signals.jsonl` 里 T-10/T-8/T-1 盘口字段完整。
4. 现场盘口和历史 Telonex 字段口径一致。
5. 明确接受 `T1_LATE_RISK_FRACTION=1` 的高风险含义。

历史回测不是未来收益保证。`T1_LATE_RISK_FRACTION=1` 等于每盘最多投入当前全部模拟权益，未来只要错一盘，可能接近打穿账户。
