# AstrBot Rust — Provider 配置指南

## 支持的 Provider 列表

### 国际 Provider

| Provider | 类型 | 状态 | 配置 key |
|---------|------|------|---------|
| Anthropic | Claude 系列 | ✅ 可用 | `anthropic` |
| Gemini | Google 系列 | ✅ 可用 | `gemini` |
| Groq | Llama/Mistral 等 | ✅ 可用 | `groq` |
| OpenRouter | 聚合路由 | ✅ 可用 | `openrouter` |
| Together | 开源模型 | ✅ 可用 | `together` |
| Azure | OpenAI Azure | ✅ 可用 | `azure` |
| Fireworks | 开源/专用 | ✅ 可用 | `fireworks` |
| Perplexity | Sonar 系列 | ✅ 可用 | `perplexity` |

### 国内 Provider

| Provider | 类型 | 状态 | 配置 key |
|---------|------|------|---------|
| 百度千帆 | ERNIE 系列 | ✅ 可用 | `qianfan` |
| 阿里通义 | Qwen 系列 | ✅ 可用 | `tongyi` |
| 智谱 AI | ChatGLM 系列 | ✅ 可用 | `zhipu` |

### 特殊 Provider

| Provider | 说明 | 状态 | 配置 key |
|---------|------|------|---------|
| Coze | Bot 平台 | 🟡 skeleton | `coze` |
| Dify | 工作流平台 | 🟡 skeleton | `dify` |
| DashScope | 阿里模型广场 | 🟡 skeleton | `dashscope` |

---

## 各 Provider 配置示例

### Anthropic
```json
{
  "provider": "anthropic",
  "api_key": "sk-ant-xxxxx",
  "model": "claude-3-5-sonnet-20241022",
  "base_url": "https://api.anthropic.com",
  "max_tokens": 4096,
  "temperature": 0.7
}
```

### Gemini
```json
{
  "provider": "gemini",
  "api_key": "AIzaSyxxxxx",
  "model": "gemini-1.5-pro",
  "base_url": "https://generativelanguage.googleapis.com",
  "temperature": 0.7
}
```

### Groq
```json
{
  "provider": "groq",
  "api_key": "gsk_xxxxx",
  "model": "llama-3.1-70b-versatile",
  "base_url": "https://api.groq.com/openai/v1",
  "temperature": 0.7
}
```

### OpenRouter
```json
{
  "provider": "openrouter",
  "api_key": "sk-or-v1-xxxxx",
  "model": "anthropic/claude-3.5-sonnet",
  "base_url": "https://openrouter.ai/api/v1",
  "temperature": 0.7
}
```

### Azure
```json
{
  "provider": "azure",
  "api_key": "xxxxx",
  "model": "gpt-4",
  "base_url": "https://your-resource.openai.azure.com",
  "api_version": "2024-02-01",
  "deployment": "your-deployment-name"
}
```

### 百度千帆
```json
{
  "provider": "qianfan",
  "api_key": "xxxxx",
  "secret_key": "xxxxx",
  "model": "ernie-4.0-turbo-8k",
  "temperature": 0.7
}
```

### 阿里通义
```json
{
  "provider": "tongyi",
  "api_key": "sk-xxxxx",
  "model": "qwen-max",
  "base_url": "https://dashscope.aliyuncs.com",
  "temperature": 0.7
}
```

### 智谱 AI
```json
{
  "provider": "zhipu",
  "api_key": "xxxxx",
  "model": "glm-4",
  "base_url": "https://open.bigmodel.cn/api/paas/v4",
  "temperature": 0.7
}
```

---

## 核心功能说明

### Agent 框架

AstrBot Rust 内置 Agent 执行引擎，支持多轮 tool calling loop：

1. 用户输入 → LLM 判断是否需要调用 tool
2. 调用 MCP / 内置 tool → 获取结果
3. 结果回传 LLM → 生成最终回复

支持 Agent 类型：
- **ReAct Agent**: 推理-行动交替模式
- **Plan-and-Execute**: 先规划后执行
- **Coze/Dify/DeerFlow**: 第三方 Bot 平台接入（skeleton）

### MCP (Model Context Protocol)

MCP 让 LLM 能调用外部工具和数据源：

```json
{
  "mcp": {
    "servers": [
      {
        "name": "filesystem",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed"]
      },
      {
        "name": "sqlite",
        "command": "uvx",
        "args": ["mcp-server-sqlite", "--db-path", "/path/to/db.sqlite"]
      }
    ]
  }
}
```

MCP 实现特性：
- stdio transport 支持
- Server 注册与发现（Arc<RwLock<>> 管理）
- Tool 调用 pipeline 集成

### RAG (Retrieval-Augmented Generation)

RAG pipeline 支持知识库问答：

```json
{
  "rag": {
    "enabled": true,
    "vector_store": {
      "type": "memory",  // memory / qdrant / milvus
      "dimension": 768
    },
    "retriever": {
      "top_k": 5,
      "score_threshold": 0.7
    },
    "chunk_size": 512,
    "chunk_overlap": 128
  }
}
```

RAG 组件状态：
- **VectorStore**: ✅ 内存/Qdrant/Milvus 适配
- **Retriever**: ✅ 向量检索 + 重排序
- **Chunker**: ✅ 多策略分块
- **E2E Pipeline**: 🟡 测试中

### WASM 插件

WASM 插件系统支持动态加载安全隔离的扩展：

```json
{
  "wasm": {
    "plugins": [
      {
        "name": "my-plugin",
        "path": "./plugins/my-plugin.wasm",
        "enabled": true
      }
    ]
  }
}
```

WASM 特性：
- WASI 标准接口
- 内存安全隔离
- 热加载 / 卸载
- 与 Star API 互通

---

## 完整配置模板 (config.json)

```json
{
  "bot": {
    "name": "AstrBot",
    "debug": false
  },

  "provider": {
    "default": "anthropic",
    "providers": [
      {
        "name": "anthropic-main",
        "type": "anthropic",
        "api_key": "${ANTHROPIC_API_KEY}",
        "model": "claude-3-5-sonnet-20241022",
        "max_tokens": 4096,
        "temperature": 0.7
      },
      {
        "name": "qianfan-backup",
        "type": "qianfan",
        "api_key": "${QIANFAN_API_KEY}",
        "secret_key": "${QIANFAN_SECRET_KEY}",
        "model": "ernie-4.0-turbo-8k"
      }
    ]
  },

  "platform": {
    "enabled": ["qq", "telegram"],
    "qq": {
      "type": "onebot",
      "ws_url": "ws://127.0.0.1:3001",
      "access_token": ""
    },
    "telegram": {
      "token": "${TG_BOT_TOKEN}",
      "webhook": null
    }
  },

  "agent": {
    "enabled": true,
    "type": "react",
    "max_iterations": 10
  },

  "mcp": {
    "enabled": true,
    "servers": [
      {
        "name": "fs",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "./data"]
      }
    ]
  },

  "rag": {
    "enabled": false,
    "vector_store": {
      "type": "memory",
      "dimension": 768
    }
  },

  "wasm": {
    "enabled": false,
    "plugins": []
  },

  "dashboard": {
    "enabled": true,
    "host": "0.0.0.0",
    "port": 6185
  },

  "log": {
    "level": "info",
    "output": "stdout"
  }
}
```

环境变量支持：`${VAR_NAME}` 语法在配置中自动替换为对应环境变量值。

---

## 多 Provider 切换

支持对话中动态切换 Provider：

```
/provider anthropic-main
/provider qianfan-backup
```

或配置 fallback 链：当主 Provider 失败时自动降级。
