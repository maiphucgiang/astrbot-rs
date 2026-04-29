use astrbot_security::auth::token::{
    generate_access_token, generate_jti, security_headers, verify_token,
};
use astrbot_security::executor::{HardenedLocalExecutor, SafeExecutionResult};
use astrbot_security::file::upload::SafeFileStorage;
use astrbot_security::net::ssrf_guard::validate_url;
use astrbot_security::plugin::capability::{
    check_capability, check_install_permission, Capability, PluginManifest, RiskLevel,
};
use astrbot_security::webhook::security::WebhookSecurity;

/// 端到端安全集成验证
#[tokio::test]
async fn test_end_to_end_security_stack() {
    // 1. 硬化代码执行
    let executor = HardenedLocalExecutor::default();
    let result = executor
        .execute("print('hello from sandbox')")
        .await
        .unwrap();
    assert!(result.stdout.contains("hello from sandbox"));

    // 2. 恶意代码被 AST 拦截
    let bad = executor.execute("import os; os.system('id')").await;
    assert!(bad.is_err());

    // 3. Webhook 防 replay
    let secret = b"shared-secret";
    let payload = b"{}";
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let nonce = "end2end-nonce-999";

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(payload);
    let sig = hex::encode(mac.finalize().into_bytes());

    assert!(WebhookSecurity::verify(secret, payload, &sig, &ts, nonce).is_ok());
    let replay = WebhookSecurity::verify(secret, payload, &sig, &ts, nonce);
    assert!(replay.is_err());

    // 4. 文件上传安全
    let dir = std::env::temp_dir().join("astrbot_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let storage = SafeFileStorage {
        base_dir: dir.clone(),
    };
    let id = storage.save("test.txt", b"safe content").unwrap();
    assert!(!id.is_empty());
    let bad_ext = storage.save("evil.exe", b"payload");
    assert!(bad_ext.is_err());

    // 5. SSRF 防护
    assert!(validate_url("https://public-api.com/data").is_ok());
    assert!(validate_url("http://192.168.1.1/internal").is_err());
    assert!(validate_url("http://localhost:8080/admin").is_err());

    // 6. JWT 签发与校验
    let jwt_secret = b"jwt-secret-must-be-32-bytes-long!!!";
    let jti = generate_jti();
    let token = generate_access_token("admin", &jti, "fp-abc", jwt_secret).unwrap();
    let decoded = verify_token(&token, jwt_secret).unwrap();
    assert_eq!(decoded.claims.sub, "admin");

    // 7. Plugin 权限模型
    let manifest = PluginManifest {
        id: "test.plugin".to_string(),
        name: "Test".to_string(),
        version: "1.0.0".to_string(),
        author: "test".to_string(),
        checksum: "sha256-abc".to_string(),
        capabilities: vec![Capability::ReadMessages, Capability::ExecuteCode],
        description: "Test plugin".to_string(),
    };
    assert_eq!(manifest.risk_level(), RiskLevel::Critical);
    assert!(check_install_permission(&manifest, false).is_err());
    assert!(check_install_permission(&manifest, true).is_ok());
    assert!(check_capability(&manifest, &Capability::ReadMessages).is_ok());
    assert!(check_capability(&manifest, &Capability::SendMessages).is_err());

    // 8. 安全响应头
    let headers = security_headers();
    assert!(headers.iter().any(|(k, _)| *k == "Content-Security-Policy"));
    assert!(headers.iter().any(|(k, _)| *k == "X-Frame-Options"));
}
