# mybot-codex

`mybot-codex` 是一个独立的 Polymarket BTC 5 分钟 taker dry-run 机器人。

它默认跑一条策略: `btc_oracle_fallback`。旧的 `btc_distance_ladder`、`btc_distance_tail` 和 `t1_late` 仍保留，可通过配置切换。它不包含老机器人的 accum、maker 做市、zscore、sniper、训练模型等逻辑，也不会读 `/opt/jy-data`。VPS 上的老机器人 `jy-bot.service` 和新机器人 `mybot-codex.service` 是两个服务。

## 策略口径

### btc_oracle_fallback

当前默认策略是 Candidate B：BTC 涨跌幅和 ask 盈亏比绑定的 taker fallback 策略。

1. 通过 Polymarket RTDS 订阅 `btc/usd` 实时价格。
2. 新盘口开始时记录 BTC 开盘附近价格，作为本盘基准价。
3. 当前 BTC 高于基准价只考虑 Up，低于或等于基准价只考虑 Down。
4. 只在精确 T-8 / T-5 / T-3 三个秒点检查，不使用最后 2 秒。
5. T-8 强信号：BTC 偏离 `>=5 bps`，ask `<=0.95`，可用 100% 当盘预算。
6. T-8 普通信号：BTC 偏离 `>=0.5 bps`，ask `<=0.93`，可用 75% 当盘预算。
7. T-5 强 fallback：BTC 偏离 `>=1.5 bps`，ask `<=0.98`，spread `<=0.10`，可用 100% 当盘预算。
8. T-5 低价 fallback：BTC 偏离 `>=0.2 bps`，ask `<=0.85`，spread `<=0.10`，可用 75% 当盘预算。
9. T-3 最后 fallback：ask `<=0.95`，可用 100% 当盘预算；不吃 0.97~0.99 高价尾盘。
10. 同一盘口最多下一笔 FAK，默认 `DRY_RUN=1` 只模拟。

当前 Candidate B 回测口径:

```text
300u -> 10027.39u
平均每天 +313.79u
最差日 -282.14u
最大回撤 866.25u
最大单笔成本 269.97u
2102 笔，错 421 笔
```

这个结果来自 `telonex-qty295-single-month/scripts/roll_btc_oracle_fallback.py`，使用一个月 live-state 盘口和 Telonex BTC 价格特征。它达到了历史 taker 回测目标，但仍只能先跑 VPS dry-run；重点验证 T-8/T-5/T-3 的真实 FAK 成交率、盘口延迟和 ask 消失率。

### btc_distance_ladder

旧的 BTC 距离分层 taker 策略，默认不再死磕最后 2 秒:

1. 通过 Polymarket RTDS 订阅 `btc/usd` 实时价格。
2. 新盘口开始时记录 BTC 开盘附近价格，作为本盘基准价。
3. 当前 BTC 价格高于本盘基准价就只考虑 Up，低于基准价就只考虑 Down。
4. T-60 开始允许 early 层，但要求 BTC 距离极端，只能补到单盘口风险上限的 5%。
5. T-5 开始允许 mid 层，要求 BTC 距离和强度仍达标，最多补到单盘口风险上限的 100%。
6. Tail 层默认关闭，`BTC_LADDER_TAIL_MAX_ASK=0.01`，只在专门测试 T-2/T-1 时再打开。
7. 同一盘口不追着换边；如果已持有一边，BTC 方向反转时不再加仓。
8. 同一时刻只补到当前最高允许层，不会把 early/mid/tail 在同一顶档重复吃三次。
9. 所有下单都是 FAK，默认 `DRY_RUN=1` 只模拟。

当前默认 no-tail cap255 口径:

```text
300u -> 10037.07u
平均每天 +314.10u
最差日 -308.87u
最大回撤 844.59u
最大单盘口成本 255.00u
2181 盘，错 116 盘
```

这个结果来自 `telonex-qty295-single-month/scripts/roll_btc_distance_ladder_taker.py`，使用一个月 live-state 盘口和 Telonex BTC 价格特征。旧的 T-2/T-1 tail 补仓版本回测更高，但默认不采用，因为实盘模拟更需要验证 T-5 可成交性和风险边界。

### btc_distance_tail

旧的尾盘策略是 BTC 距离尾盘 taker 策略:

1. 通过 Polymarket RTDS 订阅 `btc/usd` 实时价格。
2. 新盘口开始时记录 BTC 开盘附近价格，作为本盘基准价。
3. T-2 秒开始检查；如果 T-2 不合格，T-1 再兜底检查一次。
4. 当前 BTC 价格高于本盘基准价就买 Up，低于基准价就买 Down。
5. BTC 距离不能等于 0，且必须满足 `BTC_DISTANCE_MIN_ABS_BPS`。
6. 选中方向 ask 必须 `<= BTC_DISTANCE_MAX_ASK`，默认 `0.99`。
7. 选中方向 spread 必须 `<= BTC_DISTANCE_MAX_SPREAD`，默认 `0.10`。
8. 跳过 `BTC_DISTANCE_EXCLUDE_ASK_LOW <= ask < BTC_DISTANCE_EXCLUDE_ASK_HIGH`，默认跳过 `0.85~0.90`。
9. 满足条件后买选中方向，订单类型是 FAK。

