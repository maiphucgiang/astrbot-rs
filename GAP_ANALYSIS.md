# AstrBot Python → Rust 功能差距分析

> 基于原版 AstrBot (https://github.com/AstrBotDevs/AstrBot) HEAD 与 astrbot-rs HEAD `c324af4` 对比
> 分析时间：2026-04-30

## 一、Pipeline 阶段对比

| 阶段 | Python 原版 | Rust 当前 | 状态 |
|---|---|---|---|
| WakingCheckStage | 检查是否需要唤醒 | ✅ WakingCheckStage | 已实现 |
| WhitelistCheckStage | 群聊/私聊白名单 | ✅ WhitelistCheckStage | 已实现 |
| SessionStatusCheckStage | 检查会话整体启用 | ❌ 未实现 | **缺失** |
| RateLimitStage | 频率限制 | ✅ RateLimitStage | 已实现 |
| ContentSafetyCheckStage | 内容安全 | ✅ ContentSafetyCheckStage (百度 AIP) | 已实现 |
| PreProcessStage | 预处理 | ❌ 未实现 | **缺失** |
| ProcessStage | Stars 处理 / LLM 调用 | ✅ ProcessStage (Provider 路由 + Agent 调用) | 已实现 |
| ResultDecorateStage | 结果装饰 (t2i/语音转换) | ❌ 未实现 | **缺失** |
| RespondStage | 发送消息 | ✅ RespondStage (SendFn 回调) | 已实现 |

**缺失 3 个阶段**：SessionStatusCheckStage、PreProcessStage、ResultDecorateStage

## 二、核心模块对比

### 2.1 Star (插件) 系统

| 功能 | Python | Rust | 状态 |
|---|---|---|---|
| 插件加载 (Python 模块动态导入) | ✅ 完整 | ⚠️ WASM stub | WASM 插件骨架 |
| 插件配置管理 (AstrBotConfig) | ✅ 完整 | ❌ 未实现 | **缺失** |
| 插件命令注册 (@command) | ✅ 完整 | ❌ 未实现 | **缺失** |
| 插件事件监听 (@event) | ✅ 完整 | ❌ 未实现 | **缺失** |
| 插件权限管理 | ✅ 完整 | ❌ 未实现 | **缺失** |
| 插件热更新/重载 | ✅ 完整 | ❌ 未实现 | **缺失** |
| 插件市场 (自动下载安装) | ✅ 完整 | ❌ 未实现 | **缺失** |
| 插件依赖管理 (pip install) | ✅ 完整 | ❌ 不适用 | 需 Rust 方案 |

### 2.2 Provider (大模型 Provider)

| Provider | Python | Rust | 状态 |
|---|---|---|---|
| OpenAI | ✅ | ✅ | 已实现 |
| Anthropic Claude | ✅ | ✅ | 已实现 |
| Google Gemini | ✅ | ✅ | 已实现 |
| DashScope (阿里云) | ✅ | ✅ | 已实现 |
| Baidu (千帆/文心) | ✅ | ✅ | 已实现 |
| Moonshot (Kimi) | ✅ | ✅ | 已实现 |
| DeepSeek | ✅ | ❌ | **缺失** |
| Groq | ✅ | ❌ | **缺失** |
| Together AI | ✅ | ❌ | **缺失** |
| Fireworks | ✅ | ❌ | **缺失** |
| Cohere | ✅ | ❌ | **缺失** |
| Perplexity | ✅ | ❌ | **缺失** |
| OpenRouter | ✅ | ❌ | **缺失** |
| ZeroOneAI (01.AI) | ✅ | ❌ | **缺失** |
| SiliconFlow | ✅ | ✅ | 已实现 |
| 本地 LLM (ollama/llama.cpp) | ✅ | ❌ | **缺失** |

### 2.3 Platform (平台适配器)

| 平台 | Python | Rust | 状态 |
|---|---|---|---|
| QQ (OneBot) | ✅ | ✅ | 已实现 |
| QQ Official | ✅ | ✅ | 已实现 |
| Telegram | ✅ | ✅ | 已实现 |
| Discord | ✅ | ❌ | **缺失** |
| 飞书 (Lark) | ✅ | ⚠️ skeleton | 骨架 |
| 企业微信 (Wecom) | ✅ | ⚠️ skeleton | 骨架 |
| 微信 (Wechat) | ✅ | ⚠️ skeleton | 骨架 |
| 钉钉 (Dingtalk) | ✅ | ⚠️ skeleton | 骨架 |
| LINE | ✅ | ❌ | **缺失** |
| Matrix | ✅ | ✅ | 已实现 |
| Slack | ✅ | ❌ | **缺失** |
| Mattermost | ✅ | ❌ | **缺失** |
| Misskey | ✅ | ❌ | **缺失** |
| Kook | ✅ | ❌ | **缺失** |
| Satori | ✅ | ❌ | **缺失** |
| WebChat (WebSocket) | ✅ | ✅ | 已实现 |
| Webhook | ✅ | ✅ | 已实现 |

### 2.4 Agent 系统

| 功能 | Python | Rust | 状态 |
|---|---|---|---|
| ToolLoop Agent | ✅ | ✅ | 已实现 |
| Coze Runner | ✅ | ✅ | 已实现 |
| Dify Runner | ✅ | ✅ | 已实现 |
| DashScope Agent | ✅ | ✅ | 已实现 |
| DeerFlow Workflow | ✅ | ✅ | 已实现 |
| 主 Agent (AstrMainAgent) | ✅ | ⚠️ 部分 | 需完善 |
| Agent 上下文管理 | ✅ | ⚠️ 部分 | 需完善 |
| Agent 工具执行器 | ✅ | ⚠️ 部分 | 需完善 |
| Agent Hook 系统 | ✅ | ❌ | **缺失** |
| Agent Handoff | ✅ | ❌ | **缺失** |

### 2.5 工具 (Tools)

| 工具 | Python | Rust | 状态 |
|---|---|---|---|
| Web 搜索 | ✅ | ✅ | 已实现 |
| 内容审核 | ✅ | ✅ | 已实现 |
| 图片生成 (T2I) | ✅ | ✅ | 已实现 |
| 语音合成 (TTS) | ✅ | ✅ | 已实现 |
| 语音识别 (STT) | ✅ | ✅ | 已实现 |
| Computer Use | ✅ | ✅ | 已实现 |
| 定时任务工具 | ✅ | ❌ | **缺失** |
| 知识库工具 | ✅ | ❌ | **缺失** |
| 消息工具 | ✅ | ❌ | **缺失** |

### 2.6 知识库 (Knowledge Base)

| 功能 | Python | Rust | 状态 |
|---|---|---|---|
| 向量数据库 (SQLite) | ✅ | ❌ | **缺失** |
| 知识库管理 | ✅ | ❌ | **缺失** |
| Embedding 模型 | ✅ | ⚠️ trait 定义 | 有 trait 无实现 |
| Rerank 模型 | ✅ | ⚠️ trait 定义 | 有 trait 无实现 |
| 文档导入 (PDF/Word/TXT) | ✅ | ❌ | **缺失** |
| 检索增强生成 (RAG) | ✅ | ⚠️ 骨架 | 有模块未接线 |

### 2.7 定时任务 (Cron)

| 功能 | Python | Rust | 状态 |
|---|---|---|---|
| Cron 表达式解析 | ✅ | ❌ | **缺失** |
| 定时任务管理器 | ✅ | ❌ | **缺失** |
| 事件触发 | ✅ | ❌ | **缺失** |

### 2.8 其他核心功能

| 功能 | Python | Rust | 状态 |
|---|---|---|---|
| 会话管理 (Conversation Manager) | ✅ | ❌ | **缺失** |
| 消息历史管理 | ✅ | ⚠️ 部分 | 有 session history |
| 配置管理 (AstrBotConfig) | ✅ | ⚠️ 部分 | 有 config 模块 |
| 文件 Token 服务 | ✅ | ❌ | **缺失** |
| 备份/恢复 | ✅ | ❌ | **缺失** |
| 更新器 (Updator) | ✅ | ❌ | **缺失** |
| 事件总线 (Event Bus) | ✅ | ⚠️ 部分 | 有事件机制 |
| 多语言 (i18n) | ✅ | ❌ | **缺失** |
| 数据库 (SQLite/SQLAlchemy) | ✅ | ⚠️ skeleton | 有 db 模块 |
| KV 存储 | ✅ | ❌ | **缺失** |
| 指标/监控 (Metrics) | ✅ | ❌ | **缺失** |
| 日志系统 | ✅ | ⚠️ 部分 | 有 log 模块 |
| 子 Agent 编排器 | ✅ | ❌ | **缺失** |

## 三、Dashboard 对比

| 功能 | Python (Vue3) | Rust (Vue3) | 状态 |
|---|---|---|---|
| Web UI 管理后台 | ✅ | ✅ | 已实现 |
| 配置编辑 | ✅ | ⚠️ 部分 | 需完善 |
| 插件管理 | ✅ | ❌ | **缺失** |
| 提供商管理 | ✅ | ⚠️ 部分 | 需完善 |
| 平台适配器管理 | ✅ | ⚠️ 部分 | 需完善 |
| 知识库管理 | ✅ | ❌ | **缺失** |
| 定时任务管理 | ✅ | ❌ | **缺失** |
| 日志查看 | ✅ | ❌ | **缺失** |
| 统计/监控面板 | ✅ | ❌ | **缺失** |

## 四、API 对比

| API | Python | Rust | 状态 |
|---|---|---|---|
| 消息组件 API | ✅ | ✅ | 已实现 |
| 平台 API | ✅ | ⚠️ 部分 | 需完善 |
| Provider API | ✅ | ⚠️ 部分 | 需完善 |
| Star API | ✅ | ❌ | **缺失** |
| 工具 API | ✅ | ⚠️ 部分 | 需完善 |
| 事件 API | ✅ | ⚠️ 部分 | 需完善 |

## 五、总结

### 已实现 (相对完整)
- ✅ Pipeline 6/9 阶段
- ✅ 5 种 Agent Runner
- ✅ MCP Client
- ✅ RAG 骨架 (trait 定义)
- ✅ 核心 Provider (OpenAI/Claude/Gemini/DashScope/Baidu/Moonshot/SF)
- ✅ QQ/Telegram/Matrix/WebChat/Webhook 平台
- ✅ Computer Use + TTS/STT + T2I
- ✅ Web 搜索 + 内容审核
- ✅ Dashboard 基础
- ✅ CI/CD (GitHub Actions)

### 主要缺失
1. **插件系统**：Star/Plugin 完整生命周期（加载、配置、命令、事件、热更新）
2. **更多 Provider**：DeepSeek、Groq、Together、Fireworks、Cohere、Perplexity、OpenRouter、ZeroOneAI、本地 LLM
3. **更多平台**：Discord、LINE、Slack、Mattermost、Misskey、Kook、Satori
4. **知识库完整实现**：向量数据库、文档导入、Embedding/Rerank 具体实现
5. **定时任务**：Cron 管理器
6. **会话管理**：Conversation Manager、消息历史
7. **系统功能**：备份恢复、更新器、KV 存储、Metrics、子 Agent 编排
8. **Dashboard**：插件管理、知识库、定时任务、日志、监控
9. **3 个 Pipeline 阶段**：SessionStatusCheck、PreProcess、ResultDecorate
10. **本地 LLM**：ollama/llama.cpp 桥接

### Phase 5 建议优先级

**P0 (高优先级)**：
1. 本地 LLM (ollama bridge) — piexianclaw 已分配
2. DeepSeek Provider — 热门 Provider
3. Discord 平台 — 热门平台
4. ResultDecorateStage (t2i/tts 转换)

**P1 (中优先级)**：
5. 插件系统完整实现
6. 知识库向量数据库实现
7. 会话管理 (Conversation Manager)
8. 定时任务 (Cron)
9. 更多 Provider (Groq/Together/Fireworks)
10. 更多平台 (LINE/Slack/Kook)

**P2 (低优先级)**：
11. 备份恢复
12. 更新器
13. KV 存储
14. Metrics/监控
15. 子 Agent 编排器
16. 多语言 i18n
