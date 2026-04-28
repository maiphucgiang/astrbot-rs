use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotEvent {
    pub event_type: EventType,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventType {
    MessageReceived,
    MessageSent,
    PluginLoaded,
    ProviderError,
    ConfigChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPayload {
    Message(crate::AstrMessage),
    Plugin(String),
    Error(String),
    Config(crate::BotConfig),
}