当前一个月回测最佳口径:

```text
300u -> 12094.76u
平均每天 +380.48u
最差日 -216.26u
最大回撤 740.93u
1881 盘，错 125 盘
```

### t1_late

旧策略是尾盘盘口确认 taker 策略:

1. T-10 秒读取 Up/Down 顶档 ask。
2. T-8 秒再次读取 Up/Down 顶档 ask。
3. T-10 和 T-8 的强势边必须相同。
4. T-10 和 T-8 的强势边 ask 必须 `>= T1_LATE_CONFIRM_MIN_ASK`。
5. T-1 秒再次读取盘口。
6. T-1 强势边必须仍然是同一边。
7. T-1 强势边 ask 必须 `>= T1_LATE_ENTRY_MIN_ASK`。
8. T-1 弱势边 ask 必须 `<= T1_LATE_OPP_MAX_ASK`。
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
STRATEGY=btc_oracle_fallback
```

策略选择。默认 `btc_oracle_fallback`；如需旧分层逻辑可改为 `btc_distance_ladder`，如需旧尾盘 BTC 距离逻辑可改为 `btc_distance_tail`，如需盘口确认旧逻辑可改为 `t1_late`。

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
BTC_PRICE_WS_URL=wss://ws-live-data.polymarket.com
BTC_PRICE_TOPIC=crypto_prices_chainlink
BTC_PRICE_TYPE=*
BTC_PRICE_FILTERS={"symbol":"btc/usd"}
BTC_PRICE_SYMBOL=btc/usd
BTC_PRICE_MAX_AGE_MS=3000
```

