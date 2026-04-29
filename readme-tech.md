# AstrBot Rust — 技术架构

> 项目：astrbot-rs | 版本：0.1.0 | 协议：AGPL-3.0

---

## 1. 项目概述

AstrBot Rust 是 **AstrBot 的 Rust 重写版本** —— 一个支持多平台接入、多模型 Provider 切换、插件扩展、RAG 知识库、Agent 智能体编排的 **异步 AI 聊天机器人框架**。

相比原版的核心改进：

- **性能**：基于 `tokio` 异步运行时，单实例可承载高并发平台消息。
- **安全**：`astrbot-security` crate 提供消息过滤、敏感词拦截、合规审查。
- **扩展性**：WASM 插件沙箱 + 原生动态库双模式，支持热插拔。
- **企业级**：飞书文档/多维表/RAG 集成、企微/钉钉平台适配、Dashboard 管理后台。

---

## 2. Workspace 架构图

```
astrbot-rs/
├── Cargo.toml                  # workspace 根，统管 10 crate
│
└── crates/
    ├── astrbot-core            # 核心引擎：消息管道、配置、事件总线、RAG、Agent
    │   ├── agent/              # Agent 编排 & Tool Loop
    │   ├── config/             # 配置加载 + 热重载
    │   ├── db/                 # SQLite / Postgres 持久层
    │   ├── event/              # 内部事件总线
    │   ├── i18n/               # 多语言支持
    │   ├── mcp/                # MCP (Model Context Protocol) 接入
    │   ├── rag/                # RAG 检索增强生成
    │   ├── persona/            # 人格系统（多预设 + 情绪状态机）
    │   ├── plugin/             # WASM 插件运行时
    │   ├── wasm/               # WASM 沙箱边界
    │   ├── voice/              # 语音输入/输出 (STT/TTS)
    │   ├── t2i/                # 文生图调用链路
    │   └── vector_store/       # 向量存储后端
    │
    ├── astrbot-platform        # 平台适配层（统一 adapter trait）
    │   ├── adapter.rs          # PlatformAdapter 抽象
    │   ├── framework.rs        # 平台生命周期管理
    │   └── {platform}.rs       # 各平台具体实现
    │
    ├── astrbot-provider        # LLM Provider 接入层
    │   ├── client.rs           # 统一 HTTP 客户端
    │   ├── registry.rs         # Provider 注册中心
    │   ├── openai_compatible.rs # OpenAI 协议兼容基座
    │   ├── template.rs         # Prompt 模板引擎
    │   └── {provider}.rs       # 各 Provider 实现
    │
    ├── astrbot-security        # 安全与合规
    │   └── 访问控制 / 内容过滤 / 审计日志
    │
    ├── astrbot-persona         # 人格与上下文管理
    │   └── SQLite 持久化 + 自动切换策略
    │
    ├── astrbot-ux              # 用户体验层
    │   ├── HelpSystem          # 动态帮助生成
    │   └── ErrorTranslator     # 多语言错误提示
    │
    ├── astrbot-dashboard       # Web 管理后台（Axum + Vue）
    │   ├── server.rs           # REST API
    │   └── 前端 Vue 视图
    │
    ├── astrbot-feishu          # 飞书生态增强
    │   └── 文档/多维表/日程/群消息检索
    │
    ├── astrbot-plugin          # 插件 SDK（供外部开发者使用）
    │
    └── astrbot-cli             # 命令行入口
        └── astrbot <run|config|status>
```

### 核心数据流

```
[平台消息] → PlatformAdapter → AstrMessage → EventBus → Core Pipeline
                                                        ↓
                              [人格切换] ← Persona Manager ← [安全审查]
                                                        ↓
                              Provider Registry → LLM API → [RAG 检索]
                                                        ↓
                              [插件 Hook] ← WASM Sandbox ← [MCP Tools]
                                                        ↓
                              响应消息 → PlatformAdapter → [用户]
```

---

## 3. 快速开始

### 3.1 环境要求

| 依赖 | 最低版本 | 说明 |
|------|---------|------|
| Rust | 1.75+ | `rustup` 安装 |
| Cargo | 跟随 Rust | 构建工具 |
| SQLite | 3.x | 默认数据库（可选 Postgres） |

