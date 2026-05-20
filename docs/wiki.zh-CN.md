# TokenAltar 中文使用 Wiki

本文是 TokenAltar 的中文使用手册，面向四类读者：

- **部署者**：负责把 TokenAltar 跑起来、升级、备份和维护。
- **管理员**：负责用户、通道、价格、路由规则、系统设置和整体秩序。
- **普通用户**：负责创建自己的 API Key、调用模型、管理自己的通道和点数。
- **API 使用者**：负责把 OpenAI、Anthropic、Gemini 或兼容客户端接入 TokenAltar。

TokenAltar 的核心目标是把分散在不同成员手里的 LLM API 额度组织成一个可管理、可计费、可路由、可协作的共享资源池。成员用本地 `sk-...` Key 调用模型，通道提供者贡献上游账号能力，系统用点数完成消费扣费、提供者奖励、账本记录、排行榜和社交经济流转。

## 目录

- [核心概念](#核心概念)
- [角色与权限](#角色与权限)
- [快速开始](#快速开始)
- [Docker 部署](#docker-部署)
- [首次初始化](#首次初始化)
- [控制台页面总览](#控制台页面总览)
- [普通用户使用流程](#普通用户使用流程)
- [管理员使用流程](#管理员使用流程)
- [API Key 使用方法](#api-key-使用方法)
- [通道配置方法](#通道配置方法)
- [点数、价格与结算](#点数价格与结算)
- [路由与可靠性](#路由与可靠性)
- [亲和规则](#亲和规则)
- [健康监控](#健康监控)
- [经济系统](#经济系统)
- [排行榜](#排行榜)
- [账本](#账本)
- [运行设置](#运行设置)
- [API 调用示例](#api-调用示例)
- [升级、备份与迁移](#升级备份与迁移)
- [常见问题](#常见问题)
- [排障清单](#排障清单)

## 核心概念

### TokenAltar 是什么

TokenAltar 是一个单进程 Rust + SQLite + Vue 控制台应用。它同时提供：

- Web 控制台，用于管理用户、API Key、通道、价格、健康、账本和设置。
- LLM 网关接口，用于接收 OpenAI、Anthropic、Gemini 风格的请求。
- 点数结算系统，用于把模型消耗、通道贡献和内部转账统一起来。

部署时，前端控制台会被构建并嵌入到 Rust 二进制里。运行时不需要单独部署 Nginx、Node 服务或前端静态目录。

### 用户

用户是 TokenAltar 里的成员账号。每个用户有：

- 邮箱
- 显示名
- 角色
- 点数余额
- 是否启用
- 是否匿名参与排行榜

用户分为普通用户和管理员。管理员可以管理全局资源，普通用户主要管理自己的 Key、自己的通道和自己的经济行为。

### API Key

API Key 是客户端调用 TokenAltar 网关时使用的本地密钥，格式以 `sk-...` 开头。

API Key 可以限制：

- 是否启用
- 消费上限
- 过期时间
- 可用模型
- 可用通道

TokenAltar 的 API Key 不等于上游平台密钥。上游平台密钥只保存在通道配置里，客户端不直接接触。

### 通道

通道是一个上游模型服务入口。例如：

- 一个 OpenAI API 账号
- 一个 Anthropic API 账号
- 一个 Gemini API 账号
- 一个兼容上述协议的代理服务

每个通道包含：

- 服务商类型
- Base URL
- 上游 API Key
- 可用模型范围
- 点数额度窗口
- 促销策略
- 健康状态

普通用户可以创建和管理自己的通道。管理员可以看到全局通道，但控制台返回的数据始终会隐藏上游 API Key。

### 点数

点数是 TokenAltar 内部的统一价值单位。

一次模型调用会消耗点数。通道提供者承载请求后会获得提供者奖励点数。P2P 转账、口令红包和排行榜也围绕点数展开。

### 价格

模型价格按每 **100 万 token** 设置，拆成三类：

- 输入 token 价格
- 输出 token 价格
- 缓存 token 价格

价格可以是全局价格，也可以是某个通道的专属价格。结算时通道专属价格优先于全局价格。

### 通道额度窗口

通道额度窗口使用点数计量，不是直接使用 token 计量。

一个通道可以同时设置多个窗口，例如：

- 月窗口：每月最多可承载多少点数的请求
- 日窗口：每天最多可承载多少点数的请求
- 小时窗口：每小时最多可承载多少点数的请求

只要任意一个窗口不足，通道就不能继续承载新的请求，直到对应窗口刷新。

### 账本

账本记录每次网关请求的结算结果，包括：

- 时间
- 模型
- 输入/输出/缓存 token
- 消费点数
- tokenizer 说明
- 结算公式文本

账本是理解消费、收益、排行榜和问题排查的主要依据。

## 角色与权限

### 普通用户能做什么

普通用户可以：

- 登录控制台
- 查看自己的余额
- 创建、启用、禁用、旋转和删除自己的 API Key
- 设置 API Key 的模型范围、通道范围和消费上限
- 添加和管理自己的上游通道
- 为自己的通道设置通道专属模型价格
- 查看自己可见通道的健康状态
- 查看账本记录
- 发起点数转账
- 创建和领取口令红包
- 设置排行榜匿名状态
- 查看排行榜和 Guide 页面

普通用户看不到管理员专用页面，例如 Users、Affinity 和 Settings。

### 管理员能做什么

管理员可以：

- 创建、编辑、启用、禁用用户
- 重置用户密码
- 调整用户余额和角色
- 查看全局通道、价格、账本和健康状态
- 管理全局模型价格
- 创建亲和路由规则
- 修改运行设置
- 控制邀请注册、默认余额、默认 Key 限额、默认通道配置、峰值倍率、重试参数、提供者奖励倍率等

管理员不应该把自己的登录态或控制台 token 给客户端使用。客户端只需要 `sk-...` 网关 Key。

### 停用账号的影响

账号被停用后：

- 无法登录控制台
- 现有会话不可继续使用
- 该用户的 API Key 不可继续认证
- 该用户的活动通道会被关闭
- 历史账本和资源记录仍保留

系统会保护最后一个启用状态的管理员，避免把控制台彻底锁死。

## 快速开始

### 本地运行

先安装依赖并构建前端：

```bash
pnpm --dir frontend install
pnpm --dir frontend build
```

启动服务：

```bash
TOKENALTAR_ADMIN_EMAIL=admin@example.com \
TOKENALTAR_ADMIN_PASSWORD='change-me-now' \
cargo run
```

默认监听：

```text
http://127.0.0.1:8080
```

默认数据库：

```text
tokenaltar.sqlite3
```

打开浏览器访问控制台，用 `TOKENALTAR_ADMIN_EMAIL` 和 `TOKENALTAR_ADMIN_PASSWORD` 登录。

### 构建发布版

```bash
pnpm --dir frontend build
cargo build --release
```

发布二进制在：

```text
target/release/tokenaltar
```

注意：每次修改前端后，都要先运行 `pnpm --dir frontend build`，再构建 Rust。否则二进制里嵌入的还是旧控制台。

## Docker 部署

### 使用 Docker Compose

复制环境变量模板：

```bash
cp .env.example .env
```

编辑 `.env`，至少修改：

```dotenv
TOKENALTAR_ADMIN_EMAIL=admin@example.com
TOKENALTAR_ADMIN_PASSWORD=replace-with-a-long-random-password
TOKENALTAR_LEADERBOARD_TIMEZONE=Asia/Shanghai
```

启动：

```bash
docker compose up -d --build
```

访问：

```text
http://localhost:8080
```

### 使用 GHCR 镜像

如果要直接使用 GitHub Container Registry 的镜像：

```bash
docker pull ghcr.io/codeboy2006/tokenaltar:latest
docker compose up -d
```

### Docker 数据卷

Compose 使用固定命名的数据卷：

```text
tokenaltar-data
```

容器内数据库路径：

```text
/data/tokenaltar.sqlite3
```

不要随意执行：

```bash
docker compose down -v
```

这会删除数据库卷，除非你明确想清空数据。

### 从旧 Compose 卷迁移

如果旧部署因为 Compose 项目名不同产生了类似 `oldproject_tokenaltar-data` 的卷，可以迁移到固定卷：

```bash
docker volume create tokenaltar-data
docker run --rm \
  -v oldproject_tokenaltar-data:/from:ro \
  -v tokenaltar-data:/to \
  alpine sh -c 'cp -a /from/. /to/'
docker compose up -d
```

## 首次初始化

### 1. 创建管理员

首次启动时，如果数据库里还没有管理员，并且设置了：

```bash
TOKENALTAR_ADMIN_EMAIL=...
TOKENALTAR_ADMIN_PASSWORD=...
```

系统会自动创建第一个管理员。

如果数据库已经有管理员，环境变量不会覆盖已有管理员密码。

### 2. 登录控制台

访问服务地址，使用管理员邮箱和密码登录。

登录后建议先做三件事：

1. 进入 Settings 检查邀请注册、初始余额、价格、路由和默认通道设置。
2. 进入 Channels 添加至少一个真实上游通道。
3. 进入 API Keys 创建一个测试 Key，并用本地客户端调用一次模型。

### 3. 配置生产凭据

不要在生产环境继续使用示例密码。建议：

- 使用长随机管理员密码
- 使用 HTTPS 反向代理保护外部访问
- 只给可信成员开放控制台
- 定期旋转上游 API Key 和本地 API Key

## 控制台页面总览

### Dashboard

Dashboard 显示当前资源池总览：

- Surge 状态
- 可路由点数
- 启用通道数
- 今日消费点数
- 支持的网关路径

这里适合快速判断资源池是否健康、点数是否充足、请求压力是否接近峰值。

### Users

管理员专用页面。

用于：

- 创建用户
- 修改邮箱、显示名、角色、余额
- 启用/停用账号
- 重置密码
- 查看用户 Key 数量、通道数量、消费和贡献

普通用户看不到该页面。

### API Keys

用于管理本地客户端密钥。

主要操作：

- 创建 Key
- 编辑 Key 名称
- 设置是否启用
- 设置消费上限
- 设置过期时间
- 设置允许模型
- 设置允许通道
- 旋转 Key
- 删除 Key

创建 Key 后，完整密钥只会在创建或旋转时显示一次。后续控制台只显示前缀。

### Channels

用于管理上游通道。

主要操作：

- 添加通道
- 编辑通道
- 配置服务商和 Base URL
- 填写上游 API Key
- 设置模型范围
- 设置点数额度窗口
- 设置 fire-sale 策略
- 测试通道
- 复制通道
- 启用/禁用通道
- 批量启用/禁用通道
- 删除通道

普通用户只能管理自己的通道。管理员可以全局查看和操作。

### Health

用于查看通道健康。

健康数据来自真实请求，而不是定时探测。

它会展示：

- 当前状态
- 近 24 小时 48 个半小时窗口
- 成功样本
- 空回复
- 降级样本
- 下线样本
- 平均 TTFT

TTFT 只统计成功且非空的响应。

### Pricing

用于设置模型价格。

价格字段：

- Input / 1M
- Output / 1M
- Cache / 1M

管理员可以设置全局默认价格。普通用户可以给自己的通道设置通道价格。

价格匹配顺序见 [点数、价格与结算](#点数价格与结算)。

### Affinity

管理员专用页面。

用于设置亲和路由规则，让同一租户、会话或缓存键尽量命中同一个通道。

适合：

- prompt cache
- 租户隔离
- 会话稳定性
- 缓存敏感业务

### Economy

用于点数社交流转。

支持：

- P2P 转账
- 创建口令红包
- 领取口令红包
- 查看转账历史
- 查看红包历史
- 设置排行榜匿名状态

### Leaderboards

用于查看贡献榜和消费榜。

支持：

- Day
- Month

Provider 榜按成功账本里的 token 供给排名。Consumer 榜按成功账本里的点数消费排名。

如果用户开启匿名排行，榜单会隐藏身份。

### Ledger

账本页面展示结算明细。

它适合回答：

- 某次请求用了多少 token
- 扣了多少点数
- 使用了哪个 tokenizer 估算
- 结算公式是什么
- 是否应用了 surge 或 fire-sale

### Guide

Guide 页面包含：

- 项目流程浮雕图
- 英文全局机制说明

这页适合给新用户解释 TokenAltar 的整体流程。

### Settings

管理员专用页面。

用于修改运行设置，包括：

- 邀请注册
- 默认邀请码
- 初始管理员点数
- 初始普通用户点数
- 结算小数位
- fallback 模型价格
- surge 阈值和倍率
- 路由重试参数
- fire-sale 路由权重
- 账本队列容量
- 亲和缓存容量
- 默认 API Key 消费上限
- 默认通道模板
- Provider Payout Multiplier

多数请求期设置会立即生效。队列容量、亲和缓存容量等启动期设置需要重启进程。

## 普通用户使用流程

### 只消费模型的用户

如果你只想使用模型，不贡献通道：

1. 登录控制台。
2. 查看自己的点数余额。
3. 进入 API Keys。
4. 创建一个 Key。
5. 如果需要，限制允许模型和允许通道。
6. 把 `sk-...` Key 配置到你的客户端。
7. 按 OpenAI、Anthropic 或 Gemini 风格调用 TokenAltar。
8. 在 Ledger 查看结算记录。

如果调用失败，先检查：

- 自己是否还有点数
- API Key 是否启用
- API Key 是否过期
- API Key 是否超过消费上限
- API Key 是否允许请求的模型
- API Key 是否允许至少一个可用通道

### 贡献上游通道的用户

如果你有上游 API 额度，可以把它作为通道加入资源池：

1. 进入 Channels。
2. 点击 Add Channel。
3. 选择 Provider。
4. 填写 Base URL。
5. 填写上游 API Key。
6. 填写 Models，例如 `gpt-*` 或具体模型名。
7. 设置点数额度窗口。
8. 设置 fire-sale 策略。
9. 保存并测试通道。
10. 等待请求命中通道后，在 Ledger 和 Leaderboards 查看贡献。

上游 API Key 不会显示给其他普通用户。

### 设置自己的通道价格

如果你贡献了通道，可以进入 Pricing 为自己的通道设置价格。

推荐做法：

1. 选择 Scope 为你的通道。
2. 设置 `default` 价格，作为该通道兜底价格。
3. 对特殊模型单独设置正则，例如 `^gpt-5\\.5$`。
4. 检查 Ledger 中公式是否符合预期。

如果不设置通道价格，系统会使用全局价格或运行设置里的 fallback 价格。

## 管理员使用流程

### 初始配置顺序

建议管理员按以下顺序配置：

1. Settings：确认邀请制、默认余额、默认 Key 限额、价格 fallback、Provider Payout Multiplier。
2. Users：创建或导入成员账号。
3. Channels：添加至少一个全局可用通道。
4. Pricing：配置全局模型价格。
5. Affinity：按业务需要配置亲和规则。
6. API Keys：创建测试 Key。
7. Health：观察真实请求产生的健康窗口。
8. Leaderboards 和 Ledger：验证结算和榜单。

### 管理用户

在 Users 页面可以创建和维护用户。

常见操作：

- 创建普通用户
- 创建管理员
- 调整余额
- 禁用异常账号
- 重置忘记的密码

谨慎操作：

- 不要禁用最后一个管理员。
- 不要随意把普通用户升为管理员。
- 调整余额会影响成员之间的经济公平性，建议留有外部记录。

### 管理全局价格

管理员在 Pricing 页面选择 Global default 可以设置全局价格。

建议至少设置：

- `default`
- 常用 GPT 模型
- 常用 Claude 模型
- 常用 Gemini 模型

模型模式是正则表达式。更具体的模式应优先添加，避免被宽泛规则误匹配。

### 管理 Provider Payout Multiplier

Provider Payout Multiplier 在 Settings 中设置，对提供者奖励生效：

```text
provider_points = total_points * Provider Payout Multiplier
```

如果设置为：

- `0.7`：提供者获得消费者实际结算点数的 70%。
- `1.0`：提供者获得与消费者扣费相同的点数。
- `1.25`：提供者获得 125%，相当于平台补贴提供者。

这是全局策略，不再由单个通道自行设置。

### 管理邀请制

Settings 中：

- `invite_required = true`：注册必须填写邀请码。
- `invite_code_default`：默认邀请码。

如果是私有小圈子，建议开启邀请制。

## API Key 使用方法

### 创建 Key

进入 API Keys 页面，填写：

- Name：方便识别用途。
- Status：是否启用。
- Spend Limit：累计消费上限，可留空表示不限制。
- Expires At：过期时间，可留空。
- Allowed Models：允许模型列表，可留空表示不限制。
- Allowed Channels：允许使用的通道。

创建后保存完整 `sk-...`。

### 模型白名单

Allowed Models 支持：

- 精确模型名，例如 `gpt-5.4`
- 前缀通配，例如 `gpt-4o*`

空列表表示允许所有模型。

### 通道白名单

Allowed Channels 决定该 Key 可以路由到哪些通道。

新 Key 默认包含当前全部路由通道。如果 Key 一直覆盖完整通道池，后续新增通道会自动加入。如果你手动缩小过范围，后续新增通道不会自动加入。

### 消费上限

Spend Limit 是 Key 维度的累计点数上限。

如果请求预估会超过上限，系统会拒绝请求。

### 旋转 Key

如果 Key 泄露：

1. 进入 API Keys。
2. 找到对应 Key。
3. 点击 Rotate。
4. 用新 Key 更新客户端。

旧 Key 会失效。

### 删除 Key

删除是软删除：

- Key 不再可用。
- 控制台不再展示。
- 历史账本仍保留。

## 通道配置方法

### Provider

支持：

- `openai`
- `anthropic`
- `gemini`

Provider 决定上游协议和转发路径。

### Base URL

Base URL 是上游服务地址。

示例：

```text
https://api.openai.com
https://api.anthropic.com
https://generativelanguage.googleapis.com
```

如果使用兼容代理，填写代理提供的基础地址。

### API Key

这里填写上游平台 Key。它只用于 TokenAltar 到上游服务的请求，不会返回给普通控制台响应。

编辑已有通道时，如果 API Key 输入框留空，系统会保留旧密钥。

### Models

Models 决定通道支持哪些模型。

可以填写：

```text
*
gpt-*
gpt-4o*
claude-sonnet-*
gemini-*
```

多个模型规则用逗号分隔。

### 点数额度窗口

每个窗口包含：

- Name
- Limit Points
- Every
- Unit
- Anchor
- Timezone

示例：

| Name | Limit Points | Every | Unit | Timezone |
| --- | ---: | ---: | --- | --- |
| Monthly | 5 | 1 | month | UTC |
| Daily | 1 | 1 | day | UTC |
| Hourly | 0.25 | 1 | hour | UTC |

所有窗口都会被强制执行。一次请求必须让所有窗口都有足够点数，才会被路由到这个通道。

### 第一窗口的特殊作用

第一个窗口是 primary window，会用于：

- Dashboard 可用点数
- 路由权重
- surge 归一化
- fire-sale 重置时间判断

建议把最能代表通道主要库存的窗口放在第一位，例如月窗口。

### Fire Sale

Fire sale 用于“临近窗口结束但仍有较多剩余额度”的场景。

配置项：

- Fire Sale Days：距离窗口结束多少天内才可能触发。
- Fire Sale Remaining：剩余比例高于多少才触发。
- Fire Sale Discount：触发后结算折扣。

例子：

```text
Fire Sale Days = 3
Fire Sale Remaining = 0.25
Fire Sale Discount = 0.2
```

含义：

在 primary window 结束前 3 天内，如果剩余点数比例仍高于 25%，该通道可以进入 fire-sale 状态，请求结算应用 0.2 倍折扣，并在路由中获得更高权重。

### 测试通道

在 Channels 页面点击 Test 可以手动测试通道。

注意：

- 手动测试只是明确的操作，不是后台定时探测。
- 健康页主要来自真实请求。
- 测试失败通常表示 Base URL、Provider、API Key 或模型能力不匹配。

## 点数、价格与结算

### 价格单位

TokenAltar 当前固定使用：

```text
1,000,000 tokens
```

也就是每 100 万 token 价格。

价格拆分：

- Input / 1M
- Output / 1M
- Cache / 1M

### 价格匹配顺序

一次请求结算时，价格按以下顺序查找：

1. 通道专属模型正则
2. 通道专属 `default`
3. 全局模型正则
4. 全局 `default`
5. Settings 中的 fallback 价格

### 请求前预留

请求发送到上游前，系统会先估算输入 token，并预留：

- 用户点数
- API Key spend limit
- 通道点数额度窗口

如果预留失败，请求不会发送到上游。

### 请求后结算

上游返回后，系统使用 upstream `usage` 做最终结算。

结算会考虑：

- 输入 token
- 输出 token
- 缓存 token
- 匹配到的价格
- surge multiplier
- fire-sale discount
- Provider Payout Multiplier
- 舍入设置

如果最终结算和预留不同，系统会对用户余额、Key 已消费、通道已用点数做差额调整。

如果上游在 usage details 中报告 cached prompt/input tokens，TokenAltar 会先把这部分从普通输入 token 中拆出，再按 cache 价格单独结算，避免同一批缓存命中 token 同时按 input 和 cache 重复收费。

### 提供者奖励

提供者奖励使用全局 Provider Payout Multiplier：

```text
provider_points = total_points * Provider Payout Multiplier
```

这里的提供者是承载请求的通道 owner。

### 点数额度与 token 的关系

通道额度窗口是点数窗口，不是 token 窗口。

这意味着：

- 昂贵模型会更快消耗窗口点数。
- 便宜模型会更慢消耗窗口点数。
- 同一通道可以用统一点数额度管理不同模型成本。

## 路由与可靠性

### 路由筛选顺序

一次请求大致会经历：

1. 验证 `sk-...` API Key。
2. 检查用户启用状态。
3. 检查 Key 启用、过期、消费上限。
4. 检查 Key 是否允许请求模型。
5. 取出 Key 允许的通道。
6. 过滤禁用、删除、冷却中的通道。
7. 过滤模型不匹配的通道。
8. 过滤点数窗口不足的通道。
9. 应用亲和绑定。
10. 将 fire-sale 倍率并入通道权重。
11. 在同一候选池里按 primary window 剩余点数加权选择通道。

### 自动重试

以下情况可以在客户端看到有效内容前切换通道：

- 连接错误
- `408`
- `429`
- `5xx`
- 非流式空语义回复
- 流式请求在输出有效语义前结束

失败通道会进入短暂冷却，避免立即再次命中。

### 流式请求边界

流式请求中，以下内容不算有效语义：

- heartbeat/comment frame
- usage-only metadata
- whitespace-only delta
- terminal marker

一旦真实文本或工具调用内容已经转发给客户端，TokenAltar 不会中途重放请求。这是为了避免重复输出或上下文混乱。

## 亲和规则

亲和规则用于让一类请求尽量固定到同一个通道。

### 适用场景

- 同一租户希望稳定命中同一通道。
- 同一会话希望减少通道漂移。
- prompt cache 或 cachedContent 希望复用同一上游上下文。
- 某些业务希望路由稳定可解释。

### 内置规则

系统默认内置：

- GPT Responses：基于 `prompt_cache_key`
- Claude Messages：基于 `metadata.user_id`
- Gemini Generate Content：基于 `cachedContent`

这些规则 TTL 为 3600 秒，并默认按模型家族保持 locality。

### 自定义规则字段

Affinity 页面可配置：

- Name
- Path
- Model Regex
- Source
- Source Path
- TTL
- Model-scoped key
- skip retry on failure
- switch on success

一般用户不需要配置亲和规则。管理员应只在明确知道业务请求结构时修改。

## 健康监控

### 健康事件来源

健康状态来自真实网关请求和手动测试结果。

常见状态：

- Available：成功且有有效内容。
- Empty：上游返回成功但没有有效语义内容。
- Degraded：可恢复或部分异常。
- Down：失败、不可用或明确错误。
- Gray：窗口内没有记录。

### TTFT

TTFT 是首字响应时间。

Health 页面显示的平均 TTFT 只统计成功且非空的样本。失败、空回复、降级事件会影响窗口颜色和计数，但不参与 TTFT 平均值。

### 48 个窗口

Health 页面展示最近 24 小时，每 30 分钟一个窗口，共 48 个窗口。

建议观察：

- 是否长期 Gray：说明没有流量或没有被路由。
- 是否频繁 Empty：上游可能返回空内容或协议转换不匹配。
- 是否频繁 Down：Base URL、API Key、模型或额度可能有问题。
- TTFT 是否持续升高：上游延迟或网络可能不稳定。

## 经济系统

### P2P 转账

Economy 页面可以给其他用户转点数。

需要填写：

- Recipient User ID
- Points
- Memo

转账会直接改变双方余额，但不会直接影响 provider/consumer 排行榜。

### 口令红包

红包包含：

- Phrase
- Total Points
- Parts
- Mode

Mode 支持：

- Even：均分。
- Lucky：随机。

同一个用户对同一个红包只能领取一次。

### 匿名排行

用户可以在 Economy 页面切换排行榜匿名状态。

匿名后，排行榜会隐藏用户身份，但账本和后台记录仍然保留真实关联。

## 排行榜

### Provider 榜

Provider 榜统计成功账本中，用户作为通道提供者承载的 token 总量。

它回答的是：

```text
谁贡献了最多模型供给？
```

### Consumer 榜

Consumer 榜统计成功账本中，用户消费的点数。

它回答的是：

```text
谁使用了最多模型能力？
```

### Day 和 Month

排行榜支持：

- Day
- Month

时间窗口受 `TOKENALTAR_LEADERBOARD_TIMEZONE` 影响。如果未设置，使用服务器本地时区。

## 账本

Ledger 是最重要的审计页面。

建议在以下场景查看：

- 用户质疑扣费。
- 提供者质疑收益。
- 管理员检查价格是否匹配。
- 排查为什么某个模型成本异常。
- 验证 surge/fire-sale 是否生效。

账本中的 formula note 会把输入、输出、缓存 token 价格以及倍率写成可读文本。

## 运行设置

### 邀请与初始余额

- Invite Required：是否需要邀请码。
- Default Invite Code：默认邀请码。
- Initial Admin Points：新管理员初始点数。
- Initial User Points：新普通用户初始点数。

### 价格 fallback

- Fallback Input Price / 1M
- Fallback Output Price / 1M
- Fallback Cache Price / 1M

当没有任何通道价格或全局价格匹配时，会使用 fallback。

### Surge 设置

- Surge Low Threshold
- Surge High Threshold
- Surge Idle Multiplier
- Surge Normal Multiplier
- Surge Peak Multiplier

Surge 用最近一小时点数需求和 healthy 通道 primary window 的小时化剩余供给做比较。

如果没有可用供给，Dashboard 会显示 `no_capacity`，不会直接套用 peak pricing。

### 路由设置

- Routing Max Attempts：单次请求最多尝试多少通道。
- Retry Cooldown Seconds：失败通道冷却多久。
- Fire Sale Route Weight：fire-sale 通道在路由中的额外权重。

### 启动期容量设置

- Ledger Queue Capacity
- Affinity Cache Capacity

这些设置需要重启服务才会完全生效。

### 默认 API Key 和通道模板

- Default API Key Spend Limit
- Default Channel Name
- Default Channel Provider
- Default Channel Base URL
- Default Channel Models
- Default Channel Windows JSON
- Default Fire Sale Days
- Default Fire Sale Remaining
- Default Fire Sale Discount

这些用于创建新 Key 或新通道时填充默认值。

### Provider Payout Multiplier

控制提供者奖励倍数。

允许大于 1，表示平台补贴提供者。

## API 调用示例

以下示例假设：

```text
TokenAltar 地址：http://127.0.0.1:8080
本地 API Key：sk-your-tokenaltar-key
```

### OpenAI Chat Completions

```bash
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer sk-your-tokenaltar-key' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "gpt-5.4",
    "messages": [
      {"role": "user", "content": "用三句话解释 TokenAltar。"}
    ],
    "stream": false
  }'
```

### OpenAI Responses

```bash
curl http://127.0.0.1:8080/v1/responses \
  -H 'Authorization: Bearer sk-your-tokenaltar-key' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "gpt-5.4",
    "input": "写一个中文项目简介。",
    "stream": false
  }'
```

### Anthropic Messages

```bash
curl http://127.0.0.1:8080/v1/messages \
  -H 'Authorization: Bearer sk-your-tokenaltar-key' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "claude-sonnet-4.6",
    "max_tokens": 512,
    "messages": [
      {"role": "user", "content": "总结一下这个系统的用途。"}
    ],
    "stream": false
  }'
```

### Gemini Generate Content

```bash
curl http://127.0.0.1:8080/v1beta/models/gemini-2.5-pro:generateContent \
  -H 'Authorization: Bearer sk-your-tokenaltar-key' \
  -H 'Content-Type: application/json' \
  -d '{
    "contents": [
      {
        "role": "user",
        "parts": [
          {"text": "请解释 TokenAltar 的点数机制。"}
        ]
      }
    ]
  }'
```

### 流式请求

以 OpenAI Responses 为例：

```bash
curl http://127.0.0.1:8080/v1/responses \
  -H 'Authorization: Bearer sk-your-tokenaltar-key' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "gpt-5.4",
    "input": "逐步解释路由过程。",
    "stream": true
  }'
```

## 升级、备份与迁移

### 本地部署升级

建议流程：

1. 停止服务。
2. 备份 SQLite 数据库。
3. 拉取新代码。
4. 运行 `pnpm --dir frontend build`。
5. 运行 `cargo build --release`。
6. 启动新二进制。
7. 登录控制台检查 Dashboard、Channels、Pricing、Health。

### Docker 部署升级

```bash
docker compose pull
docker compose up -d
```

如果需要本地重新构建：

```bash
docker compose up -d --build
```

### 备份 SQLite

如果使用 Docker：

```bash
docker compose stop
docker run --rm \
  -v tokenaltar-data:/data:ro \
  -v "$PWD":/backup \
  alpine sh -c 'cp /data/tokenaltar.sqlite3 /backup/tokenaltar.sqlite3.backup'
docker compose start
```

如果本地运行，直接备份：

```bash
cp tokenaltar.sqlite3 tokenaltar.sqlite3.backup
```

## 常见问题

### 为什么请求还没发出去就失败？

常见原因：

- 用户余额不足。
- API Key 已禁用。
- API Key 过期。
- API Key spend limit 不够。
- API Key 不允许该模型。
- API Key 没有允许任何可用通道。
- 所有匹配通道点数窗口不足。

### 为什么某个通道没有被选中？

可能原因：

- 通道被禁用。
- 通道状态不是 healthy。
- 通道处于冷却期。
- 模型规则不匹配。
- 点数窗口不足。
- API Key 没有授权该通道。
- 亲和规则绑定到了其他通道。

### 为什么价格不是我预期的？

检查顺序：

1. 该通道是否有专属模型正则。
2. 该通道是否有专属 `default`。
3. 全局是否有模型正则。
4. 全局是否有 `default`。
5. Settings 中 fallback 是否生效。
6. 账本 formula note 是否显示了具体价格。

### 为什么提供者收益和消费者扣费不同？

提供者收益由 Provider Payout Multiplier 控制：

```text
provider_points = total_points * Provider Payout Multiplier
```

如果该值不是 `1.0`，收益和扣费自然不同。

### 为什么 Health 页面很多 Gray？

Gray 表示对应窗口没有记录。通常不是错误，只说明该通道在对应时间窗口没有真实请求或测试事件。

### 为什么排行榜没有显示转账或红包？

排行榜只统计成功的模型账本记录。P2P 转账和红包会改变余额，但不直接进入 provider/consumer 排行榜。

### 修改 Settings 后为什么没有生效？

大多数请求期设置会立即生效。

但以下类型通常需要重启：

- Ledger Queue Capacity
- Affinity Cache Capacity

另外，正在进行中的请求不会被新设置 retroactively 改写。

## 排障清单

### 登录失败

检查：

- 邮箱和密码是否正确。
- 用户是否被禁用。
- 是否连接到了预期数据库。
- 首次管理员环境变量是否只在空库初始化时生效。

### 注册失败

检查：

- Settings 是否开启 Invite Required。
- 邀请码是否正确。
- 用户邮箱是否已存在。

### API 返回 unauthorized

检查：

- 是否使用 `Authorization: Bearer sk-...`。
- 是否误用了控制台 `ta-...` token。
- API Key 是否被删除、禁用或旋转。
- 所属用户是否被停用。

### API 返回 no healthy channel

检查：

- 是否至少有一个启用通道。
- 通道状态是否 healthy。
- 通道模型是否匹配。
- 通道点数窗口是否还有剩余。
- API Key 是否允许这些通道。
- Base URL 和上游 API Key 是否正确。

### API 返回 insufficient points

检查：

- 用户余额是否足够。
- 请求输入是否过大。
- 模型输入价格是否过高。
- API Key spend limit 是否过低。

### 通道测试失败

检查：

- Provider 是否选对。
- Base URL 是否正确。
- 上游 API Key 是否有效。
- 上游是否支持通道填写的模型。
- 网络是否能访问上游服务。

### Docker 启动后数据为空

检查：

- 是否误用了新的 Compose 项目名导致创建了新卷。
- 当前卷是否为 `tokenaltar-data`。
- 是否执行过 `docker compose down -v`。
- 是否需要从旧 `*_tokenaltar-data` 卷迁移。

### 前端不是最新版本

本地部署检查：

```bash
pnpm --dir frontend build
cargo build --release
```

Docker 部署检查：

```bash
docker compose up -d --build
```

如果只替换 Rust 二进制但没有重新构建前端，控制台可能仍是旧版嵌入资源。

## 推荐运维习惯

- 生产环境使用长随机管理员密码。
- 开启邀请注册，避免公开注册被滥用。
- 为不同应用创建不同 API Key。
- 给自动化任务设置 spend limit。
- 为高价值通道设置合理点数窗口。
- 定期检查 Health 页面里的 empty/down 事件。
- 用 Ledger 作为结算争议依据。
- 升级前备份 SQLite。
- Docker 部署不要删除 `tokenaltar-data` 卷。
- 上游 API Key 泄露后立即更新通道密钥，并旋转相关本地 API Key。
