use astrbot_feishu::{
    auth::FeishuAuth,
    platform::{FeishuAdapter, FeishuAdapterConfig},
    models::AppCredentials,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let creds = AppCredentials {
        app_id: std::env::var("FEISHU_APP_ID").unwrap_or("cli_xxx".into()),
        app_secret: std::env::var("FEISHU_APP_SECRET").unwrap_or("sec_xxx".into()),
        encrypt_key: std::env::var("FEISHU_ENCRYPT_KEY").ok(),
        verification_token: std::env::var("FEISHU_VERIFICATION_TOKEN").ok(),
    };

    let auth = FeishuAuth::new(creds);
    let config = FeishuAdapterConfig::default();
    let adapter = FeishuAdapter::new(auth, config);

    let chat_id = std::env::var("FEISHU_TEST_CHAT_ID").unwrap_or("oc_xxx".into());
    let msg_id = adapter.send_text(&chat_id, "AstrBot Feishu adapter online 🚀").await?;
    println!("Sent message: {}", msg_id);

    Ok(())
}
