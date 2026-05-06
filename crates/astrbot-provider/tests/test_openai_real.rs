use astrbot_core::provider::{ChatConfig, ChatMessage, Provider};
use astrbot_provider::openai::OpenAiProvider;

#[tokio::test]
async fn test_openai_real_chat() {
 let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
 if key.is_empty() {
 eprintln!("OPENAI_API_KEY not set — skipping");
 return;
 }
 let provider = OpenAiProvider::new(
 "test-openai".to_string(), key,
 "https://api.openai.com".to_string(), "gpt-4o-mini".to_string(),
 );
 let messages = vec![ChatMessage::user("say a one-word greeting")];
 let config = ChatConfig { model: Some("gpt-4o-mini".to_string()), max_tokens: Some(10), ..Default::default() };
 let response = provider.chat(messages, config).await.expect("OpenAI chat should succeed");
 assert!(!response.content.is_empty());
 println!("OpenAI real chat: {}", response.content.trim());
}
