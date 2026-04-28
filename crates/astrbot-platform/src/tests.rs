use crate::adapter::PlatformAdapter;
use astrbot_core::message::MessageHandler;
use crate::{QQAdapter, TelegramAdapter};
use astrbot_core::message::AstrBotMessage;
use astrbot_core::platform::MessageSource;
use astrbot_core::message::MessageChain;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::collections::HashMap;

struct MockHandler {
    count: AtomicUsize,
}

#[async_trait::async_trait]
impl MessageHandler for MockHandler {
    async fn on_message(&self, _message: AstrBotMessage) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn test_qq_adapter_lifecycle() {
    let mut adapter = QQAdapter::new(
        "127.0.0.1".to_string(),
        0, // port 0 = random available port
        "http://localhost:8080".to_string(),
        None,
    );
    
    assert_eq!(adapter.metadata().name, "QQ");
    assert!(!adapter.health_check().await.unwrap());
    
    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    // QQ reverse WS: server is listening but no client connected yet
    assert!(adapter.health_check().await.unwrap());
    
    adapter.stop().await.unwrap();
    assert!(!adapter.health_check().await.unwrap());
}

#[tokio::test]
async fn test_telegram_adapter_lifecycle() {
    let mut adapter = TelegramAdapter::new(
        "test_token".to_string(),
        None,
        None,
    );
    
    assert_eq!(adapter.metadata().name, "Telegram");
    assert!(!adapter.health_check().await.unwrap());
    
    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    // Telegram polling: running but no successful poll yet
    assert!(adapter.health_check().await.unwrap());
    
    adapter.stop().await.unwrap();
    assert!(!adapter.health_check().await.unwrap());
}

#[tokio::test]
async fn test_qq_adapter_send_not_running() {
    let adapter = QQAdapter::new(
        "127.0.0.1".to_string(),
        0,
        "http://localhost:8080".to_string(),
        None,
    );
    
    let source = MessageSource {
        platform: astrbot_core::platform::PlatformType::Aiocqhttp,
        session_id: "12345".to_string(),
        message_id: "1".to_string(),
        user_id: "user1".to_string(),
    };
    let chain = MessageChain::default();
    
    let result = adapter.send_message(&source, &chain).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_telegram_adapter_send_not_running() {
    let adapter = TelegramAdapter::new(
        "test_token".to_string(),
        None,
        None,
    );
    
    let source = MessageSource {
        platform: astrbot_core::platform::PlatformType::Telegram,
        session_id: "12345".to_string(),
        message_id: "1".to_string(),
        user_id: "user1".to_string(),
    };
    let chain = MessageChain::default();
    
    let result = adapter.send_message(&source, &chain).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_qq_message_handler() {
    let handler = Arc::new(MockHandler { count: AtomicUsize::new(0) });
    let mut adapter = QQAdapter::new(
        "127.0.0.1".to_string(),
        0,
        "http://localhost:8080".to_string(),
        None,
    );
    adapter.set_message_handler(handler.clone());
    
    // Just verify it compiles and runs
    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    adapter.stop().await.unwrap();
}

#[tokio::test]
async fn test_telegram_message_handler() {
    let handler = Arc::new(MockHandler { count: AtomicUsize::new(0) });
    let mut adapter = TelegramAdapter::new(
        "test_token".to_string(),
        None,
        None,
    );
    adapter.set_message_handler(handler.clone());
    
    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    adapter.stop().await.unwrap();
}

#[tokio::test]
async fn test_onebot_message_parsing() {
    use crate::qq::{OneBotMessageEvent, OneBotSender};
    
    let event = OneBotMessageEvent {
        post_type: "message".to_string(),
        message_type: "group".to_string(),
        user_id: 123456,
        group_id: Some(789012),
        message_id: 42,
        message: vec![
            crate::qq::OneBotSegment {
                seg_type: "text".to_string(),
                data: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("text".to_string(), serde_json::Value::String("hello".to_string()));
                    m
                },
            }
        ],
        raw_message: "hello".to_string(),
        sender: OneBotSender {
            user_id: 123456,
            nickname: Some("TestUser".to_string()),
            card: None,
            role: Some("member".to_string()),
        },
        self_id: 999999,
    };
    
    let msg = super::qq::parse_onebot_message(&event);
    assert_eq!(msg.message_id, "42");
    assert_eq!(msg.session_id, "789012");
    assert_eq!(msg.sender.user_id, "123456");
    assert_eq!(msg.chain.plain_text(), "hello");
    assert!(msg.chain.contains("plain"));
}

#[tokio::test]
async fn test_mattermost_adapter_lifecycle() {
    let shared = crate::mattermost::MattermostShared::new(
        "ws://localhost:8065/api/v4/websocket",
        "http://localhost:8065",
        "test_token",
        "bot_id",
    );
    let mut adapter = crate::mattermost::MattermostAdapter::new(shared);

    assert_eq!(adapter.metadata().name, "Mattermost");
    assert!(!adapter.health_check().await.unwrap());

    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    // WS not actually connected in test, so connected=false
    // but running=true means health_check returns running && connected
    assert!(!adapter.health_check().await.unwrap());

    adapter.stop().await.unwrap();
    assert!(!adapter.health_check().await.unwrap());
}

#[tokio::test]
async fn test_webchat_adapter_lifecycle() {
    let (mut adapter, shared) = crate::webchat::WebchatAdapter::new(100);

    assert_eq!(adapter.metadata().name, "WebChat");
    assert!(!adapter.health_check().await.unwrap());

    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    assert!(adapter.health_check().await.unwrap());

    // Inject a message
    let msg = crate::webchat::WebchatIncomingMessage {
        session_id: "session1".to_string(),
        user_id: "user1".to_string(),
        username: "Alice".to_string(),
        text: "hello".to_string(),
        message_id: "msg1".to_string(),
    };
    shared.tx.send(msg).await.unwrap();

    // Give the loop a moment to process
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    adapter.stop().await.unwrap();
    assert!(!adapter.health_check().await.unwrap());
}

#[tokio::test]
async fn test_webhook_adapter_lifecycle() {
    let (mut adapter, _shared) = crate::webhook::WebhookAdapter::new("/webhook/test", Some("secret".to_string()), 100);

    assert_eq!(adapter.metadata().name, "Webhook");
    assert!(!adapter.health_check().await.unwrap());

    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    assert!(adapter.health_check().await.unwrap());

    adapter.stop().await.unwrap();
    assert!(!adapter.health_check().await.unwrap());
}

#[tokio::test]
async fn test_mattermost_adapter_send_not_running() {
    let shared = crate::mattermost::MattermostShared::new(
        "ws://localhost:8065/api/v4/websocket",
        "http://localhost:8065",
        "test_token",
        "bot_id",
    );
    let adapter = crate::mattermost::MattermostAdapter::new(shared);

    let source = MessageSource {
        platform: astrbot_core::platform::PlatformType::Mattermost,
        session_id: "channel1".to_string(),
        message_id: "1".to_string(),
        user_id: "user1".to_string(),
    };
    let chain = MessageChain::default();

    let result = adapter.send_message(&source, &chain).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_webchat_adapter_send_not_running() {
    let (adapter, _shared) = crate::webchat::WebchatAdapter::new(100);

    let source = MessageSource {
        platform: astrbot_core::platform::PlatformType::Webchat,
        session_id: "session1".to_string(),
        message_id: "1".to_string(),
        user_id: "user1".to_string(),
    };
    let chain = MessageChain::default();

    let result = adapter.send_message(&source, &chain).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_webhook_adapter_send_not_running() {
    let (adapter, _shared) = crate::webhook::WebhookAdapter::new("/webhook/test", None, 100);

    let source = MessageSource {
        platform: astrbot_core::platform::PlatformType::Webhook,
        session_id: "session1".to_string(),
        message_id: "1".to_string(),
        user_id: "user1".to_string(),
    };
    let chain = MessageChain::default();

    let result = adapter.send_message(&source, &chain).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_mattermost_message_handler() {
    let handler = Arc::new(MockHandler { count: AtomicUsize::new(0) });
    let shared = crate::mattermost::MattermostShared::new(
        "ws://localhost:8065/api/v4/websocket",
        "http://localhost:8065",
        "test_token",
        "bot_id",
    );
    let mut adapter = crate::mattermost::MattermostAdapter::new(shared);
    adapter.set_message_handler(handler.clone());

    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    adapter.stop().await.unwrap();
}

#[tokio::test]
async fn test_webchat_message_handler() {
    let handler = Arc::new(MockHandler { count: AtomicUsize::new(0) });
    let (mut adapter, _shared) = crate::webchat::WebchatAdapter::new(100);
    adapter.set_message_handler(handler.clone());

    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    adapter.stop().await.unwrap();
}

#[tokio::test]
async fn test_webhook_message_handler() {
    let handler = Arc::new(MockHandler { count: AtomicUsize::new(0) });
    let (mut adapter, _shared) = crate::webhook::WebhookAdapter::new("/webhook/test", None, 100);
    adapter.set_message_handler(handler.clone());

    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();
    adapter.stop().await.unwrap();
}

#[tokio::test]
async fn test_webchat_inject_and_send() {
    let (mut adapter, shared) = crate::webchat::WebchatAdapter::new(100);
    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();

    let msg = crate::webchat::WebchatIncomingMessage {
        session_id: "session1".to_string(),
        user_id: "user1".to_string(),
        username: "Bob".to_string(),
        text: "hi bot".to_string(),
        message_id: "msg2".to_string(),
    };
    shared.tx.send(msg).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send a reply
    let source = MessageSource {
        platform: astrbot_core::platform::PlatformType::Webchat,
        session_id: "session1".to_string(),
        message_id: "msg2".to_string(),
        user_id: "user1".to_string(),
    };
    let chain = MessageChain::new().text("Hello Bob");
    adapter.send_message(&source, &chain).await.unwrap();

    let outgoing = adapter.pop_outgoing().await;
    assert_eq!(outgoing, Some(("session1".to_string(), "Hello Bob".to_string())));

    adapter.stop().await.unwrap();
}

#[tokio::test]
async fn test_webhook_payload_processing() {
    let (mut adapter, shared) = crate::webhook::WebhookAdapter::new("/webhook/test", None, 100);
    adapter.initialize().await.unwrap();
    adapter.start().await.unwrap();

    let payload = crate::webhook::WebhookPayload {
        headers: HashMap::new(),
        body: r#"{"text":"hello from webhook","user_id":"user1","session_id":"room1"}"#.to_string(),
        source_ip: Some("127.0.0.1".to_string()),
    };
    shared.tx.send(payload).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    adapter.stop().await.unwrap();
}
