use crate::errors::Result;
use crate::message::{AstrBotMessage, MessageChain};
use crate::platform::MessageSource;
use async_trait::async_trait;

/// Base trait for all events in AstrBot
#[async_trait]
pub trait Event: Send + Sync {
    /// Get the event type name
    fn event_type(&self) -> &'static str;
    /// Get the event source
    fn source(&self) -> &MessageSource;
    /// Clone as a boxed trait object
    fn clone_box(&self) -> Box<dyn Event>;
}

/// Result of processing any event type
#[derive(Debug, Clone, PartialEq)]
pub enum EventResult {
    /// Reply with a message chain
    MessageReply { chain: MessageChain },
    /// No action needed
    Nothing,
    /// Forward to another session
    Forward {
        target: MessageSource,
        chain: MessageChain,
    },
    /// Acknowledge a notice event (e.g., member joined, friend added)
    NoticeAck { action: String },
    /// Acknowledge a meta event (e.g., heartbeat, status update)
    MetaAck { action: String },
    /// Accept/reject a request (e.g., friend request, group join request)
    RequestResponse {
        approved: bool,
        reason: Option<String>,
    },
}

impl EventResult {
    pub fn reply(chain: MessageChain) -> Self {
        Self::MessageReply { chain }
    }

    pub fn reply_text(text: impl Into<String>) -> Self {
        Self::MessageReply {
            chain: MessageChain::new().text(text),
        }
    }

    pub fn nothing() -> Self {
        Self::Nothing
    }

    pub fn notice_ack(action: impl Into<String>) -> Self {
        Self::NoticeAck {
            action: action.into(),
        }
    }

    pub fn meta_ack(action: impl Into<String>) -> Self {
        Self::MetaAck {
            action: action.into(),
        }
    }

    pub fn approve() -> Self {
        Self::RequestResponse {
            approved: true,
            reason: None,
        }
    }

    pub fn reject(reason: impl Into<String>) -> Self {
        Self::RequestResponse {
            approved: false,
            reason: Some(reason.into()),
        }
    }

    /// Convert to MessageEventResult (for on_message default impl)
    pub fn into_message_result(self) -> crate::message::MessageEventResult {
        match self {
            EventResult::MessageReply { chain } => crate::message::MessageEventResult::Reply { chain },
            EventResult::Nothing => crate::message::MessageEventResult::Nothing,
            EventResult::Forward { target, chain } => crate::message::MessageEventResult::Forward { target, chain },
            _ => crate::message::MessageEventResult::Nothing,
        }
    }
}

/// Message event - triggered when a message is received
#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub source: MessageSource,
    pub message: AstrBotMessage,
}

#[async_trait]
impl Event for MessageEvent {
    fn event_type(&self) -> &'static str {
        "message"
    }

    fn source(&self) -> &MessageSource {
        &self.source
    }

    fn clone_box(&self) -> Box<dyn Event> {
        Box::new(self.clone())
    }
}

/// Command event - triggered when a command is detected
#[derive(Debug, Clone)]
pub struct CommandEvent {
    pub source: MessageSource,
    pub message: AstrBotMessage,
    pub command: String,
    pub args: Vec<String>,
}

#[async_trait]
impl Event for CommandEvent {
    fn event_type(&self) -> &'static str {
        "command"
    }

    fn source(&self) -> &MessageSource {
        &self.source
    }

    fn clone_box(&self) -> Box<dyn Event> {
        Box::new(self.clone())
    }
}

/// Notice event - group member changes, friend additions, etc.
#[derive(Debug, Clone)]
pub struct NoticeEvent {
    pub source: MessageSource,
    pub notice_type: String,
    pub data: serde_json::Value,
}

#[async_trait]
impl Event for NoticeEvent {
    fn event_type(&self) -> &'static str {
        "notice"
    }

    fn source(&self) -> &MessageSource {
        &self.source
    }

    fn clone_box(&self) -> Box<dyn Event> {
        Box::new(self.clone())
    }
}

/// Meta event - heartbeats, connection status, etc.
#[derive(Debug, Clone)]
pub struct MetaEvent {
    pub source: MessageSource,
    pub meta_type: String,
    pub data: serde_json::Value,
}

#[async_trait]
impl Event for MetaEvent {
    fn event_type(&self) -> &'static str {
        "meta"
    }

    fn source(&self) -> &MessageSource {
        &self.source
    }

    fn clone_box(&self) -> Box<dyn Event> {
        Box::new(self.clone())
    }
}

/// Request event - friend requests, group join requests, etc.
#[derive(Debug, Clone)]
pub struct RequestEvent {
    pub source: MessageSource,
    pub request_type: String,
    pub user_id: String,
    pub comment: Option<String>,
    pub data: serde_json::Value,
}

#[async_trait]
impl Event for RequestEvent {
    fn event_type(&self) -> &'static str {
        "request"
    }

    fn source(&self) -> &MessageSource {
        &self.source
    }

    fn clone_box(&self) -> Box<dyn Event> {
        Box::new(self.clone())
    }
}

/// Event filter trait
#[async_trait]
pub trait EventFilter: Send + Sync {
    /// Check if an event passes the filter
    async fn filter(&self, event: &dyn Event) -> bool;
    /// Get filter name
    fn name(&self) -> &'static str;
}

/// Event handler trait
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Handle an event
    async fn handle(&self, event: &dyn Event) -> Result<EventResult>;
    /// Get handler name
    fn name(&self) -> &'static str;
    /// Check if this handler can handle the given event
    fn can_handle(&self, event: &dyn Event) -> bool;
}

/// Event bus for routing events to handlers
pub struct EventBus {
    handlers: Vec<Box<dyn EventHandler>>,
    filters: Vec<Box<dyn EventFilter>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// Create a new event bus
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
            filters: Vec::new(),
        }
    }

    /// Register a handler
    pub fn register_handler(&mut self, handler: Box<dyn EventHandler>) {
        self.handlers.push(handler);
    }

    /// Register a filter
    pub fn register_filter(&mut self, filter: Box<dyn EventFilter>) {
        self.filters.push(filter);
    }

    /// Dispatch an event to all matching handlers
    pub async fn dispatch(&self, event: &dyn Event) -> Vec<Result<EventResult>> {
        let mut results = Vec::new();

        // Check all filters first
        for filter in &self.filters {
            if !filter.filter(event).await {
                return vec![];
            }
        }

        // Dispatch to handlers
        for handler in &self.handlers {
            if handler.can_handle(event) {
                let result = handler.handle(event).await;
                results.push(result);
            }
        }

        results
    }
}
