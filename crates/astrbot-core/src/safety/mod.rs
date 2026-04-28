use crate::errors::{AstrBotError, Result};
use crate::message::MessageChain;
use async_trait::async_trait;
use std::collections::HashSet;

#[cfg(test)]
mod tests;

/// Result of content safety check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyResult {
    /// Content is safe
    Safe,
    /// Content violates policy with reason
    Violation { reason: String, strategy: String },
    /// Check failed (e.g. AI moderation API error)
    Error { message: String },
}

/// Strategy for content safety detection
#[async_trait]
pub trait SafetyStrategy: Send + Sync {
    /// Strategy name
    fn name(&self) -> &str;
    /// Check message chain
    async fn check(&self, chain: &MessageChain) -> SafetyResult;
}

/// Keyword-based filter — checks for blocked words
pub struct KeywordFilter {
    name: String,
    blocked_words: HashSet<String>,
    case_sensitive: bool,
}

impl KeywordFilter {
    pub fn new(name: impl Into<String>, blocked_words: Vec<String>, case_sensitive: bool) -> Self {
        let name = name.into();
        let words = if case_sensitive {
            blocked_words.into_iter().collect()
        } else {
            blocked_words.into_iter().map(|w| w.to_lowercase()).collect()
        };
        Self {
            name,
            blocked_words: words,
            case_sensitive,
        }
    }

    pub fn add_word(&mut self, word: impl Into<String>) {
        let word = if self.case_sensitive {
            word.into()
        } else {
            word.into().to_lowercase()
        };
        self.blocked_words.insert(word);
    }

    pub fn remove_word(&mut self, word: &str) {
        let key = if self.case_sensitive {
            word.to_string()
        } else {
            word.to_lowercase()
        };
        self.blocked_words.remove(&key);
    }
}

#[async_trait]
impl SafetyStrategy for KeywordFilter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn check(&self, chain: &MessageChain) -> SafetyResult {
        let text = chain.plain_text();
        let check_text = if self.case_sensitive {
            text
        } else {
            text.to_lowercase()
        };

        for word in &self.blocked_words {
            if check_text.contains(word) {
                return SafetyResult::Violation {
                    reason: format!("Blocked keyword detected: '{}'", word),
                    strategy: self.name.clone(),
                };
            }
        }

        SafetyResult::Safe
    }
}

/// Regex-based filter — checks for patterns
pub struct RegexFilter {
    name: String,
    patterns: Vec<regex::Regex>,
    descriptions: Vec<String>,
}

impl RegexFilter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            patterns: vec![],
            descriptions: vec![],
        }
    }

    pub fn add_pattern(
        &mut self, pattern: &str, description: impl Into<String>) -> Result<()> {
        let regex = regex::Regex::new(pattern).map_err(|e| {
            AstrBotError::Internal(format!("Invalid regex pattern '{}': {}", pattern, e))
        })?;
        self.patterns.push(regex);
        self.descriptions.push(description.into());
        Ok(())
    }
}

#[async_trait]
impl SafetyStrategy for RegexFilter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn check(&self, chain: &MessageChain) -> SafetyResult {
        let text = chain.plain_text();

        for (i, pattern) in self.patterns.iter().enumerate() {
            if pattern.is_match(&text) {
                let desc = self.descriptions.get(i)
                    .map(|d| d.as_str())
                    .unwrap_or("pattern matched");
                return SafetyResult::Violation {
                    reason: format!("Regex matched: {}", desc),
                    strategy: self.name.clone(),
                };
            }
        }

        SafetyResult::Safe
    }
}

/// AI moderation filter — calls LLM to judge content
///
/// Skeleton implementation: uses a configurable prompt template
pub struct AiModeration {
    name: String,
    provider: std::sync::Arc<dyn crate::provider::Provider>,
    model: String,
    threshold: f32,  // 0.0 ~ 1.0, content above this is flagged
}

impl AiModeration {
    pub fn new(
        name: impl Into<String>,
        provider: std::sync::Arc<dyn crate::provider::Provider>,
        model: impl Into<String>,
        threshold: f32,
    ) -> Self {
        Self {
            name: name.into(),
            provider,
            model: model.into(),
            threshold: threshold.clamp(0.0, 1.0),
        }
    }
}

