use astrbot_core::provider::{ChatConfig, ChatMessage, Provider};
use astrbot_provider::openai::OpenAiProvider;

#[tokio::test]
async fn test_openrouter_real_chat() {
 let key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
 if key.is_empty() {
 eprintln!("OPENROUTER_API_KEY not set — skipping");
 return;
 }
 let provider = OpenAiProvider::new(
 "test-openrouter".to_string(), key,
 "https://openrouter.ai/api/v1".to_string(),
 "openai/gpt-4o-mini".to_string(),
 );
 let messages = vec![ChatMessage::user("say a one-word greeting")];
 let config = ChatConfig {
 model: Some("openai/gpt-4o-mini".to_string()),
 max_tokens: Some(10), ..Default::default()
 };
 let response = provider.chat(messages, config).await
 .expect("OpenRouter chat should succeed");
 assert!(!response.content.is_empty());
 println!("OpenRouter real chat: {}", response.content.trim());
}
