//! Feishu group message search/retrieval for context and RAG

use chrono::{DateTime, Duration, Utc};
use reqwest::Method;
use serde_json::json;
use tracing::{debug, info};

use crate::{auth::FeishuAuth, FeishuError, GroupMessage, PaginatedResponse, Result};

/// Search query for group messages
#[derive(Clone, Debug, Default)]
pub struct SearchQuery {
    pub chat_id: Option<String>,
    pub sender_id: Option<String>,
    pub keyword: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub msg_type: Option<String>, // "text", "image", "file", etc.
    pub page_size: i32,
}

impl SearchQuery {
    pub fn with_chat(mut self, chat_id: impl Into<String>) -> Self {
        self.chat_id = Some(chat_id.into());
        self
    }

    pub fn with_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keyword = Some(keyword.into());
        self
    }

    pub fn with_time_range(mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        self.start_time = Some(start);
        self.end_time = Some(end);
        self
    }

    pub fn last_24h() -> Self {
        let now = Utc::now();
        Self {
            start_time: Some(now - Duration::hours(24)),
            end_time: Some(now),
            page_size: 50,
            ..Default::default()
        }
    }
}

/// Group message search client
pub struct GroupMessageSearch {
    auth: FeishuAuth,
}

impl GroupMessageSearch {
    pub fn new(auth: FeishuAuth) -> Self {
        Self { auth }
    }

    /// Search messages in a specific chat
    pub async fn search_in_chat(
        &self,
        chat_id: &str,
        query: &SearchQuery,
    ) -> Result<Vec<GroupMessage>> {
        let mut path = format!(
            "/im/v1/messages?container_chat_id={}&page_size={}",
            chat_id,
            query.page_size.max(1).min(50)
        );

        if let Some(start) = query.start_time {
            path.push_str(&format!("&start_time={}", start.timestamp_millis()));
        }
        if let Some(end) = query.end_time {
            path.push_str(&format!("&end_time={}", end.timestamp_millis()));
        }

        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let data = api_resp.data.unwrap();
        let items = data
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut messages: Vec<GroupMessage> = Vec::new();
        for item in items {
            let msg = self.parse_message(&item).await?;
            // Apply keyword filter locally
            if let Some(ref kw) = query.keyword {
                let text = msg.content_text.as_deref().unwrap_or("");
                if !text.to_lowercase().contains(&kw.to_lowercase()) {
                    continue;
                }
            }
            // Apply sender filter
            if let Some(ref sender) = query.sender_id {
                if &msg.sender.open_id != sender {
                    continue;
                }
            }
            messages.push(msg);
        }

        info!(
            "Search in chat {} returned {} messages",
            chat_id,
            messages.len()
        );
        Ok(messages)
    }

    /// Search messages across chats (via user message search API)
    pub async fn search_cross_chat(&self, query: &SearchQuery) -> Result<Vec<GroupMessage>> {
        let body = json!({
            "query": query.keyword.as_deref().unwrap_or(""),
            "page_size": query.page_size.max(1).min(50),
            "message_type": query.msg_type.as_deref(),
        });

        let req = self
            .auth
            .auth_request(Method::POST, "/im/v1/messages/search")
            .await?;
        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;

        let api_resp: PaginatedResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let data = api_resp.data.unwrap();
        let mut messages = Vec::new();
        for item in data.items {
            if let Ok(msg) = self.parse_message(&item).await {
                messages.push(msg);
            }
        }

        Ok(messages)
    }

    /// Get message by ID
    pub async fn get_message(&self, message_id: &str) -> Result<GroupMessage> {
        let path = format!("/im/v1/messages/{}", message_id);
        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        self.parse_message(&api_resp.data.unwrap()).await
    }

    /// Extract text content from message body
    pub fn extract_text(content: &serde_json::Value) -> Option<String> {
        // Handle text messages
        if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
            return Some(text.to_string());
        }

        // Handle post/rich text (simplified)
        if let Some(post) = content.get("post") {
            // Try zh_cn content
            if let Some(cn) = post.get("zh_cn") {
                let mut texts = Vec::new();
                if let Some(content_arr) = cn.get("content").and_then(|v| v.as_array()) {
                    for block in content_arr {
                        if let Some(block_arr) = block.as_array() {
                            for elem in block_arr {
                                if let Some(tag) = elem.get("tag").and_then(|v| v.as_str()) {
                                    if tag == "text" {
                                        if let Some(t) = elem.get("text").and_then(|v| v.as_str()) {
                                            texts.push(t.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !texts.is_empty() {
                    return Some(texts.join(""));
                }
            }
        }

        // Fallback: stringify the whole content
        content
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| Some(content.to_string().trim_matches('"').to_string()))
    }

    /// Format messages as context for LLM RAG
    pub fn format_as_context(messages: &[GroupMessage], max_chars: usize) -> String {
        let mut context = String::new();
        context.push_str("# 群聊历史记录\n\n");

        for msg in messages.iter().rev() {
            let sender = msg.sender.name.as_deref().unwrap_or(&msg.sender.open_id);
            let time = msg.create_time.format("%m-%d %H:%M");
            let text = msg.content_text.as_deref().unwrap_or("[非文本消息]");

            let line = format!("[{}] {}: {}\n", time, sender, text);

            if context.len() + line.len() > max_chars {
                context.push_str("\n...(截断)\n");
                break;
            }
            context.push_str(&line);
        }

        context
    }

    // --- internal ---

    async fn parse_message(&self, raw: &serde_json::Value) -> Result<GroupMessage> {
        let message_id = raw
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let chat_id = raw
            .get("chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let msg_type = raw
            .get("msg_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let sender = raw
            .get("sender")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(crate::FeishuUser {
                open_id: "unknown".into(),
                union_id: None,
                user_id: None,
                name: None,
                avatar: None,
            });

        let content_text = raw
            .get("body")
            .and_then(|b| b.get("content"))
            .and_then(|c| Self::extract_text(c));

        let create_time = raw
            .get("create_time")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Ok(GroupMessage {
            message_id,
            chat_id,
            chat_name: None,
            sender,
            msg_type,
            content_text,
            create_time,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_simple() {
        let content = json!({ "text": "hello world" });
        assert_eq!(
            GroupMessageSearch::extract_text(&content),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_format_context_empty() {
        let ctx = GroupMessageSearch::format_as_context(&[], 4000);
        assert!(ctx.contains("群聊历史记录"));
    }

    #[test]
    fn test_search_query_builder() {
        let q = SearchQuery::last_24h()
            .with_chat("oc_xxx")
            .with_keyword("test");
        assert_eq!(q.chat_id, Some("oc_xxx".to_string()));
        assert_eq!(q.keyword, Some("test".to_string()));
        assert!(q.start_time.is_some());
    }
}
