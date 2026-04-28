use crate::db::Database;
use crate::errors::{AstrBotError, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Persona definition
// ---------------------------------------------------------------------------

/// A persona (character / personality) for the bot.
///
/// Each persona defines a name, a system prompt template, and an optional
/// set of template variables (e.g. `{{name}}`, `{{mood}}`) that are
/// substituted at runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Persona {
    /// Unique identifier for this persona
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// System prompt template (may contain `{{var}}` placeholders)
    pub system_prompt: String,
    /// Default variable values — used when a variable is not supplied at render time
    #[serde(default)]
    pub variables: HashMap<String, String>,
    /// Whether this persona is marked as default
    #[serde(default)]
    pub is_default: bool,
    /// Optional description for UI display
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Persona {
    /// Create a new persona.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            system_prompt: system_prompt.into(),
            variables: HashMap::new(),
            is_default: false,
            description: None,
        }
    }

    /// Set a default variable value (builder style).
    pub fn with_variable(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.variables.insert(key.into(), value.into());
        self
    }

    /// Mark as default persona (builder style).
    pub fn with_default(mut self, is_default: bool) -> Self {
        self.is_default = is_default;
        self
    }

    /// Render the system prompt by substituting `{{variable}}` placeholders.
    ///
    /// Variables supplied in `overrides` take precedence over the persona's own
    /// defaults.  If a variable is missing from both, the placeholder is left
    /// untouched (so the LLM sees `{{var}}` rather than a blank string).
    pub fn render(&self, overrides: Option<&HashMap<String, String>>) -> String {
        let re = Regex::new(r"\{\{\s*([a-zA-Z0-9_]+)\s*\}\}").expect("valid regex");
        let mut result = self.system_prompt.clone();

        // Collect all known variables: persona defaults < overrides
        let mut ctx = self.variables.clone();
        if let Some(ov) = overrides {
            for (k, v) in ov {
                ctx.insert(k.clone(), v.clone());
            }
        }

        // Replace each occurrence
        for cap in re.captures_iter(&self.system_prompt.clone()) {
            let full = cap.get(0).map(|m| m.as_str()).unwrap_or("");
            let key = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if let Some(val) = ctx.get(key) {
                result = result.replace(full, val);
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// Persona registry
// ---------------------------------------------------------------------------

/// In-memory registry of personas with load / get / list / switch operations.
///
/// Persists state to SQLite when a `Database` handle is provided.
pub struct PersonaRegistry {
    personas: RwLock<HashMap<String, Arc<Persona>>>,
    active_id: RwLock<Option<String>>,
    db: Option<Arc<Database>>,
}

impl Default for PersonaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PersonaRegistry {
    /// Create an empty registry (memory-only).
    pub fn new() -> Self {
        Self {
            personas: RwLock::new(HashMap::new()),
            active_id: RwLock::new(None),
            db: None,
        }
    }

    /// Create a registry backed by SQLite.
    pub fn with_db(db: Arc<Database>) -> Self {
        Self {
            personas: RwLock::new(HashMap::new()),
            active_id: RwLock::new(None),
            db: Some(db),
        }
    }

    /// Initialize from database: load all stored personas and active id.
    pub async fn init_from_db(&self) -> Result<()> {
        let Some(ref db) = self.db else {
            return Ok(());
        };

        let rows = db.load_personas().await?;
        let mut map = HashMap::new();
        let mut default_id: Option<String> = None;

        for row in rows {
            let variables: HashMap<String, String> = serde_json::from_str(&row.variables)
                .unwrap_or_default();
            let persona = Persona {
                id: row.id.clone(),
                name: row.name,
                system_prompt: row.system_prompt,
                variables,
                is_default: row.is_default != 0,
                description: row.description,
            };
            if persona.is_default && default_id.is_none() {
                default_id = Some(row.id.clone());
            }
            map.insert(row.id, Arc::new(persona));
        }

        let mut lock = self.personas.write().await;
        *lock = map;
        drop(lock);

        if let Some(active) = db.load_active_persona().await? {
            let mut active_lock = self.active_id.write().await;
            *active_lock = Some(active);
        } else if let Some(id) = default_id {
            let mut active_lock = self.active_id.write().await;
            *active_lock = Some(id);
        }

        info!("[PersonaRegistry] Loaded {} persona(s) from DB", self.personas.read().await.len());
        Ok(())
    }

    /// Persist a single persona to DB.
    async fn persist_persona(&self, persona: &Persona) -> Result<()> {
        let Some(ref db) = self.db else {
            return Ok(());
        };
        let variables_json = serde_json::to_string(&persona.variables)
            .unwrap_or_else(|_| "{}".to_string());
        db.save_persona(
            &persona.id,
            &persona.name,
            &persona.system_prompt,
            &variables_json,
            persona.is_default,
            persona.description.as_deref(),
        ).await
    }

    /// Remove a persona from DB.
    async fn remove_persona_db(&self, id: &str) -> Result<()> {
        let Some(ref db) = self.db else {
            return Ok(());
        };
        db.delete_persona(id).await
    }

    /// Persist active persona ID to DB.
    async fn persist_active(&self, id: &str) -> Result<()> {
        let Some(ref db) = self.db else {
            return Ok(());
        };
        db.save_active_persona(id).await
    }

    /// Load personas from a list, replacing any existing entries.
    /// The first persona marked `is_default` becomes active automatically.
    pub async fn load(&self, personas: Vec<Persona>) {
        let mut map = HashMap::new();
        let mut default_id: Option<String> = None;

        for p in personas {
            if p.is_default && default_id.is_none() {
                default_id = Some(p.id.clone());
            }
            map.insert(p.id.clone(), Arc::new(p));
        }

        let mut lock = self.personas.write().await;
        *lock = map;
        drop(lock);

        if let Some(id) = default_id {
            let mut active = self.active_id.write().await;
            *active = Some(id.clone());
            drop(active);
            let _ = self.persist_active(&id).await;
        }

        info!("[PersonaRegistry] Loaded {} persona(s)", self.personas.read().await.len());
    }

    /// Register a single persona.
    pub async fn register(&self, persona: Persona) {
        let id = persona.id.clone();
        let _ = self.persist_persona(&persona).await;
        let mut lock = self.personas.write().await;
        lock.insert(id.clone(), Arc::new(persona));
        info!("[PersonaRegistry] Registered persona: {}", id);
    }

    /// Remove a persona.  Cannot remove the currently active one.
    pub async fn unregister(&self, id: &str) -> Result<()> {
        let active = self.active_id.read().await;
        if active.as_deref() == Some(id) {
            return Err(AstrBotError::Validation(
                format!("Cannot remove active persona: {}", id)
            ));
        }
        drop(active);

        let _ = self.remove_persona_db(id).await;
        let mut lock = self.personas.write().await;
        lock.remove(id);
        info!("[PersonaRegistry] Unregistered persona: {}", id);
        Ok(())
    }

    /// Get a persona by ID.
    pub async fn get(&self, id: &str) -> Option<Arc<Persona>> {
        let lock = self.personas.read().await;
        lock.get(id).cloned()
    }

    /// Get the currently active persona.
    pub async fn active(&self) -> Option<Arc<Persona>> {
        let id = self.active_id.read().await;
        if let Some(ref id_str) = *id {
            let lock = self.personas.read().await;
            lock.get(id_str).cloned()
        } else {
            None
        }
    }

    /// Get the ID of the currently active persona.
    pub async fn active_id(&self) -> Option<String> {
        self.active_id.read().await.clone()
    }

    /// List all registered personas.
    pub async fn list(&self) -> Vec<Arc<Persona>> {
        let lock = self.personas.read().await;
        lock.values().cloned().collect()
    }

    /// Switch to another persona by ID.
    pub async fn switch(&self, id: &str) -> Result<Arc<Persona>> {
        let lock = self.personas.read().await;
        let persona = lock
            .get(id)
            .cloned()
            .ok_or_else(|| AstrBotError::NotFound(format!("persona: {}", id)))?;
        drop(lock);

        let mut active = self.active_id.write().await;
        *active = Some(id.to_string());
        drop(active);
        let _ = self.persist_active(id).await;

        info!("[PersonaRegistry] Switched to persona: {} ({})", persona.name, id);
        Ok(persona)
    }

    /// Load personas from a YAML / TOML / JSON string (Serde-compatible).
    pub async fn load_from_str(&self, content: &str, format: ConfigFormat) -> Result<()> {
        let personas: Vec<Persona> = match format {
            ConfigFormat::Yaml => serde_yaml::from_str(content)
                .map_err(|e| AstrBotError::Serialization(format!("YAML parse error: {}", e)))?,
            ConfigFormat::Json => serde_json::from_str(content)
                .map_err(|e| AstrBotError::Serialization(format!("JSON parse error: {}", e)))?,
            ConfigFormat::Toml => toml::from_str(content)
                .map_err(|e| AstrBotError::Serialization(format!("TOML parse error: {}", e)))?,
        };
        self.load(personas).await;
        Ok(())
    }

    /// Build the system prompt for the currently active persona.
    /// Returns a generic default if no persona is active.
    pub async fn build_system_prompt(&self, overrides: Option<&HashMap<String, String>>) -> String {
        if let Some(persona) = self.active().await {
            persona.render(overrides)
        } else {
            warn!("[PersonaRegistry] No active persona — using fallback system prompt");
            "You are a helpful assistant.".to_string()
        }
    }
}

/// Supported config serialization formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Yaml,
    Json,
    Toml,
}

// ---------------------------------------------------------------------------
// Built-in default personas
// ---------------------------------------------------------------------------

/// The built-in default persona shipped with AstrBot.
pub fn default_persona() -> Persona {
    Persona::new(
        "default",
        "Default",
        "You are {{name}}, a helpful AI assistant.\n\nYour current mood is {{mood}}.",
    )
    .with_variable("name", "AstrBot")
    .with_variable("mood", "cheerful")
    .with_default(true)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_persona_registry_load_get_list_switch() {
        let registry = PersonaRegistry::new();

        // Load two personas, one marked default
        let p1 = Persona::new("p1", "Friendly", "Be friendly, {{name}}.")
            .with_variable("name", "Friend")
            .with_default(true);
        let p2 = Persona::new("p2", "Strict", "Be strict, {{name}}.")
            .with_variable("name", "Sir");

        registry.load(vec![p1, p2]).await;

        // list
        let all = registry.list().await;
        assert_eq!(all.len(), 2);

        // get by id
        let got = registry.get("p1").await.unwrap();
        assert_eq!(got.name, "Friendly");

        // active is p1 (default)
        assert_eq!(registry.active_id().await, Some("p1".to_string()));

        // switch to p2
        let switched = registry.switch("p2").await.unwrap();
        assert_eq!(switched.id, "p2");
        assert_eq!(registry.active_id().await, Some("p2".to_string()));
    }

    #[tokio::test]
    async fn test_persona_registry_switch_not_found() {
        let registry = PersonaRegistry::new();
        let result = registry.switch("missing").await;
        assert!(matches!(result, Err(AstrBotError::NotFound(_))));
    }

    #[test]
    fn test_persona_render_with_defaults() {
        let p = Persona::new("test", "Test", "Hello {{name}}, mood: {{mood}}.")
            .with_variable("name", "AstrBot")
            .with_variable("mood", "happy");

        let rendered = p.render(None);
        assert_eq!(rendered, "Hello AstrBot, mood: happy.");
    }

    #[test]
    fn test_persona_render_with_overrides() {
        let p = Persona::new("test", "Test", "Hello {{name}}, mood: {{mood}}.")
            .with_variable("name", "AstrBot")
            .with_variable("mood", "happy");

        let mut overrides = HashMap::new();
        overrides.insert("mood".to_string(), "grumpy".to_string());

        let rendered = p.render(Some(&overrides));
        assert_eq!(rendered, "Hello AstrBot, mood: grumpy.");
    }

    #[test]
    fn test_persona_render_missing_variable_preserved() {
        // If a variable has no default and no override, the placeholder stays
        let p = Persona::new("test", "Test", "Hello {{name}}, missing: {{unknown}}.");
        let rendered = p.render(None);
        assert_eq!(rendered, "Hello {{name}}, missing: {{unknown}}.");
    }

    #[tokio::test]
    async fn test_persona_build_system_prompt_fallback() {
        let registry = PersonaRegistry::new();
        let prompt = registry.build_system_prompt(None).await;
        assert_eq!(prompt, "You are a helpful assistant.");
    }

    #[tokio::test]
    async fn test_persona_unregister_active_fails() {
        let registry = PersonaRegistry::new();
        let p = Persona::new("active", "Active", "sys")
            .with_default(true);
        registry.register(p).await;
        // Make it the active persona
        registry.switch("active").await.unwrap();

        let result = registry.unregister("active").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_from_json() {
        let registry = PersonaRegistry::new();
        let json = r#"[
            {
                "id": "kimi",
                "name": "Kimi",
                "system_prompt": "You are {{name}}.",
                "variables": {"name": "Kimi"},
                "is_default": true
            }
        ]"#;
        registry.load_from_str(json, ConfigFormat::Json).await.unwrap();

        let active = registry.active().await.unwrap();
        assert_eq!(active.name, "Kimi");
    }

    #[test]
    fn test_default_persona() {
        let p = default_persona();
        assert_eq!(p.id, "default");
        assert!(p.is_default);
        let rendered = p.render(None);
        assert!(rendered.contains("AstrBot"));
        assert!(rendered.contains("cheerful"));
    }
}
