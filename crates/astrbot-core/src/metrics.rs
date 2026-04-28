use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub total_messages: u64,
    pub active_adapters: u32,
    pub active_providers: u32,
    pub healthy: bool,
}
