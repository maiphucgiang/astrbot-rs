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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneBotRequestEvent {
    #[serde(rename = "post_type")]
    pub post_type: String,
    #[serde(rename = "request_type")]
    pub request_type: String,
    #[serde(rename = "sub_type", default)]
    pub sub_type: Option<String>,
    #[serde(rename = "user_id")]
    pub user_id: i64,
    #[serde(rename = "group_id", default)]
    pub group_id: Option<i64>,
    pub comment: Option<String>,
    pub flag: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OneBotGroupMember {
    #[serde(rename = "group_id")]
    pub group_id: i64,
    #[serde(rename = "user_id")]
    pub user_id: i64,
    pub nickname: String,
    #[serde(default)]
    pub card: Option<String>,
    #[serde(default)]
    pub sex: Option<String>,
    #[serde(default)]
    pub age: Option<i32>,
    #[serde(default)]
    pub area: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

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

pub(crate) fn parse_onebot_request(event: &OneBotRequestEvent) -> AstrBotMessage {
    let (msg_type, session_id) = if let Some(gid) = event.group_id {
        (MessageType::Group, gid.to_string())
    } else {
        (MessageType::Private, event.user_id.to_string())
    };
    let text = match event.request_type.as_str() {
        "friend" => format!(
            "[好友请求] 用户 {} 请求添加好友，备注：{}",
            event.user_id,
            event.comment.as_deref().unwrap_or("无")
        ),
        "group" => {
            let sub = event.sub_type.as_deref().unwrap_or("add");
            if sub == "invite" {
                format!(
                    "[群邀请] 用户 {} 邀请你加入群 {}",
                    event.user_id,
                    event.group_id.unwrap_or(0)
                )
            } else {
                format!(
                    "[入群申请] 用户 {} 申请加入群 {}，理由：{}",
                    event.user_id,
                    event.group_id.unwrap_or(0),
                    event.comment.as_deref().unwrap_or("无")
                )
            }
        }
        _ => format!("[请求事件] {}", event.request_type),
    };
    let raw = serde_json::to_value(event).ok();
    AstrBotMessage {
        message_id: event.flag.clone(),
        timestamp: chrono::Utc::now(),
        platform: PlatformType::Aiocqhttp,
        session_id,
        sender: MessageMember {
            user_id: event.user_id.to_string(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        },
        message_type: msg_type,
        chain: MessageChain::new().text(text),
        raw_payload: raw,
    }
}

pub(crate) fn build_reply_chain(original: &AstrBotMessage, chain: &MessageChain) -> MessageChain {
    let mut reply = MessageChain::new();
    reply.0.push(MessageComponent::Reply {
        message_id: original.message_id.clone(),
        chain: None,
    });
    if original.message_type == MessageType::Group {
        reply.0.push(MessageComponent::At {
            target: original.sender.user_id.clone(),
            display: original.sender.nickname.clone(),
        });
    }
    reply.0.extend(chain.0.clone());
    reply
}

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
                if json.get("post_type").and_then(|v| v.as_str()) == Some("meta_event") {
                    let _ = write
                        .send(tokio_tungstenite::tungstenite::Message::Text(text))
                        .await;
                    continue;
                }
                if let Ok(event) = serde_json::from_value::<OneBotMessageEvent>(json.clone()) {
                    if event.post_type == "message" {
                        let msg = parse_onebot_message(&event);
                        let handler_opt = handler.lock().unwrap().clone();
                        if let Some(ref handler) = handler_opt {
                            handler.on_message(msg).await;
                        }
                    }
                }
                if let Ok(req) = serde_json::from_value::<OneBotRequestEvent>(json.clone()) {
                    if req.post_type == "request" {
                        let msg = parse_onebot_request(&req);
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
            Ok(Some(Ok(_))) => {}
            Err(_) => {}
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

    async fn call_api(
        &self,
        endpoint: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: "adapter not running".to_string(),
            });
        }
        let url = format!("{}{}", self.http_url.trim_end_matches('/'), endpoint);
        let mut request = self.http_client.post(&url).json(body);
        if let Some(ref token) = self.access_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }
        let response = request.send().await.map_err(|e| AstrBotError::Platform {
            adapter: "QQ".to_string(),
            message: format!("HTTP request failed: {}", e),
        })?;
        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: format!("OneBot API error: {} - {}", status, body_text),
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
        Ok(api_resp.data.unwrap_or(serde_json::Value::Null))
    }

    pub async fn approve_friend_request(
        &self,
        flag: &str,
        approve: bool,
        remark: Option<&str>,
    ) -> Result<()> {
        let body =
            serde_json::json!({ "flag": flag, "approve": approve, "remark": remark.unwrap_or("") });
        self.call_api("/set_friend_add_request", &body).await?;
        info!(
            "[QQ] Friend request {} {}",
            flag,
            if approve { "approved" } else { "rejected" }
        );
        Ok(())
    }

    pub async fn approve_group_invite(
        &self,
        flag: &str,
        sub_type: &str,
        approve: bool,
        reason: Option<&str>,
    ) -> Result<()> {
        let body = serde_json::json!({ "flag": flag, "sub_type": sub_type, "approve": approve, "reason": reason.unwrap_or("") });
        self.call_api("/set_group_add_request", &body).await?;
        info!(
            "[QQ] Group request {} {}",
            flag,
            if approve { "approved" } else { "rejected" }
        );
        Ok(())
    }

    pub async fn get_group_member_info(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<OneBotGroupMember> {
        let body =
            serde_json::json!({ "group_id": group_id, "user_id": user_id, "no_cache": false });
        let data = self.call_api("/get_group_member_info", &body).await?;
        let member: OneBotGroupMember =
            serde_json::from_value(data).map_err(|e| AstrBotError::Platform {
                adapter: "QQ".to_string(),
                message: format!("Failed to parse member info: {}", e),
            })?;
        Ok(member)
    }

    pub async fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        let response =
            self.http_client
                .get(url)
                .send()
                .await
                .map_err(|e| AstrBotError::Platform {
                    adapter: "QQ".to_string(),
                    message: format!("Image download failed: {}", e),
                })?;
        let bytes = response.bytes().await.map_err(|e| AstrBotError::Platform {
            adapter: "QQ".to_string(),
            message: format!("Image download failed: {}", e),
        })?;
        Ok(bytes.to_vec())
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
            run_ws_server(host, port, running, connected, handler, token).await
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
        let reply_chain = build_reply_chain(original, chain);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_onebot_to_chain_image() {
        let seg = OneBotSegment {
            seg_type: "image".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "url".to_string(),
                    serde_json::Value::String("https://example.com/a.png".to_string()),
                );
                m.insert(
                    "file".to_string(),
                    serde_json::Value::String("abc123".to_string()),
                );
                m
            },
        };
        let chain = onebot_to_chain(&[seg]);
        assert_eq!(chain.0.len(), 1);
        if let MessageComponent::Image {
            url,
            file_id,
            base64,
        } = &chain.0[0]
        {
            assert_eq!(url.as_deref(), Some("https://example.com/a.png"));
            assert_eq!(file_id.as_deref(), Some("abc123"));
            assert!(base64.is_none());
        } else {
            panic!("Expected Image component");
        }
    }

    #[test]
    fn test_onebot_to_chain_voice() {
        let seg = OneBotSegment {
            seg_type: "record".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "url".to_string(),
                    serde_json::Value::String("https://example.com/voice.silk".to_string()),
                );
                m
            },
        };
        let chain = onebot_to_chain(&[seg]);
        if let MessageComponent::Voice {
            url,
            file_id,
            base64,
        } = &chain.0[0]
        {
            assert_eq!(url.as_deref(), Some("https://example.com/voice.silk"));
            assert!(file_id.is_none() && base64.is_none());
        } else {
            panic!("Expected Voice component");
        }
    }

    #[test]
    fn test_onebot_to_chain_at() {
        let seg = OneBotSegment {
            seg_type: "at".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "qq".to_string(),
                    serde_json::Value::String("123456".to_string()),
                );
                m.insert(
                    "name".to_string(),
                    serde_json::Value::String("Alice".to_string()),
                );
                m
            },
        };
        let chain = onebot_to_chain(&[seg]);
        if let MessageComponent::At { target, display } = &chain.0[0] {
            assert_eq!(target, "123456");
            assert_eq!(display.as_deref(), Some("Alice"));
        } else {
            panic!("Expected At component");
        }
    }

    #[test]
    fn test_onebot_to_chain_reply() {
        let seg = OneBotSegment {
            seg_type: "reply".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "id".to_string(),
                    serde_json::Value::String("999".to_string()),
                );
                m
            },
        };
        let chain = onebot_to_chain(&[seg]);
        if let MessageComponent::Reply { message_id, chain } = &chain.0[0] {
            assert_eq!(message_id, "999");
            assert!(chain.is_none());
        } else {
            panic!("Expected Reply component");
        }
    }

    #[test]
    fn test_chain_to_onebot_roundtrip() {
        let chain = MessageChain::new()
            .text("Hello")
            .at("123456")
            .image_url("https://example.com/img.png")
            .reply("888", None);
        let segs = chain_to_onebot(&chain);
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0].seg_type, "text");
        assert_eq!(segs[1].seg_type, "at");
        assert_eq!(segs[2].seg_type, "image");
        assert_eq!(segs[3].seg_type, "reply");
    }

    #[test]
    fn test_parse_onebot_request_friend() {
        let event = OneBotRequestEvent {
            post_type: "request".to_string(),
            request_type: "friend".to_string(),
            sub_type: None,
            user_id: 123456,
            group_id: None,
            comment: Some("Hello, add me".to_string()),
            flag: "abc123".to_string(),
        };
        let msg = parse_onebot_request(&event);
        assert_eq!(msg.message_id, "abc123");
        assert_eq!(msg.message_type, MessageType::Private);
        assert_eq!(msg.sender.user_id, "123456");
        let text = msg.chain.plain_text();
        assert!(text.contains("好友请求") && text.contains("Hello, add me"));
        assert!(msg.raw_payload.is_some());
    }

    #[test]
    fn test_parse_onebot_request_group_invite() {
        let event = OneBotRequestEvent {
            post_type: "request".to_string(),
            request_type: "group".to_string(),
            sub_type: Some("invite".to_string()),
            user_id: 789,
            group_id: Some(10086),
            comment: None,
            flag: "flag456".to_string(),
        };
        let msg = parse_onebot_request(&event);
        assert_eq!(msg.message_type, MessageType::Group);
        assert_eq!(msg.session_id, "10086");
        let text = msg.chain.plain_text();
        assert!(text.contains("群邀请") && text.contains("10086"));
    }

    #[test]
    fn test_build_reply_chain_auto_at_group() {
        let original = AstrBotMessage {
            message_id: "100".to_string(),
            timestamp: chrono::Utc::now(),
            platform: PlatformType::Aiocqhttp,
            session_id: "10086".to_string(),
            sender: MessageMember {
                user_id: "123456".to_string(),
                nickname: Some("Alice".to_string()),
                card: None,
                role: None,
                is_self: false,
            },
            message_type: MessageType::Group,
            chain: MessageChain::new().text("hi"),
            raw_payload: None,
        };
        let reply = build_reply_chain(&original, &MessageChain::new().text("yo"));
        assert_eq!(reply.0.len(), 3);
        assert!(
            matches!(reply.0[0], MessageComponent::Reply { .. })
                && matches!(reply.0[1], MessageComponent::At { .. })
                && matches!(reply.0[2], MessageComponent::Plain { .. })
        );
    }

    #[test]
    fn test_build_reply_chain_no_at_private() {
        let original = AstrBotMessage {
            message_id: "101".to_string(),
            timestamp: chrono::Utc::now(),
            platform: PlatformType::Aiocqhttp,
            session_id: "123456".to_string(),
            sender: MessageMember {
                user_id: "123456".to_string(),
                nickname: None,
                card: None,
                role: None,
                is_self: false,
            },
            message_type: MessageType::Private,
            chain: MessageChain::new().text("hi"),
            raw_payload: None,
        };
        let reply = build_reply_chain(&original, &MessageChain::new().text("yo"));
        assert_eq!(reply.0.len(), 2);
    }

    #[test]
    fn test_chain_to_onebot_voice() {
        let mut c = MessageChain::new();
        c.0.push(MessageComponent::Voice {
            url: Some("/tmp/voice.wav".to_string()),
            file_id: Some("fid".to_string()),
            base64: Some("YmFzZTY0".to_string()),
        });
        let segs = chain_to_onebot(&c);
        assert_eq!(segs[0].seg_type, "record");
        assert_eq!(
            segs[0].data.get("url").and_then(|v| v.as_str()),
            Some("/tmp/voice.wav")
        );
        assert_eq!(
            segs[0].data.get("base64").and_then(|v| v.as_str()),
            Some("YmFzZTY0")
        );
    }
}
