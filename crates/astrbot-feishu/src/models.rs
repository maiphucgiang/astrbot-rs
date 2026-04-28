//! Shared data models for Feishu integration

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Feishu app credentials
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppCredentials {
    pub app_id: String,
    pub app_secret: String,
    pub encrypt_key: Option<String>,
    pub verification_token: Option<String>,
}

/// Generic Feishu API response wrapper
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub msg: String,
    #[serde(default)]
    pub data: Option<T>,
}

/// Paginated response wrapper
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PaginatedResponse<T> {
    pub code: i32,
    pub msg: String,
    #[serde(default)]
    pub data: Option<PaginatedData<T>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PaginatedData<T> {
    pub items: Vec<T>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub page_token: Option<String>,
    #[serde(default)]
    pub total: Option<i64>,
}

/// A Feishu user (open_id style)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FeishuUser {
    pub open_id: String,
    #[serde(default)]
    pub union_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

/// A chat/group
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FeishuChat {
    pub chat_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub member_count: Option<i32>,
}

/// Message received from Feishu
#[derive(Clone, Debug, Default, Deserialize)]
pub struct IncomingMessage {
    pub message_id: String,
    pub chat_id: String,
    pub sender: FeishuUser,
    pub msg_type: String,
    pub content: serde_json::Value,
    #[serde(default)]
    pub create_time: Option<DateTime<Utc>>,
}

/// Outgoing message to Feishu
#[derive(Clone, Debug, Default, Serialize)]
pub struct OutgoingMessage {
    pub msg_type: String,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

/// Webhook event envelope
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WebhookEvent {
    pub schema: String,
    pub header: EventHeader,
    pub event: serde_json::Value,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct EventHeader {
    pub event_id: String,
    pub event_type: String,
    pub create_time: String,
    pub token: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub tenant_key: Option<String>,
}

/// Document metadata
#[derive(Clone, Debug, Default, Deserialize)]
pub struct DocumentInfo {
    pub document_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub create_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub update_time: Option<DateTime<Utc>>,
}

/// Bitable (multidimensional table) metadata
#[derive(Clone, Debug, Default, Deserialize)]
pub struct BitableInfo {
    pub app_token: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Bitable record
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BitableRecord {
    pub record_id: String,
    #[serde(default)]
    pub created_time: Option<i64>,
    #[serde(default)]
    pub updated_time: Option<i64>,
    #[serde(default)]
    pub fields: serde_json::Value,
}

/// Calendar event
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CalendarEventData {
    pub event_id: String,
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub attendees: Vec<String>,
}

/// Group message for search results
#[derive(Clone, Debug, Default, Deserialize)]
pub struct GroupMessage {
    pub message_id: String,
    pub chat_id: String,
    pub chat_name: Option<String>,
    pub sender: FeishuUser,
    pub msg_type: String,
    pub content_text: Option<String>,
    pub create_time: DateTime<Utc>,
}
