use astrbot_core::provider::{ChatConfig, ChatMessage, Provider};
use astrbot_provider::openai::OpenAiProvider;

#[tokio::test]
async fn test_openai_real_chat() {
 let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
 if key.is_empty() {
 eprintln!("OPENAI_API_KEY not set — skipping");
 return;
 }
 let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".to_string());
 let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
 let provider = OpenAiProvider::new(
 "test-openai".to_string(), key, base_url.clone(), model.clone(),
 );
 let messages = vec![ChatMessage::user("say a one-word greeting")];
 let config = ChatConfig { model: Some(model), max_tokens: Some(10), ..Default::default() };
 let response = provider.chat(messages, config).await.expect("OpenAI chat should succeed");
 assert!(!response.content.is_empty());
 println!("OpenAI real chat ({}): {}", base_url, response.content.trim());
}