BTC 实时价格源。用于计算当前 BTC 价格相对本盘开盘价的涨跌距离。

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
T1_LATE_TARGET_QTY=5000
```

目标份额。两个策略共用。实际成交还会受顶档 ask size 和资金上限限制。

```text
T1_LATE_START_EQUITY=300
T1_LATE_RISK_FRACTION=0.2
T1_LATE_MAX_DEPLOY_USDC=255
```

dry-run 资金模型。

- `T1_LATE_START_EQUITY=300`: 模拟初始本金 300u。
- `T1_LATE_RISK_FRACTION=0.2`: 每盘最多用当前模拟权益的 20%。
- `T1_LATE_MAX_DEPLOY_USDC=255`: 每个盘口最多部署 255u。
- `T1_LATE_MAX_DEPLOY_USDC=0`: 不设置固定单盘口上限。

```text
BTC_ORACLE_PROFILE=label=e8_strong,sec=8,bps=5,ask=0.95,spread=none,frac=1;...
BTC_ORACLE_RISK_FRACTION=0.25
BTC_ORACLE_MAX_DEPLOY_USDC=270
BTC_ORACLE_DAILY_TAKE_PROFIT=800
```

Candidate B 参数。

- `BTC_ORACLE_PROFILE`: 五层 fallback 配置；机器人按顺序检查同一秒的 tier，第一档满足就下单。
- `label`: `jytd` 和信号日志里显示的阶段名。
- `sec`: 精确剩余秒数，只在这个秒点检查。
- `bps`: BTC 相对本盘基准价的最小绝对偏离，单位 bps。
- `ask`: 选中方向最高可吃 ask。
- `spread`: 选中方向最大 ask-bid spread；`none` 表示不检查。
- `frac`: 这一档最多使用当盘预算的比例。
- `BTC_ORACLE_RISK_FRACTION=0.25`: 每盘最多用当前 dry-run 权益的 25%。
- `BTC_ORACLE_MAX_DEPLOY_USDC=270`: 每个盘口硬上限 270u。
- `BTC_ORACLE_DAILY_TAKE_PROFIT=800`: 当天已结算 dry-run PnL 到 800u 后停止当天新开仓。

```text
BTC_DISTANCE_TAIL_MAX_SECS=2
BTC_DISTANCE_START_CAPTURE_MIN_SECS=295
BTC_DISTANCE_MAX_ASK=0.99
BTC_DISTANCE_MAX_SPREAD=0.10
BTC_DISTANCE_EXCLUDE_ASK_LOW=0.85
BTC_DISTANCE_EXCLUDE_ASK_HIGH=0.90
BTC_DISTANCE_MIN_ABS_BPS=0
```

BTC 距离策略参数。

- `BTC_DISTANCE_TAIL_MAX_SECS=2`: T-2 开始允许入场；T-2 不合格时 T-1 兜底。
- `BTC_DISTANCE_START_CAPTURE_MIN_SECS=295`: 只在盘口刚开始时记录 BTC 基准价；中途重启会跳过当前盘口。
- `BTC_DISTANCE_MAX_ASK=0.99`: 选中方向最高可买 ask。
- `BTC_DISTANCE_MAX_SPREAD=0.10`: 选中方向最大 spread。
- `BTC_DISTANCE_EXCLUDE_ASK_LOW/HIGH=0.85/0.90`: 跳过回测中表现差的半强价格带。
- `BTC_DISTANCE_MIN_ABS_BPS=0`: 只要求 BTC 距离不等于 0。

```text
BTC_LADDER_EARLY_SECS=60
BTC_LADDER_EARLY_BUDGET_FRAC=0.05
BTC_LADDER_EARLY_MIN_ABS_BPS=10
BTC_LADDER_EARLY_MIN_SCORE=2.5
BTC_LADDER_EARLY_MAX_ASK=0.95
BTC_LADDER_EARLY_MAX_SPREAD=0.10
```

分层策略 early 层。T-60 开始看，BTC 距离和强度必须很高，最多只补到单盘口风险上限的 5%。

```text
BTC_LADDER_MID_SECS=5
BTC_LADDER_MID_BUDGET_FRAC=1.0
BTC_LADDER_MID_MIN_ABS_BPS=1
BTC_LADDER_MID_MIN_SCORE=0.5
BTC_LADDER_MID_MAX_ASK=0.99
BTC_LADDER_MID_MAX_SPREAD=0.10
BTC_LADDER_MID_EXCLUDE_ASK_LOW=0.85
BTC_LADDER_MID_EXCLUDE_ASK_HIGH=0.90
```

分层策略 mid 层。T-5 开始看，最多补到单盘口风险上限的 100%，并跳过 `0.85~0.90` 的坏价格带。

```text
BTC_LADDER_TAIL_SECS=2
BTC_LADDER_TAIL_BUDGET_FRAC=1.0
BTC_LADDER_TAIL_MIN_ABS_BPS=0
BTC_LADDER_TAIL_MIN_SCORE=0
BTC_LADDER_TAIL_MAX_ASK=0.01
BTC_LADDER_TAIL_MAX_SPREAD=0.10
BTC_LADDER_TAIL_EXCLUDE_ASK_LOW=0.85
BTC_LADDER_TAIL_EXCLUDE_ASK_HIGH=0.90
```

分层策略 tail 层。默认 `BTC_LADDER_TAIL_MAX_ASK=0.01`，实际等于关闭 T-2/T-1 补仓；只有专门测试尾盘时才把它调高。

```text
REST_FALLBACK_TIMEOUT_MS=700
```

尾盘 WebSocket 没有 ask 时，使用 REST `/book` 补盘口的超时时间。

## 文件作用

```text
src/main.rs
```

机器人主程序。负责读配置、连接 Polymarket、订阅盘口和 BTC 价格、执行 taker 策略、记录状态、结算 dry-run PnL。

主要代码块:

- `Config`: 从 `.env` 读取所有配置。
- `ClobClient`: 通过 REST 查当前 BTC 5m 市场、盘口、结算赢家。
- `MarketWs`: 连接 Polymarket market WebSocket，维护顶档盘口缓存。
- `BtcPriceWs`: 订阅 Polymarket RTDS BTC 价格，维护最新 `btc/usd`。
- `OrderBook`: 保存 Up/Down 的 asks/bids。
- `State` / `Trade`: 保存 dry-run 交易记录和结算结果。
- `Bot::run_once`: 每轮主循环，找市场、取盘口、检查结算、调用策略。
- `Bot::decide_btc_oracle_fallback`: Candidate B，BTC 涨跌幅和 ask 盈亏比分层 fallback。
- `Bot::decide_btc_distance_ladder`: BTC 距离分层策略核心。
- `Bot::decide_btc_distance_tail`: BTC 距离尾盘策略核心。
- `Bot::decide_t1_late`: 旧 T-10/T-8/T-1 策略核心。
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
- `btc_distance_start`: 记录本盘 BTC 基准价。
- `btc_oracle_tail`: Candidate B 尾盘盘口快照。
- `btc_oracle_book_missing`: Candidate B 尾盘缺少 ask，无法判断。
- `btc_oracle_block`: Candidate B 被条件拦截，不入场。
- `btc_oracle_fallback_entry`: Candidate B dry-run 成交。
- `btc_distance_block`: BTC 距离策略被条件拦截，不入场。
- `btc_distance_ladder_entry`: BTC 距离分层策略 dry-run 成交。
- `btc_distance_tail_entry`: BTC 距离策略 dry-run 成交。
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
3. `t1_late_signals.jsonl` 里 `btc_distance_start`、`btc_oracle_fallback_entry`、`intent`、`submit`、`settled` 字段完整。
4. 现场盘口和历史 Telonex 字段口径一致。
5. 明确接受 `BTC_ORACLE_RISK_FRACTION=0.25` 和 `BTC_ORACLE_MAX_DEPLOY_USDC=270` 的风险边界。
6. 统计 T-8/T-5/T-3 的 FAK 成交率、无 ask 率、REST fallback 次数和 RTDS BTC 延迟。

历史回测不是未来收益保证。当前默认 Candidate B 回测仍然存在约 -270u 的单盘口亏损和约 866.25u 的最大回撤，必须先 dry-run 验证 T-8/T-5/T-3 真实可成交性、RTDS 延迟和时间戳对齐。
