use chrono::{DateTime, Utc};

/// Format a timestamp as a human-readable string
pub fn format_timestamp(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// Get current timestamp as milliseconds since epoch
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
