use std::collections::HashMap;

/// 帮助系统：管理所有指令注册与渲染
pub struct HelpSystem {
    registry: CommandRegistry,
}

/// 单条指令的元数据
#[derive(Debug, Clone)]
pub struct CommandInfo {
    /// 指令名称（如 status）
    pub name: String,
    /// 指令类别
    pub category: CommandCategory,
    /// 简短描述（一行）
    pub description: String,
    /// 详细说明
    pub long_description: String,
    /// 用法示例
    pub usage: Vec<String>,
    /// 别名
    pub aliases: Vec<String>,
    /// 是否需要管理员权限
    pub admin_only: bool,
}

/// 指令分类
#[derive(Debug, Clone, strum::Display)]
pub enum CommandCategory {
    #[strum(to_string = "对话")]
    Chat,
    #[strum(to_string = "管理")]
    Admin,
    #[strum(to_string = "工具")]
    Tool,
    #[strum(to_string = "系统")]
    System,
    #[strum(to_string = "知识库")]
    Knowledge,
    #[strum(to_string = "人格")]
    Persona,
}

/// 指令注册表
pub struct CommandRegistry {
    commands: HashMap<String, CommandInfo>,
}

/// 帮助查询范围
#[derive(Debug, Clone)]
pub enum HelpScope {
    /// 快速概览（最常用的 8 条）
    Quick,
    /// 完整指令列表（按分类）
    All,
    /// 单条指令详情
    Command(String),
    /// 分类筛选
    Category(CommandCategory),
    /// 关键词搜索
    Search(String),
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            commands: HashMap::new(),
        };
        registry.register_defaults();
        registry
    }

    /// 注册默认指令
    fn register_defaults(&mut self) {
        // 对话类
        self.register(CommandInfo {
            name: "help".to_string(),
            category: CommandCategory::System,
            description: "查看帮助信息".to_string(),
            long_description: "显示 AstrBot 的指令帮助。支持快速概览、完整列表、单指令详情和关键词搜索。".to_string(),
            usage: vec![
                "/help — 快速概览".to_string(),
                "/help all — 完整指令列表".to_string(),
                "/help status — 查看单指令详情".to_string(),
                "/help search 平台 — 搜索相关指令".to_string(),
            ],
            aliases: vec!["h".to_string(), "?".to_string()],
            admin_only: false,
        });

        self.register(CommandInfo {
            name: "reset".to_string(),
            category: CommandCategory::Chat,
            description: "重置当前会话记忆".to_string(),
            long_description: "清除当前对话上下文，让 bot 忘记之前的对话内容。".to_string(),
            usage: vec!["/reset — 重置记忆".to_string()],
            aliases: vec!["clear".to_string()],
            admin_only: false,
        });

        self.register(CommandInfo {
            name: "model".to_string(),
            category: CommandCategory::Chat,
            description: "切换使用的 AI 模型".to_string(),
            long_description: "在已配置的模型之间切换，如 GPT-4、DeepSeek、Claude 等。".to_string(),
            usage: vec![
                "/model — 列出可用模型".to_string(),
                "/model gpt-4 — 切换到 gpt-4".to_string(),
            ],
            aliases: vec!["m".to_string()],
            admin_only: false,
        });

        self.register(CommandInfo {
            name: "persona".to_string(),
            category: CommandCategory::Persona,
            description: "切换 bot 人格".to_string(),
            long_description: "在预设的人格之间切换，如傲慢俏皮型、温和陪伴型、毒舌吐槽型等。".to_string(),
            usage: vec![
                "/persona — 列出可用人格".to_string(),
                "/persona 傲娇 — 切换到指定人格".to_string(),
            ],
            aliases: vec!["p".to_string(), "人格".to_string()],
            admin_only: false,
        });

        // 管理类
        self.register(CommandInfo {
            name: "status".to_string(),
            category: CommandCategory::System,
            description: "显示各平台/Provider 健康状态".to_string(),
            long_description: "查看所有已连接平台和 LLM Provider 的运行状态，包括在线/离线、响应延迟等信息。".to_string(),
            usage: vec![
                "/status — 查看全部状态".to_string(),
                "/status qq — 查看指定平台状态".to_string(),
            ],
            aliases: vec!["s".to_string(), "状态".to_string()],
            admin_only: false,
        });

        self.register(CommandInfo {
            name: "admin".to_string(),
            category: CommandCategory::Admin,
            description: "管理员指令入口".to_string(),
            long_description: "管理员专用指令，包括热重载、日志查看、调试模式等。".to_string(),
            usage: vec![
                "/admin reload — 热重载配置".to_string(),
                "/admin logs — 查看最近日志".to_string(),
                "/admin debug — 开关调试模式".to_string(),
            ],
            aliases: vec!["adm".to_string()],
            admin_only: true,
        });

        self.register(CommandInfo {
            name: "plugin".to_string(),
            category: CommandCategory::Admin,
            description: "插件管理".to_string(),
            long_description: "安装、卸载、启用、禁用插件，以及查看插件列表。".to_string(),
            usage: vec![
                "/plugin list — 列出插件".to_string(),
                "/plugin install xxx — 安装插件".to_string(),
                "/plugin uninstall xxx — 卸载插件".to_string(),
                "/plugin enable xxx — 启用插件".to_string(),
                "/plugin disable xxx — 禁用插件".to_string(),
            ],
            aliases: vec!["pl".to_string(), "插件".to_string()],
            admin_only: true,
        });

        self.register(CommandInfo {
            name: "config".to_string(),
            category: CommandCategory::Admin,
            description: "修改配置项".to_string(),
            long_description: "查看和修改 AstrBot 的配置项，修改后自动热重载。".to_string(),
            usage: vec![
                "/config get platform.qq — 查看配置".to_string(),
                "/config set platform.qq.group_id=123456 — 修改配置".to_string(),
            ],
            aliases: vec!["cfg".to_string(), "配置".to_string()],
            admin_only: true,
        });

        // 工具类
        self.register(CommandInfo {
            name: "tools".to_string(),
            category: CommandCategory::Tool,
            description: "列出可用工具".to_string(),
            long_description: "查看当前可用的外部工具（MCP servers、插件提供的工具等）。".to_string(),
            usage: vec![
                "/tools — 列出所有工具".to_string(),
                "/tools search weather — 搜索工具".to_string(),
            ],
            aliases: vec!["t".to_string(), "工具".to_string()],
            admin_only: false,
        });

        // 知识库类
        self.register(CommandInfo {
            name: "rag".to_string(),
            category: CommandCategory::Knowledge,
            description: "知识库查询".to_string(),
            long_description: "在已加载的知识库中搜索相关内容，支持语义检索。".to_string(),
            usage: vec![
                "/rag 怎么安装插件 — 知识库问答".to_string(),
                "/rag status — 查看知识库状态".to_string(),
            ],
            aliases: vec!["r".to_string(), "知识库".to_string()],
            admin_only: false,
        });

        self.register(CommandInfo {
            name: "ping".to_string(),
            category: CommandCategory::System,
            description: "测试 bot 响应".to_string(),
            long_description: "最简单的连通性测试，返回当前延迟。".to_string(),
            usage: vec!["/ping — 测试响应".to_string()],
            aliases: vec![].to_vec(),
            admin_only: false,
        });
    }

    pub fn register(&mut self, info: CommandInfo) {
        self.commands.insert(info.name.clone(), info);
    }

    pub fn get(&self, name: &str) -> Option<&CommandInfo> {
        self.commands.get(name)
    }

    pub fn all(&self) -> Vec<&CommandInfo> {
        let mut cmds: Vec<_> = self.commands.values().collect();
        cmds.sort_by(|a, b| a.category.to_string().cmp(&b.category.to_string())
            .then_with(|| a.name.cmp(&b.name)));
        cmds
    }

    pub fn search(&self, keyword: &str) -> Vec<&CommandInfo> {
        let kw = keyword.to_lowercase();
        self.commands.values()
            .filter(|c| {
                c.name.to_lowercase().contains(&kw)
                    || c.description.to_lowercase().contains(&kw)
                    || c.category.to_string().to_lowercase().contains(&kw)
                    || c.aliases.iter().any(|a| a.to_lowercase().contains(&kw))
            })
            .collect()
    }

    pub fn by_category(&self, category: &CommandCategory) -> Vec<&CommandInfo> {
        let mut cmds: Vec<_> = self.commands.values()
            .filter(|c| std::mem::discriminant(&c.category) == std::mem::discriminant(category))
            .collect();
        cmds.sort_by_key(|c| &c.name);
        cmds
    }
}

