use crate::adapter::PlatformAdapter;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{
    AstrBotMessage, HandlerRef, MessageChain, MessageComponent, MessageHandler, MessageMember,
    MessageType,
};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// OneBot v11 data models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneBotSegment {
    #[serde(rename = "type")]
    pub seg_type: String,
    #[serde(default)]
    pub data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OneBotMessageEvent {
    #[serde(rename = "post_type")]
    pub post_type: String,
    #[serde(rename = "message_type")]
    pub message_type: String,
    #[serde(rename = "user_id")]
    pub user_id: i64,
    #[serde(rename = "group_id", default)]
    pub group_id: Option<i64>,
    #[serde(rename = "message_id")]
    pub message_id: i32,
    #[serde(default)]
    pub message: Vec<OneBotSegment>,
    #[serde(default)]
    pub raw_message: String,
    pub sender: OneBotSender,
    #[serde(rename = "self_id")]
    pub self_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OneBotSender {
    #[serde(rename = "user_id")]
    pub user_id: i64,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub card: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Serialize)]
struct OneBotSendMsgRequest {
    #[serde(rename = "message_type")]
    pub message_type: String,
    #[serde(rename = "user_id", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    #[serde(rename = "group_id", skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,
    pub message: Vec<OneBotSegment>,
}

