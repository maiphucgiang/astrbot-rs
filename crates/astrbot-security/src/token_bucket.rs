use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Token bucket for rate limiting
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: u64,
    tokens: f64,
    last_update: Instant,
    refill_rate: f64, // tokens per second
}

impl TokenBucket {
    pub fn new(capacity: u64, refill_per_sec: f64) -> Self {
        Self {
            capacity,
            tokens: capacity as f64,
            last_update: Instant::now(),
            refill_rate: refill_per_sec,
        }
    }

    /// Try to consume `amount` tokens. Returns true if allowed.
    pub fn try_consume(&mut self, amount: u64) -> bool {
        self.refill();
        if self.tokens >= amount as f64 {
            self.tokens -= amount as f64;
            true
        } else {
            false
        }
    }

    /// How many seconds until enough tokens for `amount`
    pub fn wait_time(&self, amount: u64) -> Duration {
        let needed = amount as f64 - self.tokens;
        if needed <= 0.0 {
            Duration::from_secs(0)
        } else {
            Duration::from_secs_f64(needed / self.refill_rate)
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity as f64);
        self.last_update = now;
    }
}

/// Per-key rate limiter (e.g., per IP, per user, per API key)
#[derive(Debug, Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
    default_capacity: u64,
    default_refill: f64,
}

impl RateLimiter {
    pub fn new(capacity: u64, refill_per_sec: f64) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            default_capacity: capacity,
            default_refill: refill_per_sec,
        }
    }

    /// Create global rate limiter: 100 requests / 10 seconds
    pub fn global() -> Self {
        Self::new(100, 10.0)
    }

    /// Create per-IP rate limiter: 30 requests / 10 seconds
    pub fn per_ip() -> Self {
        Self::new(30, 3.0)
    }

    /// Check if request from `key` is allowed
    pub fn check(&self, key: &str) -> RateLimitResult {
        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(self.default_capacity, self.default_refill));

        if bucket.try_consume(1) {
            RateLimitResult::Allowed
        } else {
            let wait = bucket.wait_time(1);
            RateLimitResult::Blocked {
                retry_after: wait.as_secs() as u32,
            }
        }
    }

    /// Reset bucket for a key (e.g., after ban expiry)
    pub fn reset(&self, key: &str) {
        let mut buckets = self.buckets.lock().unwrap();
        buckets.remove(key);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RateLimitResult {
    Allowed,
    Blocked { retry_after: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_token_bucket_basic() {
        let mut bucket = TokenBucket::new(10, 1.0);
        assert!(bucket.try_consume(5));
        assert!(bucket.try_consume(5));
        assert!(!bucket.try_consume(1)); // empty
    }

    #[test]
    fn test_token_bucket_refill() {
        let mut bucket = TokenBucket::new(2, 10.0); // 2 tokens, refill 10/sec
        assert!(bucket.try_consume(2));
        assert!(!bucket.try_consume(1));

        thread::sleep(Duration::from_millis(200)); // wait for refill
        assert!(bucket.try_consume(1)); // refilled
    }

    #[test]
    fn test_rate_limiter_global() {
        let limiter = RateLimiter::global();
        let key = "test_key";

        // First 100 should pass
        for _ in 0..100 {
            assert_eq!(limiter.check(key), RateLimitResult::Allowed);
        }

        // 101th should be blocked
        let result = limiter.check(key);
        assert!(matches!(result, RateLimitResult::Blocked { .. }));
    }

    #[test]
    fn test_rate_limiter_per_ip() {
        let limiter = RateLimiter::per_ip();

        assert_eq!(limiter.check("1.2.3.4"), RateLimitResult::Allowed);
        assert_eq!(limiter.check("5.6.7.8"), RateLimitResult::Allowed);

        // Different keys are independent
        for _ in 0..29 {
            limiter.check("1.2.3.4");
        }
        let result = limiter.check("1.2.3.4");
        assert!(matches!(result, RateLimitResult::Blocked { .. }));

        // Other IP still allowed
        assert_eq!(limiter.check("5.6.7.8"), RateLimitResult::Allowed);
    }

    #[test]
    fn test_rate_limiter_reset() {
        let limiter = RateLimiter::global();
        let key = "reset_me";

        for _ in 0..100 {
            limiter.check(key);
        }
        assert!(matches!(limiter.check(key), RateLimitResult::Blocked { .. }));

        limiter.reset(key);
        assert_eq!(limiter.check(key), RateLimitResult::Allowed);
    }
}