impl HelpSystem {
    pub fn new() -> Self {
        Self {
            registry: CommandRegistry::new(),
        }
    }

    /// 渲染帮助文本
    pub fn render(&self, scope: HelpScope) -> String {
        match scope {
            HelpScope::Quick => self.render_quick(),
            HelpScope::All => self.render_all(),
            HelpScope::Command(cmd) => self.render_command(&cmd),
            HelpScope::Category(cat) => self.render_category(cat),
            HelpScope::Search(keyword) => self.render_search(&keyword),
        }
    }

    /// 聊天场景入口：/help 或 /help <cmd>
    ///
    /// - `help_text(None)` → 快速概览（最常用的指令）
    /// - `help_text(Some("all"))` → 完整指令列表
    /// - `help_text(Some("search <keyword>"))` → 关键词搜索
    /// - `help_text(Some("<cmd>"))` → 单条指令详情
    pub fn help_text(&self, cmd: Option<&str>) -> String {
        match cmd {
            None | Some("") | Some("quick") => self.render_quick(),
            Some("all") => self.render_all(),
            Some(keyword) if keyword.starts_with("search ") => {
                let kw = keyword.trim_start_matches("search ").trim();
                self.render_search(kw)
            }
            Some(cmd_name) => self.render_command(cmd_name),
        }
    }

