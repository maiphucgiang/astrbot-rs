# AstrBot-Rust

AstrBot 的 Rust 重写版本 — 高性能、类型安全、原生异步。

## 架构

```
astrbot-rs/
├── crates/
│   ├── astrbot-core/       # 核心框架（DB/Config/Provider/Pipeline/Plugin/Persona/...）
│   ├── astrbot-platform/   # 平台适配器框架 + 11 个平台实现
│   ├── astrbot-provider/  # LLM Provider 框架 + 12 个 provider
│   ├── astrbot-plugin/    # 插件系统（Star API + 热加载 + 市场）
│   ├── astrbot-dashboard/ # Dashboard 后端 API（28+ 路由）
│   ├── astrbot-cli/       # CLI + E2E 测试
│   ├── astrbot-security/  # 安全硬化（Webhook防Replay/Plugin权限/CodeExecutor沙箱）
│   ├── astrbot-persona/   # 人格系统 + Prompt Injection 5层防护
│   ├── astrbot-ux/        # 用户体验（OnboardingFlow/HelpSystem/ErrorTranslator）
│   └── astrbot-feishu/    # 飞书集成增强（文档/多维表/日程/群消息检索）
└── tests/                 # 集成测试
```

## 贡献者

| 模块 | 作者 |
|------|------|
| 核心框架 P0-P10 | 红毛 |
| 安全硬化 | 哈基米 |
| 人格系统 | 哈基米 |
| 飞书集成 | Soulter的Claw |
| 用户体验 | Soulclawter |
| 平台Mock测试 | piexianclaw |

## License

AGPL-3.0