#[derive(Debug, Deserialize)]
struct OneBotApiResponse {
    pub status: String,
    #[serde(default)]
    pub retcode: i32,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Message format conversion
// ---------------------------------------------------------------------------

fn onebot_to_chain(segments: &[OneBotSegment]) -> MessageChain {
    let mut chain = MessageChain::new();
    for seg in segments {
        let comp = match seg.seg_type.as_str() {
            "text" => {
                let text = seg
                    .data
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                MessageComponent::Plain { text }
            }
            "image" => {
                let url = seg
                    .data
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let file_id = seg
                    .data
                    .get("file_id")
                    .or_else(|| seg.data.get("file"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                MessageComponent::Image {
                    url,
                    file_id,
                    base64: None,
                }
            }
            "at" => {
                let target = seg
                    .data
                    .get("qq")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                let display = seg
                    .data
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                MessageComponent::At { target, display }
            }
            "reply" => {
                let message_id = seg
                    .data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                MessageComponent::Reply {
                    message_id,
                    chain: None,
                }
            }
            "voice" | "record" => {
                let url = seg
                    .data
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let file_id = seg
                    .data
                    .get("file_id")
                    .or_else(|| seg.data.get("file"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                MessageComponent::Voice {
                    url,
                    file_id,
                    base64: None,
                }
            }
            "file" => {
                let name = seg
                    .data
                    .get("file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let url = seg
                    .data
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let file_id = seg
                    .data
                    .get("file_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                MessageComponent::File { name, url, file_id }
            }
            _ => MessageComponent::Plain {
                text: format!("[unsupported:{}]", seg.seg_type),
            },
        };
        chain.0.push(comp);
    }
    chain
}

fn chain_to_onebot(chain: &MessageChain) -> Vec<OneBotSegment> {
    let mut segments = Vec::new();
    for comp in &chain.0 {
        let seg = match comp {
            MessageComponent::Plain { text } => OneBotSegment {
                seg_type: "text".to_string(),
                data: {
                    let mut m = HashMap::new();
                    m.insert("text".to_string(), serde_json::Value::String(text.clone()));
                    m
                },
            },
            MessageComponent::At { target, display } => OneBotSegment {
                seg_type: "at".to_string(),
                data: {
                    let mut m = HashMap::new();
                    m.insert("qq".to_string(), serde_json::Value::String(target.clone()));
                    if let Some(d) = display {
                        m.insert("name".to_string(), serde_json::Value::String(d.clone()));
                    }
                    m
                },
            },
            MessageComponent::Image {
                url,
                file_id,
                base64,
            } => OneBotSegment {
                seg_type: "image".to_string(),
                data: {
                    let mut m = HashMap::new();
                    if let Some(u) = url {
                        m.insert("url".to_string(), serde_json::Value::String(u.clone()));
                    }
                    if let Some(f) = file_id {
                        m.insert("file".to_string(), serde_json::Value::String(f.clone()));
                    }
                    if let Some(b) = base64 {
                        m.insert("base64".to_string(), serde_json::Value::String(b.clone()));
                    }
                    m
                },
            },
            MessageComponent::Reply { message_id, .. } => OneBotSegment {
                seg_type: "reply".to_string(),
                data: {
                    let mut m = HashMap::new();
                    m.insert(
                        "id".to_string(),
                        serde_json::Value::String(message_id.clone()),
                    );
                    m
                },
            },
            MessageComponent::Voice {
                url,
                file_id,
                base64,
            } => OneBotSegment {
                seg_type: "record".to_string(),
                data: {
                    let mut m = HashMap::new();
                    if let Some(u) = url {
                        m.insert("url".to_string(), serde_json::Value::String(u.clone()));
                    }
                    if let Some(f) = file_id {
                        m.insert("file".to_string(), serde_json::Value::String(f.clone()));
                    }
                    if let Some(b) = base64 {
                        m.insert("base64".to_string(), serde_json::Value::String(b.clone()));
                    }
                    m
                },
            },
            MessageComponent::File { name, url, file_id } => OneBotSegment {
                seg_type: "file".to_string(),
                data: {
                    let mut m = HashMap::new();
                    m.insert("file".to_string(), serde_json::Value::String(name.clone()));
                    if let Some(u) = url {
                        m.insert("url".to_string(), serde_json::Value::String(u.clone()));
                    }
                    if let Some(f) = file_id {
                        m.insert("file_id".to_string(), serde_json::Value::String(f.clone()));
                    }
                    m
                },
            },
            MessageComponent::Json { data } => OneBotSegment {
                seg_type: "json".to_string(),
                data: {
                    let mut m = HashMap::new();
                    m.insert("data".to_string(), data.clone());
                    m
                },
            },
            MessageComponent::Xml { data } => OneBotSegment {
                seg_type: "xml".to_string(),
                data: {
                    let mut m = HashMap::new();
                    m.insert("data".to_string(), serde_json::Value::String(data.clone()));
                    m
                },
            },
            MessageComponent::Forward { message_id, .. } => OneBotSegment {
                seg_type: "forward".to_string(),
                data: {
                    let mut m = HashMap::new();
                    m.insert(
                        "id".to_string(),
                        serde_json::Value::String(message_id.clone()),
                    );
                    m
                },
            },
        };
        segments.push(seg);
    }
    segments
}

pub(crate) fn parse_onebot_message(event: &OneBotMessageEvent) -> AstrBotMessage {
    let msg_type = match event.message_type.as_str() {
        "private" => MessageType::Private,
        "group" => MessageType::Group,
        _ => MessageType::Unknown,
    };

    let session_id = if let Some(gid) = event.group_id {
        gid.to_string()
    } else {
        event.user_id.to_string()
    };

    let sender = MessageMember {
        user_id: event.sender.user_id.to_string(),
        nickname: event.sender.nickname.clone(),
        card: event.sender.card.clone(),
        role: event.sender.role.clone(),
        is_self: event.sender.user_id == event.self_id,
    };

    AstrBotMessage {
        message_id: event.message_id.to_string(),
        timestamp: chrono::Utc::now(),
        platform: PlatformType::Aiocqhttp,
        session_id,
        sender,
        message_type: msg_type,
        chain: onebot_to_chain(&event.message),
        raw_payload: None,
    }
}

// ---------------------------------------------------------------------------
// WebSocket server
// ---------------------------------------------------------------------------

async fn handle_ws_client(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    _access_token: Option<String>,
) {
    let (mut write, mut read) = ws_stream.split();

    while running.load(Ordering::Relaxed) {
        match timeout(Duration::from_secs(30), read.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                let json: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Echo heartbeat
                if json.get("post_type").and_then(|v| v.as_str()) == Some("meta_event") {
                    let _ = write
                        .send(tokio_tungstenite::tungstenite::Message::Text(text))
                        .await;
                    continue;
                }

                // Parse message event
                if let Ok(event) = serde_json::from_value::<OneBotMessageEvent>(json.clone()) {
                    if event.post_type == "message" {
                        let msg = parse_onebot_message(&event);
                        let handler_opt = handler.lock().unwrap().clone();
                        if let Some(ref handler) = handler_opt {
                            handler.on_message(msg).await;
                        }
                    }
                }
            }
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))) => {
                info!("[QQ] WebSocket closed by client");
                break;
            }
            Ok(Some(Err(e))) => {
                warn!("[QQ] WebSocket read error: {}", e);
                break;
            }
            Ok(None) => break,
            Ok(Some(Ok(_))) => {} // ignore Binary, Ping, Pong, Frame
            Err(_) => {}          // timeout, loop back to check running
        }
    }

    connected.store(false, Ordering::Relaxed);
    info!("[QQ] OneBot client disconnected");
}

async fn run_ws_server(
    host: String,
    port: u16,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    access_token: Option<String>,
) {
    let addr = format!("{}:{}", host, port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => {
            info!("[QQ] Reverse WebSocket server listening on {}", addr);
            l
        }
        Err(e) => {
            error!("[QQ] Failed to bind WS server on {}: {}", addr, e);
            return;
        }
    };

    while running.load(Ordering::Relaxed) {
        let (stream, peer) = match timeout(Duration::from_secs(1), listener.accept()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                error!("[QQ] WS accept error: {}", e);
                continue;
            }
            Err(_) => continue,
        };

        info!("[QQ] OneBot client connected from {:?}", peer);
        connected.store(true, Ordering::Relaxed);

        let ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                error!("[QQ] WebSocket handshake failed: {}", e);
                connected.store(false, Ordering::Relaxed);
                continue;
            }
        };

        let handler_clone = Arc::clone(&handler);
        let connected_clone = Arc::clone(&connected);
        let running_clone = Arc::clone(&running);
        let token_clone = access_token.clone();

        tokio::spawn(async move {
            handle_ws_client(
                ws_stream,
                handler_clone,
                connected_clone,
                running_clone,
                token_clone,
            )
            .await;
        });
    }
}

