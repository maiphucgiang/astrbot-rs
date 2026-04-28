use serde::{Deserialize, Serialize};

/// Metadata about a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMetaData {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub enabled: bool,
    pub models: Vec<String>,
}
