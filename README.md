# AstrBot Rust

> **警告：当前为完全重构中的开发版本，不建议用于生产环境。**

将 [AstrBot](https://github.com/Soulter/AstrBot)（Python 版本）完全重构为 Rust 版本，目标复刻所有核心功能。

- **架构**：Workspace 多 Crate，Tokio 异步运行时
- **测试状态**：`cargo test --workspace` — **~528 passed, 0 failed, 2 ignored**
- **协议**：AGPL-3.0（与原版一致）

---

## 架构概览

```
astrbot/
├── crates/
│   ├── astrbot-core/       # Pipeline、Agent、MCP、RAG、Provider Trait
│   ├── astrbot-persona/    # 人格管理（PersonaManager）
│   ├── astrbot-security/   # 内容安全、速率限制
│   ├── astrbot-ux/         # 用户体验
│   ├── astrbot-plugin/     # WASM 插件运行时、PluginManager、Installer
│   ├── astrbot-provider/   # ProviderManager、LLM 客户端封装
│   ├── astrbot-platform/   # PlatformAdapter Trait + 平台实现
│   ├── astrbot-dashboard/  # Axum Web 服务、SSE、REST API、WebChat
│   └── astrbot-cli/        # clap 命令行、BotRuntime
```

### 核心子系统

| 子系统 | 状态 | 说明 |
|--------|------|------|
| **Pipeline 9 Stage** | ✅ 完成 | Waking→Whitelist→SessionStatus→RateLimit→ContentSafety→PreProcess→Process→ResultDecorate→Respond |
| **Provider Trait** | ✅ 完成 | Chat / TTS / STT / Embedding / Rerank / T2I |
| **Agent Runner** | ✅ 完成 | ToolLoop / Coze / Dify / Dashscope / DeerFlow |
| **MCP 协议** | ✅ 完成 | Stdio/SSE Transport、JSON-RPC 2.0 |
| **RAG 系统** | ✅ 完成 | Document/Parser/Splitter/VectorStore/Retriever |
| **WASM 插件** | 🟡 骨架 | PluginManager/Loader/Installer 就位，生态待建 |
| **Computer Use** | ✅ 完成 | FsTool / PythonShellTool / ComputerUseTool |
| **TTS/STT** | ✅ 完成 | OpenAI/Azure/Edge/FishAudio + Whisper/SenseVoice |
| **Web 搜索** | ✅ 完成 | Brave / Tavily / Baidu |
| **Dashboard** | 🟡 部分 | 后端 API + SSE + WebSocket 完成，前端骨架（`index.html`） |
| **平台适配器** | 🟡 骨架 | 11 个平台 Trait 定义 + Telegram 实现，其余待补 |

---

## 快速开始

### 1. 克隆仓库

```bash
git clone https://github.com/你的组织/astrbot-rs.git
cd astrbot-rs
```

### 2. 编译与测试

```bash
# 编译全 workspace
cargo build --workspace

# 运行全部测试（~528 passed）
cargo test --workspace

# 编译并启动 CLI
cargo run -- run --config data/config.json
```

### 3. 启动 Dashboard + WebChat

```bash
cargo run -- dashboard --port 6185
# 浏览器打开 http://localhost:6185
# WebChat 通过 WebSocket 连接 `/ws/chat`
```

### 4. 配置 Provider

编辑 `data/config.json`：

```json
{
  "providers": [
    {
      "id": "openai_default",
      "provider_type": "openai",
      "api_key": "sk-...",
      "model": "gpt-4o-mini",
      "base_url": "https://api.openai.com",
      "enabled": true
    }
  ],
  "platforms": [
    {
      "id": "telegram_bot",
      "type": "telegram",
      "token": "你的 Bot Token",
      "enabled": true
    }
  ]
}
```

---

## 各 Crate 详细说明

### `astrbot-core`

项目核心，包含所有业务逻辑骨架。

- **`pipeline/`** — 9 Stage Pipeline，`PipelineScheduler` 调度执行
- **`agent/`** — 5 种 Agent Runner，`AgentRegistry` 注册管理
- **`mcp/`** — MCP 客户端，`McpClient` + `StdioTransport` / `SseTransport`
- **`rag/`** — RAG 系统，`DocumentParser` → `TextSplitter` → `VectorStore` → `Retriever`
- **`tools/`** — 工具集，包括 `WebSearchTool`、`KbSearchTool`、`ComputerUseTool`
- **`computer/`** — Computer Use，`FsTool` + `PythonShellTool` + `ComputerUseTool`
- **`message/`** — 消息模型，`AstrBotMessage`、`MessageChain`、`MessageEventResult`
- **`platform/`** — 平台适配器 Trait，`PlatformAdapter` + `MessageSource`
- **`testing/`** — 测试工具，`MockProvider`、`MockVectorStore`

### `astrbot-plugin`

插件系统。

- **`manager.rs`** — `PluginManager`（load/init/start/stop/unload）
- **`loader.rs`** — `PluginLoader`（scan/instantiate/reload）
- **`installer.rs`** — `PluginInstaller`（install/uninstall + pip 包管理）
- **`wasm/`** — WASM 运行时（`wasmi`）

### `astrbot-provider`

Provider 管理 + 客户端封装。

- **`client.rs`** — `ProviderManager`（注册、激活、fallback、health check）
- **`openai.rs`** — `OpenAiProvider`（真实 reqwest HTTP 调用）
- **`sources/`** — 各 Provider 源实现（moonshot/deepseek/groq/...）

### `astrbot-dashboard`

Web 面板。

- **`server.rs`** — Axum 路由，所有 REST API + WebSocket `/ws/chat`
- **`app_state.rs`** — `AppState`（真实 `PluginManager` / `ProviderManager` 引用）
- **`sse.rs`** — SSE 实时推送（tokio broadcast，30 秒心跳）
- **`api.rs`** — API handler 集合
- **`dashboard/dist/index.html`** — WebChat 最小前端

### `astrbot-cli`

命令行入口。

- **`main.rs`** — clap 子命令解析（init/run/config/status/plugin/validate/dashboard/test）
- **`runtime.rs`** — `BotRuntime`（启动时 build 9-stage pipeline）

---

## 测试

```bash
# 全 workspace
cargo test --workspace

# 单个 crate
cargo test -p astrbot-core
cargo test -p astrbot-provider

# 集成测试（需要 OPENAI_API_KEY）
OPENAI_API_KEY=sk-xxx cargo test -p astrbot-provider --test test_openai_real

# E2E 测试
cargo test --test e2e_basic_chat
```

---

## CLI 命令

```bash
astrbot init              # 初始化配置文件
astrbot run               # 启动 Bot（启动 Pipeline + 平台适配器）
astrbot config get        # 查看配置
astrbot config set        # 修改配置
astrbot status            # 查看运行状态
astrbot plugin list       # 列出插件
astrbot plugin install    # 安装插件
astrbot validate          # 验证配置
astrbot dashboard         # 启动 Dashboard
astrbot test <provider>   # 测试 Provider
```

---

## 与 Python 原版的差距

| 维度 | Python 原版 | Rust 版本 | 差距 |
|------|-----------|----------|------|
| 平台适配器 | 18+ 平台真实接入 | 11 个骨架 + Telegram 实现 | 主因 |
| Dashboard 前端 | 完整 Vue SPA | 最小 HTML + API | 较大 |
| Provider 真实调用 | 20+ 真实接入 | 代码完整，待 API key 验证 | 中等 |
| 插件生态 | 200+ 社区插件 | WASM 骨架 + Installer | 较大 |
| 微信生态 | 公众号/企微/个人号 | 0 | 严重 |

详见 `docs/GAP_ANALYSIS.md`（待建）。

---

## 技术栈

- **语言**：Rust 1.80+
- **异步运行时**：Tokio
- **Web 框架**：Axum
- **序列化**：serde
- **CLI**：clap
- **WASM 运行时**：wasmi
- **HTTP 客户端**：reqwest
- **日志**：tracing

---

## 贡献

当前处于密集重构期，代码变动频繁。欢迎通过 Issue 讨论架构方向，暂不接收大功能 PR。

---

## 许可证

[AGPL-3.0](LICENSE)
