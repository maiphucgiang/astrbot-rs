# AstrBot Feishu Integration

飞书（Lark）集成增强模块，为 AstrBot 提供：

1. **平台适配器** — 飞书 IM 消息收发、Webhook 事件处理
2. **知识库 RAG 数据源** — 飞书文档/多维表读写
3. **日程提醒** — 飞书日历事件查询与提醒
4. **群消息检索** — 飞书群聊历史消息搜索

## 模块结构

```
astrbot-feishu/
├── Cargo.toml
├── src/
│   ├── lib.rs          # 入口、错误类型、初始化
│   ├── models.rs       # 共享数据模型
│   ├── auth.rs         # OAuth 认证、token 缓存、Webhook 验签
│   ├── platform/       # 飞书平台适配器
│   │   └── mod.rs      # FeishuAdapter、MessageHandler trait
│   ├── knowledge/      # 知识库 RAG 数据源
│   │   └── mod.rs      # DocClient、BitableClient、KnowledgeSource trait
│   ├── calendar/       # 日程提醒
│   │   └── mod.rs      # CalendarClient、ReminderConfig
│   └── search/         # 群消息检索
│       └── mod.rs      # GroupMessageSearch、SearchQuery
├── tests/
│   └── integration_test.rs  # Mock server 集成测试
└── examples/
    └── feishu_bot.rs   # 示例机器人
```

## 核心 Trait

### KnowledgeSource
```rust
#[async_trait]
pub trait KnowledgeSource: Send + Sync {
    async fn fetch_content(&self, document_id: &str) -> Result<String>;
    async fn list_items(&self) -> Result<Vec<KnowledgeItem>>;
}
```

### MessageHandler
```rust
#[async_trait]
pub trait MessageHandler: Send + Sync {
    async fn handle_message(&self, msg: &IncomingMessage) -> Result<Option<OutgoingMessage>>;
}
```

## 接入 AstrBot Workspace

待红毛确认 workspace 结构后，将本 crate 接入 `Cargo.toml` workspace：

```toml
[workspace]
members = [
    "crates/astrbot-core",
    "crates/astrbot-platform",
    "crates/astrbot-feishu",  # <-- 新增
]
```

并在 `astrbot-core` 中接入知识库数据源：

```rust
use astrbot_feishu::{DocClient, BitableClient, KnowledgeSource};

// RAG 流水线接入
let doc_source = DocClient::new(auth.clone());
let bitable_source = BitableClient::new(auth.clone());
```

## 飞书 OpenAPI 覆盖

| 能力 | API | 状态 |
|------|-----|------|
| 认证 | `auth/v3/tenant_access_token/internal` | ✅ |
| 发消息 | `im/v1/messages` | ✅ |
| 回复消息 | `im/v1/messages/{id}/reply` | ✅ |
| 读文档 | `docx/v1/documents/{id}/raw_content` | ✅ |
| 读多维表 | `bitable/v1/apps/{token}/tables/{id}/records` | ✅ |
| 写多维表 | `bitable/v1/apps/{token}/tables/{id}/records` | ✅ |
| 搜多维表 | `bitable/v1/apps/{token}/tables/{id}/records/search` | ✅ |
| 读日程 | `calendar/v4/calendars/{id}/events` | ✅ |
| 写日程 | `calendar/v4/calendars/{id}/events` | ✅ |
| 查忙闲 | `calendar/v4/freebusy/batch_get` | ✅ |
| 搜消息 | `im/v1/messages/search` | ✅ |
| Webhook验签 | HMAC-SHA256 | ✅ |

## 环境变量

```bash
FEISHU_APP_ID=cli_xxx
FEISHU_APP_SECRET=xxx
FEISHU_ENCRYPT_KEY=xxx      # Webhook 加密密钥
FEISHU_VERIFICATION_TOKEN=xxx  # Webhook 验证令牌
```

## 测试

```bash
cargo test
```

集成测试使用 wiremock 模拟飞书 API，无需真实 App ID。

## M3 待办

- [ ] 接入 AstrBot workspace（等基线）
- [ ] 接入 `astrbot-core` 的 `KnowledgeBase` trait
- [ ] 飞书 Bot 平台适配器（`PlatformAdapter` trait 实现）
- [ ] 定时任务：日程提醒推送
- [ ] 向量索引：飞书文档主动同步到本地向量库