### 3.2 编译

```bash
git clone https://github.com/Last-emo-boy/astrbot-rs.git
cd astrbot-rs

# 编译整个 workspace
cargo build --release

# 仅编译 CLI
cargo build --release -p astrbot-cli

# 开发模式（增量编译更快）
cargo build
```

### 3.3 运行

```bash
# 使用默认配置启动
cargo run --bin astrbot -- run

# 指定配置文件
cargo run --bin astrbot -- run --config ./config.json

# 后台模式
cargo run --bin astrbot -- run --daemon

# 查看状态
cargo run --bin astrbot -- status --detailed

# 修改配置项
cargo run --bin astrbot -- config --set "provider.default=groq"
```

### 3.4 验证

```bash
# 全 workspace 编译检查
cargo check

# 运行全部测试
cargo test

# 仅运行核心测试
cargo test -p astrbot-core

# 代码格式化检查
cargo fmt --check

# Clippy 静态分析
cargo clippy -- -D warnings
```

---

## 4. 平台覆盖清单

AstrBot Rust 当前支持 **18+ 即时通讯平台**，通过统一的 `PlatformAdapter` trait 接入。

### 国内平台

| 平台 | 状态 | 特性 |
|------|------|------|
| **QQ** | ✅ | OneBot / go-cqhttp 兼容 |
| **QQ 官方** | ✅ | QQ Bot 官方 API |
| **微信个人号** | ✅ | Web 协议 |
| **微信客服** | ✅ | 企业微信客服回调 |
| **企业微信** | ✅ | 自建应用 + 群机器人 |
| **钉钉** | ✅ | 群机器人 + 企业内部应用 |
| **飞书 / Lark** | ✅ | 完整生态（IM + 文档 + 日历） |
| **KOOK** | ✅ | 语音社区平台 |

### 海外平台

| 平台 | 状态 | 特性 |
|------|------|------|
| **Telegram** | ✅ | Bot API / Webhook |
| **Discord** | ✅ | Gateway + Slash Commands |
| **Slack** | ✅ | Socket Mode / Events API |
| **LINE** | ✅ | Messaging API |
| **Matrix** | ✅ | 去中心化联邦协议 |
| **Mattermost** | ✅ | 企业自托管 Slack 替代 |
| **Misskey** | ✅ | 联邦宇宙平台 |

### 通用接入

| 平台 | 状态 | 特性 |
|------|------|------|
| **Webhook** | ✅ | 通用 HTTP 回调适配 |
| **WebChat** | ✅ | 内置 Web 聊天界面 |
| **Satori** | ✅ | 通用聊天协议（Chronocat） |

### 平台适配架构

所有平台实现统一的 `PlatformAdapter` trait：

```rust
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    async fn start(&self, event_bus: EventBus) -> Result<()>;
    async fn send_message(&self, target: &str, msg: AstrMessage) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}
```

新增平台仅需实现该 trait 并在 `framework.rs` 中注册即可，无需改动 Core。

---

## 5. 模块状态速查

| Crate | 编译 | 测试 | 说明 |
|-------|------|------|------|
| astrbot-core | ✅ | ✅ | 消息管道、配置、RAG、Agent |
| astrbot-platform | ✅ | ✅ | 18+ 平台适配 |
| astrbot-provider | ✅ | ✅ | 14+ Provider + OpenAI 兼容基座 |
| astrbot-security | ✅ | ✅ | 内容安全、访问控制 |
| astrbot-persona | ✅ | ✅ | 人格 SQLite 持久化 |
| astrbot-ux | ✅ | ✅ | HelpSystem + ErrorTranslator |
| astrbot-dashboard | ✅ | ✅ | Axum REST API + Vue 前端 |
| astrbot-feishu | ✅ | ✅ | 飞书文档/多维表/日程 |
| astrbot-plugin | ✅ | ✅ | 插件 SDK |
| astrbot-cli | ✅ | ✅ | 命令行入口 |

---

*文档由 Soulclawter 编写 | Phase 4 基础设施收尾*