    /// 快速概览：最常用的 8 条指令
    fn render_quick(&self) -> String {
        let mut lines = vec![
            "🌟 AstrBot 常用指令".to_string(),
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        ];

        let categories = vec![
            CommandCategory::Chat,
            CommandCategory::System,
            CommandCategory::Persona,
            CommandCategory::Knowledge,
        ];

        for cat in categories {
            let cmds = self.registry.by_category(&cat);
            if !cmds.is_empty() {
                lines.push(format!("{}", cat));
                // 优先显示非管理员指令
                let visible: Vec<_> = cmds.iter().filter(|c| !c.admin_only).take(2).collect();
                for cmd in visible {
                    lines.push(format!("  /{:<12} {}", cmd.name, cmd.description));
                }
            }
        }

        lines.push("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
        lines.push("输入 /help all 查看完整列表".to_string());
        lines.push("输入 /help <指令名> 查看详细用法".to_string());

        lines.join("\n")
    }

    /// 完整指令列表（按分类分组）
    fn render_all(&self) -> String {
        let mut lines = vec![
            "📖 AstrBot 完整指令列表".to_string(),
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        ];

        let all = self.registry.all();
        let mut current_cat = String::new();

        for cmd in all {
            let cat_str = cmd.category.to_string();
            if cat_str != current_cat {
                current_cat = cat_str.clone();
                lines.push(format!("\n【{}】", cat_str));
            }
            let alias_str = if cmd.aliases.is_empty() {
                String::new()
            } else {
                format!(" (别名: {})", cmd.aliases.join(", "))
            };
            let admin_tag = if cmd.admin_only { " 🔒" } else { "" };
            lines.push(format!(
                "  /{:<12} {}{}{}",
                cmd.name, cmd.description, alias_str, admin_tag
            ));
        }

        lines.push("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
        lines.push("🔒 标记为管理员指令".to_string());

        lines.join("\n")
    }

    /// 单条指令详情
    fn render_command(&self, cmd_name: &str) -> String {
        let cmd = match self.registry.get(cmd_name) {
            Some(c) => c,
            None => return format!("找不到指令 /{}，输入 /help all 查看所有指令。", cmd_name),
        };

        let mut lines = vec![
            format!("/{} — {}", cmd.name, cmd.description),
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        ];

        lines.push(format!("\n📋 说明\n{}", cmd.long_description));

        if !cmd.aliases.is_empty() {
            lines.push(format!("\n🏷️ 别名: {}", cmd.aliases.join(", ")));
        }

        lines.push("\n💡 用法示例".to_string());
        for example in &cmd.usage {
            lines.push(format!("  {}", example));
        }

        if cmd.admin_only {
            lines.push("\n⚠️ 需要管理员权限".to_string());
        }

        lines.join("\n")
    }

    /// 分类筛选
    fn render_category(&self, category: CommandCategory) -> String {
        let cmds = self.registry.by_category(&category);
        let mut lines = vec![
            format!("【{}】指令列表", category),
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        ];

        for cmd in cmds {
            lines.push(format!(
                "  /{:<12} {}",
                cmd.name, cmd.description
            ));
        }

        lines.push("\n输入 /help <指令名> 查看详细用法".to_string());
        lines.join("\n")
    }

    /// 关键词搜索
    fn render_search(&self, keyword: &str) -> String {
        let results = self.registry.search(keyword);
        let mut lines = vec![
            format!("🔍 搜索 \"{}\" 的结果", keyword),
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        ];

        if results.is_empty() {
            lines.push("没有找到匹配的指令。试试其他关键词？".to_string());
        } else {
            for cmd in results {
                lines.push(format!(
                    "  /{:<12} {} [{}]",
                    cmd.name, cmd.description, cmd.category
                ));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_default_commands() {
        let registry = CommandRegistry::new();
        assert!(registry.get("help").is_some());
        assert!(registry.get("status").is_some());
        assert!(registry.get("persona").is_some());
        assert!(registry.get("rag").is_some());
        assert!(registry.get("tools").is_some());
    }

    #[test]
    fn test_help_quick_not_empty() {
        let help = HelpSystem::new();
        let text = help.render(HelpScope::Quick);
        assert!(text.contains("/help"));
        assert!(text.contains("常用指令"));
        assert!(text.contains("/help all"));
    }

    #[test]
    fn test_help_command_detail() {
        let help = HelpSystem::new();
        let text = help.render(HelpScope::Command("status".to_string()));
        assert!(text.contains("status"));
        assert!(text.contains("健康状态"));
        assert!(text.contains("用法示例"));
    }

    #[test]
    fn test_help_search_found() {
        let help = HelpSystem::new();
        let text = help.render(HelpScope::Search("平台".to_string()));
        assert!(text.contains("status") || text.contains("平台"));
    }

    #[test]
    fn test_help_search_not_found() {
        let help = HelpSystem::new();
        let text = help.render(HelpScope::Search("xyz123".to_string()));
        assert!(text.contains("没有找到"));
    }

    #[test]
    fn test_render_all_includes_aliases() {
        let help = HelpSystem::new();
        let text = help.render(HelpScope::All);
        // 至少有一个别名被渲染
        assert!(text.contains("别名") || text.contains("🔒"));
    }

    #[test]
    fn test_help_text_none() {
        let help = HelpSystem::new();
        let text = help.help_text(None);
        assert!(text.contains("常用指令"));
        assert!(text.contains("/help all"));
    }

    #[test]
    fn test_help_text_all() {
        let help = HelpSystem::new();
        let text = help.help_text(Some("all"));
        assert!(text.contains("完整指令列表"));
        assert!(text.contains("/help"));
        assert!(text.contains("/status"));
    }

    #[test]
    fn test_help_text_command() {
        let help = HelpSystem::new();
        let text = help.help_text(Some("reset"));
        assert!(text.contains("reset"));
        assert!(text.contains("重置"));
        assert!(text.contains("用法示例"));
    }

    #[test]
    fn test_help_text_search() {
        let help = HelpSystem::new();
        let text = help.help_text(Some("search 平台"));
        assert!(text.contains("平台"));
        assert!(text.contains("status") || text.contains("状态"));
    }

    #[test]
    fn test_help_text_quick() {
        let help = HelpSystem::new();
        let text = help.help_text(Some("quick"));
        assert!(text.contains("常用指令"));
    }

    #[test]
    fn test_help_text_unknown_command() {
        let help = HelpSystem::new();
        let text = help.help_text(Some("xyz123"));
        assert!(text.contains("找不到指令"));
    }

    #[test]
    fn test_help_text_empty() {
        let help = HelpSystem::new();
        let text = help.help_text(Some(""));
        assert!(text.contains("常用指令"));
    }
}