#[async_trait]
impl SafetyStrategy for AiModeration {
    fn name(&self) -> &str {
        &self.name
    }

    async fn check(&self, chain: &MessageChain) -> SafetyResult {
        let text = chain.plain_text();
        if text.is_empty() {
            return SafetyResult::Safe;
        }

        let prompt = format!(
            r#"You are a content moderation assistant. Analyze the following text and respond with ONLY a number between 0.0 and 1.0 representing the safety score (0.0 = completely safe, 1.0 = extremely harmful). Do not include any other text.

Text: {}

Safety score:"#,
            text
        );

        let messages = vec![
            crate::provider::ChatMessage {
                role: "user".to_string(),
                content: prompt,
                name: None,
            },
        ];

        let config = crate::provider::ChatConfig {
            model: Some(self.model.clone()),
            temperature: Some(0.0),
            max_tokens: Some(10),
            top_p: None,
            stream: false,
            extra: std::collections::HashMap::new(),
        };

        match self.provider.chat(messages, config).await {
            Ok(response) => {
                let score_str = response.content.trim();
                if let Ok(score) = score_str.parse::<f32>() {
                    if score >= self.threshold {
                        SafetyResult::Violation {
                            reason: format!("AI moderation flagged content with score {:.2}", score),
                            strategy: self.name.clone(),
                        }
                    } else {
                        SafetyResult::Safe
                    }
                } else {
                    SafetyResult::Error {
                        message: format!("Failed to parse AI moderation score: '{}'", score_str),
                    }
                }
            }
            Err(e) => {
                SafetyResult::Error {
                    message: format!("AI moderation API error: {}", e),
                }
            }
        }
    }
}

/// Safety engine — runs multiple strategies and returns the first violation
pub struct SafetyEngine {
    strategies: Vec<Box<dyn SafetyStrategy>>,
    /// Whether to stop on first violation (true) or collect all (false)
    stop_on_first: bool,
}

impl Default for SafetyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SafetyEngine {
    pub fn new() -> Self {
        Self {
            strategies: vec![],
            stop_on_first: true,
        }
    }

    /// Configure stop-on-first-violation behavior
    pub fn with_stop_on_first(mut self, stop: bool) -> Self {
        self.stop_on_first = stop;
        self
    }

    /// Add a safety strategy
    pub fn add_strategy(mut self, strategy: Box<dyn SafetyStrategy>) -> Self {
        self.strategies.push(strategy);
        self
    }

    /// Check content against all strategies
    pub async fn check(&self,
        chain: &MessageChain,
    ) -> Vec<SafetyResult> {
        let mut results = Vec::new();

        for strategy in &self.strategies {
            let result = strategy.check(chain).await;
            let is_violation = matches!(result, SafetyResult::Violation { .. });
            results.push(result);

            if is_violation && self.stop_on_first {
                break;
            }
        }

        results
    }

    /// Quick check: returns true if safe, false if any violation
    pub async fn is_safe(&self,
        chain: &MessageChain,
    ) -> bool {
        let results = self.check(chain).await;
        results.iter().all(|r| matches!(r, SafetyResult::Safe))
    }

    /// Get first violation reason if any
    pub async fn first_violation(&self,
        chain: &MessageChain,
    ) -> Option<String> {
        let results = self.check(chain).await;
        for result in results {
            if let SafetyResult::Violation { reason, .. } = result {
                return Some(reason);
            }
        }
        None
    }
}

/// Preset safety engine with common filters
pub fn preset_engine() -> SafetyEngine {
    let keyword_filter = KeywordFilter::new(
        "keyword",
        vec![
            "badword".to_string(),
            "spam".to_string(),
        ],
        false,
    );

    let mut regex_filter = RegexFilter::new("regex");
    // Example: detect URLs (skeleton)
    let _ = regex_filter.add_pattern(
        r"https?://[^\s]+",
        "external URL detected",
    );

    SafetyEngine::new()
        .add_strategy(Box::new(keyword_filter))
        .add_strategy(Box::new(regex_filter))
}
