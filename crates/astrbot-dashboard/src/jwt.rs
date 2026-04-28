use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
    Json,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::routes::AppState;

/// JWT claims
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // subject (user id)
    pub exp: i64,    // expiration timestamp
    pub iat: i64,    // issued at
}

/// JWT auth middleware — verifies Bearer token
pub async fn jwt_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Skip auth for health and login endpoints
    let path = req.uri().path();
    if path == "/api/health" || path == "/api/login" {
        return Ok(next.run(req).await);
    }

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    // If no JWT secret configured, skip auth (test mode / unprotected)
    if state.jwt_secret.is_none() {
        return Ok(next.run(req).await);
    }

    let token = match auth_header {
        Some(t) => t,
        None => {
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::from(r#"{"error":"Missing token"}"#))
                .unwrap());
        }
    };

    let secret = state.jwt_secret.as_deref().unwrap_or("astrbot-default-secret");
    match decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    ) {
        Ok(_) => Ok(next.run(req).await),
        Err(_) => Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from(r#"{"error":"Invalid token"}"#))
            .unwrap()),
    }
}

/// Login handler — generates JWT token
pub async fn login_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let password = payload.get("password").and_then(|v| v.as_str());
    let admin_password = state.admin_password.as_deref().unwrap_or("astrbot");

    if password != Some(admin_password) {
        return Json(json!({
            "success": false,
            "error": "Invalid password"
        }));
    }

    let now = Utc::now();
    let exp = now + Duration::hours(24);
    let claims = Claims {
        sub: "admin".to_string(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
    };

    let secret = state.jwt_secret.as_deref().unwrap_or("astrbot-default-secret");
    match encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    ) {
        Ok(token) => Json(json!({
            "success": true,
            "token": token,
            "expires_in": 86400
        })),
        Err(_) => Json(json!({
            "success": false,
            "error": "Token generation failed"
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_serialize() {
        let claims = Claims {
            sub: "admin".to_string(),
            exp: 1234567890,
            iat: 1234567800,
        };
        let json = serde_json::to_string(&claims).unwrap();
        assert!(json.contains("admin"));
    }
}
