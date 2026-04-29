use astrbot_feishu::*;
use chrono::Utc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup_mock_auth(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "msg": "ok",
            "data": {
                "tenant_access_token": "test_token_123",
                "expire": 7200
            }
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_tenant_token_fetch() {
    let server = MockServer::start().await;
    setup_mock_auth(&server).await;

    let creds = AppCredentials {
        app_id: "cli_test".into(),
        app_secret: "sec_test".into(),
        encrypt_key: None,
        verification_token: None,
    };

    let auth = auth::FeishuAuth::new(creds).with_base_url(server.uri());

    let token = auth.tenant_access_token().await.unwrap();
    assert_eq!(token, "test_token_123");
}

#[tokio::test]
async fn test_send_text_message() {
    let server = MockServer::start().await;
    setup_mock_auth(&server).await;

    Mock::given(method("POST"))
        .and(path("/im/v1/messages"))
        .and(header("Authorization", "Bearer test_token_123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "msg": "ok",
            "data": {
                "message_id": "om_test_456"
            }
        })))
        .mount(&server)
        .await;

    let creds = AppCredentials {
        app_id: "cli_test".into(),
        app_secret: "sec_test".into(),
        encrypt_key: None,
        verification_token: None,
    };

    let auth = auth::FeishuAuth::new(creds).with_base_url(server.uri());
    let adapter = platform::FeishuAdapter::new(auth, platform::FeishuAdapterConfig::default());

    let msg_id = adapter.send_text("oc_test", "Hello Feishu").await.unwrap();
    assert_eq!(msg_id, "om_test_456");
}

#[tokio::test]
async fn test_calendar_list_events() {
    let server = MockServer::start().await;
    setup_mock_auth(&server).await;

    Mock::given(method("GET"))
        .and(path("/calendar/v4/calendars/primary/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "msg": "ok",
            "data": {
                "items": [
                    {
                        "event_id": "evt_1",
                        "summary": "Test Meeting",
                        "start_time": "2024-01-01T10:00:00Z",
                        "end_time": "2024-01-01T11:00:00Z",
                        "attendees": []
                    }
                ],
                "has_more": false
            }
        })))
        .mount(&server)
        .await;

    let creds = AppCredentials {
        app_id: "cli_test".into(),
        app_secret: "sec_test".into(),
        encrypt_key: None,
        verification_token: None,
    };

    let auth = auth::FeishuAuth::new(creds).with_base_url(server.uri());
    let cal = calendar::CalendarClient::new(auth);

    let filter = calendar::EventFilter {
        start_time: Some(Utc::now()),
        end_time: Some(Utc::now() + chrono::Duration::days(7)),
        calendar_id: Some("primary".into()),
    };

    let events = cal.list_events("primary", &filter).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "Test Meeting");
}

#[tokio::test]
async fn test_bitable_list_records() {
    let server = MockServer::start().await;
    setup_mock_auth(&server).await;

    Mock::given(method("GET"))
        .and(path("/bitable/v1/apps/app_test/tables/tbl_test/records"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "msg": "ok",
            "data": {
                "items": [
                    {
                        "record_id": "rec_1",
                        "fields": { "Name": "Alice", "Score": 100 }
                    },
                    {
                        "record_id": "rec_2",
                        "fields": { "Name": "Bob", "Score": 90 }
                    }
                ],
                "has_more": false,
                "total": 2
            }
        })))
        .mount(&server)
        .await;

    let creds = AppCredentials {
        app_id: "cli_test".into(),
        app_secret: "sec_test".into(),
        encrypt_key: None,
        verification_token: None,
    };

    let auth = auth::FeishuAuth::new(creds).with_base_url(server.uri());
    let bitable = knowledge::BitableClient::new(auth);

    let result = bitable
        .list_records("app_test", "tbl_test", None, 500)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 2);
    assert_eq!(result.items[0].record_id, "rec_1");
}
