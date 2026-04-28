use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;

use crate::token_bucket::{RateLimiter, RateLimitResult};

/// Global rate limiter middleware
/// Returns 429 Too Many Requests with Retry-After header when blocked
pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    // Use a lazy-static global limiter or accept as extension
    // For simplicity, using a static per-IP limiter here
    let limiter = RateLimiter::per_ip();
    let ip = addr.ip().to_string();

    match limiter.check(&ip) {
        RateLimitResult::Allowed => next.run(request).await,
        RateLimitResult::Blocked { retry_after } => {
            let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
            if let Ok(headers) = response.headers_mut().try_entry("retry-after") {
                headers.insert(retry_after.to_string().parse().unwrap());
            }
            response
        }
    }
}

/// Rate limiter that can be configured with custom limits
#[derive(Clone)]
pub struct ConfigurableRateLimiter {
    limiter: RateLimiter,
}

impl ConfigurableRateLimiter {
    pub fn new(capacity: u64, refill_per_sec: f64) -> Self {
        Self {
            limiter: RateLimiter::new(capacity, refill_per_sec),
        }
    }

    pub async fn middleware(
        &self,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        request: Request,
        next: Next,
    ) -> Response {
        let ip = addr.ip().to_string();

        match self.limiter.check(&ip) {
            RateLimitResult::Allowed => next.run(request).await,
            RateLimitResult::Blocked { retry_after } => {
                (StatusCode::TOO_MANY_REQUESTS, [("retry-after", retry_after.to_string())])
                    .into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;

    #[tokio::test]
    async fn test_configurable_rate_limiter() {
        let limiter = ConfigurableRateLimiter::new(5, 1.0);

        // First 5 requests should pass (in theory - depends on middleware chain)
        // We test the limiter directly
        for _ in 0..5 {
            assert_eq!(
                limiter.limiter.check("127.0.0.1"),
                crate::token_bucket::RateLimitResult::Allowed
            );
        }

        let result = limiter.limiter.check("127.0.0.1");
        assert!(matches!(result, crate::token_bucket::RateLimitResult::Blocked { .. }));
    }
}
