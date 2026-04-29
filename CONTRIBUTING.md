# Contributing to AstrBot Rust

## 快速开始

### 环境准备

| 依赖 | 最低版本 |
|------|---------|
| Rust | 1.75+ |
| Git | 2.x |
| SQLite | 3.x |
| PostgreSQL (可选) | 14+ |

```bash
git clone https://github.com/<你的用户名>/astrbot-rs.git
cd astrbot-rs
cargo build
cargo test --workspace
```

### 代码检查（提交前必须执行）

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

## Mock 测试示例

```rust
use astrbot_provider::{MockProvider, ChatRequest, ChatMessage};

#[test]
fn test_mock_chat() {
    let provider = MockProvider::default();
    let req = ChatRequest::new(vec![
        ChatMessage::system("你是一个助手"),
        ChatMessage::user("你好"),
    ]);
    let resp = provider.chat(req).unwrap();
    assert!(!resp.content.is_empty());
}
```

Mock 测试原则：
1. 隔离外部依赖 — 所有 HTTP/DB/文件操作用 mock
2. 确定性 — 固定返回值，每次必过
3. 并行安全 — 用 `tempfile` 创建临时目录

## PR 流程

```bash
# 1. 创建分支（命名：feat/fix/docs/refactor/perf/test）
git checkout -b feat/add-xxx

# 2. 开发 + 提交（Conventional Commits）
git commit -m "feat(provider): add Coze platform adapter

- 实现 Coze Bot API
- 添加 mock 测试

Closes #123"

# 3. 检查清单执行（check/test/clippy/fmt）

# 4. Push + PR
git push origin feat/add-xxx
```

### 提交信息规范

```
<类型>(<范围>): <描述>

<正文>

<脚注>
```

类型：feat / fix / docs / style / refactor / perf / test / chore

### PR 描述模板

```markdown
## 变更内容
简要描述做了什么。

## 关联 Issue
Closes #<issue编号>

## 测试
- [ ] cargo test --workspace 通过
- [ ] 新增测试已覆盖改动
- [ ] clippy / fmt 检查通过
```

## 代码风格

- **格式化**：`cargo fmt --all`（默认配置，不自定义 rustfmt.toml）
- **Lint**：`cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - 警告必须修复，不允许 #[allow(...)] 除非有充分理由
- **文档**：所有 public API 必须有 `///` 文档注释

## 命名规范

| 类型 | 规范 | 示例 |
|------|------|------|
| 模块/文件 | snake_case | `chat_provider.rs` |
| 类型/结构体 | PascalCase | `ChatProvider` |
| trait | PascalCase | `PlatformAdapter` |
| 函数/方法 | snake_case | `send_message()` |
| 常量 | SCREAMING_SNAKE_CASE | `MAX_RETRY_COUNT` |