// ---------------------------------------------------------------------------
// QQ Adapter
// ---------------------------------------------------------------------------

pub struct QQAdapter {
    metadata: PlatformMetadata,
    ws_host: String,
    ws_port: u16,
    http_url: String,
    access_token: Option<String>,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    server_handle: Mutex<Option<JoinHandle<()>>>,
    http_client: reqwest::Client,
}

impl QQAdapter {
    pub fn new(
        ws_host: String,
        ws_port: u16,
        http_url: String,
        access_token: Option<String>,
    ) -> Self {
        Self {
            metadata: PlatformMetadata {
                id: "qq".to_string(),
                name: "QQ".to_string(),
                platform_type: PlatformType::Aiocqhttp,
                enabled: true,
                extra: HashMap::new(),
            },
            ws_host,
            ws_port,
            http_url,
            access_token,
            connected: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            handler: Arc::new(std::sync::Mutex::new(None)),
            server_handle: Mutex::new(None),
            http_client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl PlatformAdapter for QQAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[QQ] Initializing OneBot v11 adapter...");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        info!("[QQ] Starting adapter on {}:{}", self.ws_host, self.ws_port);
        self.running.store(true, Ordering::Relaxed);

        let running = Arc::clone(&self.running);
        let connected = Arc::clone(&self.connected);
        let handler = Arc::clone(&self.handler);
        let host = self.ws_host.clone();
        let port = self.ws_port;
        let token = self.access_token.clone();

        let handle = tokio::spawn(async move {
            run_ws_server(host, port, running, connected, handler, token).await;
        });

        let mut guard = self.server_handle.lock().await;
        *guard = Some(handle);

        info!("[QQ] Adapter started (waiting for OneBot client connection)");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[QQ] Stopping adapter...");
        self.running.store(false, Ordering::Relaxed);
        self.connected.store(false, Ordering::Relaxed);

        let mut guard = self.server_handle.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: "adapter not running".to_string(),
            });
        }

        let (msg_type, user_id, group_id) = match target.platform {
            PlatformType::Aiocqhttp => {
                let uid = target.user_id.parse::<i64>().ok();
                let gid = target.session_id.parse::<i64>().ok();
                if uid.is_some() && gid == uid {
                    ("private".to_string(), uid, None)
                } else {
                    ("group".to_string(), uid, gid)
                }
            }
            _ => {
                return Err(AstrBotError::Platform {
                    adapter: "QQ".to_string(),
                    message: "unsupported platform type for QQ adapter".to_string(),
                })
            }
        };

        let req_body = OneBotSendMsgRequest {
            message_type: msg_type,
            user_id,
            group_id,
            message: chain_to_onebot(chain),
        };

        let url = format!("{}/send_msg", self.http_url);
        let mut request = self.http_client.post(&url).json(&req_body);

        if let Some(ref token) = self.access_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request.send().await.map_err(|e| AstrBotError::Platform {
            adapter: "QQ".to_string(),
            message: format!("HTTP request failed: {}", e),
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: format!("OneBot API error: {} - {}", status, body),
            });
        }

        let api_resp: OneBotApiResponse =
            response.json().await.map_err(|e| AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: format!("Failed to parse API response: {}", e),
            })?;

        if api_resp.retcode != 0 {
            return Err(AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: format!("OneBot API returned retcode: {}", api_resp.retcode),
            });
        }

        info!("[QQ] Message sent successfully");
        Ok(())
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let mut reply_chain = MessageChain::new();
        reply_chain.0.push(MessageComponent::Reply {
            message_id: original.message_id.clone(),
            chain: None,
        });
        reply_chain.0.extend(chain.0.clone());

        let source = MessageSource {
            platform: PlatformType::Aiocqhttp,
            session_id: original.session_id.clone(),
            message_id: original.message_id.clone(),
            user_id: original.sender.user_id.clone(),
        };
        self.send_message(&source, &reply_chain).await
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.running.load(Ordering::Relaxed))
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        let mut h = self.handler.lock().unwrap();
        *h = Some(handler);
    }

    async fn send_voice(&self, target: &MessageSource, data: Vec<u8>, format: &str) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: "adapter not running".to_string(),
            });
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp_path = format!("/tmp/astrbot_voice_{}.{}", timestamp, format);
        if let Err(e) = tokio::fs::write(&tmp_path, &data).await {
            return Err(AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: format!("Failed to write temp voice file: {}", e),
            });
        }
        let mut chain = MessageChain::new();
        chain.0.push(MessageComponent::Voice {
            url: Some(tmp_path.clone()),
            file_id: None,
            base64: None,
        });
        let result = self.send_message(target, &chain).await;
        let _ = tokio::fs::remove_file(&tmp_path).await;
        result
    }
}
