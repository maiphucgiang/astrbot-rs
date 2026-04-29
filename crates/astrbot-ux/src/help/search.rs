// Phase 2: HelpSystem 搜索优化 by Soulclawter
// 按关键词搜索命令列表，支持模糊匹配

use crate::help::{CommandInfo, CommandRegistry};

/// 搜索结果
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// 命令名
    pub name: String,
    /// 匹配得分（越高越靠前）
    pub score: u32,
    /// 匹配原因
    pub matched_by: MatchReason,
}

/// 匹配原因
#[derive(Debug, Clone, PartialEq)]
pub enum MatchReason {
    /// 命令名完全匹配
    ExactName,
    /// 命令名前缀匹配
    PrefixName,
    /// 别名匹配
    Alias,
    /// 描述匹配
    Description,
    /// 分类匹配
    Category,
}

/// 搜索命令
///
/// - `query` 为空 → 返回所有命令（按注册顺序）
/// - `query` 非空 → 按关键词模糊匹配，返回得分最高的前 N 条
pub fn search_commands(registry: &CommandRegistry, query: &str, limit: usize) -> Vec<SearchResult> {
    if query.trim().is_empty() {
        let mut all: Vec<_> = registry
            .all()
            .into_iter()
            .map(|cmd| SearchResult {
                name: cmd.name.clone(),
                score: 0,
                matched_by: MatchReason::ExactName,
            })
            .collect();
        all.truncate(limit);
        return all;
    }

    let q = query.to_lowercase();
    let mut results: Vec<SearchResult> = Vec::new();

    for cmd in registry.all() {
        let name_lower = cmd.name.to_lowercase();
        let desc_lower = cmd.description.to_lowercase();
        let cat_lower = cmd.category.to_string().to_lowercase();

        // 完全匹配命令名 → 最高优先级
        if name_lower == q {
            results.push(SearchResult {
                name: cmd.name.clone(),
                score: 100,
                matched_by: MatchReason::ExactName,
            });
            continue;
        }

        // 别名匹配 → 高优先级（优先于前缀匹配）
        if cmd
            .aliases
            .iter()
            .any(|a| a.to_lowercase() == q || a.to_lowercase().starts_with(&q))
        {
            results.push(SearchResult {
                name: cmd.name.clone(),
                score: 80,
                matched_by: MatchReason::Alias,
            });
            continue;
        }

        // 前缀匹配命令名 → 中高优先级
        if name_lower.starts_with(&q) {
            results.push(SearchResult {
                name: cmd.name.clone(),
                score: 60 + name_lower.len() as u32,
                matched_by: MatchReason::PrefixName,
            });
            continue;
        }

        // 描述包含关键词 → 中优先级
        if desc_lower.contains(&q) {
            results.push(SearchResult {
                name: cmd.name.clone(),
                score: 40,
                matched_by: MatchReason::Description,
            });
            continue;
        }

        // 分类包含关键词 → 低优先级
        if cat_lower.contains(&q) {
            results.push(SearchResult {
                name: cmd.name.clone(),
                score: 20,
                matched_by: MatchReason::Category,
            });
            continue;
        }
    }

    // 去重：同命令保留最高分
    results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    results.dedup_by(|a, b| a.name == b.name);
    results.truncate(limit);

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::help::CommandRegistry;

    #[test]
    fn test_search_exact_name() {
        let registry = CommandRegistry::new();
        let results = search_commands(&registry, "reset", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "reset");
        assert_eq!(results[0].matched_by, MatchReason::ExactName);
        assert_eq!(results[0].score, 100);
    }

    #[test]
    fn test_search_prefix_and_description() {
        let registry = CommandRegistry::new();
        let results = search_commands(&registry, "status", 10);
        // "status" 是精确命令名，应该排第一
        let exact = results.iter().find(|r| r.name == "status");
        assert!(exact.is_some());
        assert_eq!(exact.unwrap().matched_by, MatchReason::ExactName);
    }

    #[test]
    fn test_search_alias() {
        let registry = CommandRegistry::new();
        // "p" 是 persona 的别名
        let results = search_commands(&registry, "p", 10);
        let persona = results.iter().find(|r| r.name == "persona");
        assert!(persona.is_some(), "搜索 'p' 应该匹配 persona 的别名");
        assert_eq!(persona.unwrap().matched_by, MatchReason::Alias);
    }

    #[test]
    fn test_search_empty_query_returns_all() {
        let registry = CommandRegistry::new();
        let results = search_commands(&registry, "", 100);
        assert!(results.len() >= 10); // 默认注册了 10+ 条指令
    }

    #[test]
    fn test_search_limit() {
        let registry = CommandRegistry::new();
        let results = search_commands(&registry, "", 3);
        assert_eq!(results.len(), 3); // 限制返回 3 条
    }

    #[test]
    fn test_search_no_match() {
        let registry = CommandRegistry::new();
        let results = search_commands(&registry, "xyz123", 10);
        assert!(results.is_empty());
    }
}
