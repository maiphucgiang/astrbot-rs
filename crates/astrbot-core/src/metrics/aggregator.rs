//! Stats aggregator — computes derived metrics from raw MetricsCollector data.

use super::MetricsCollector;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Aggregated dashboard statistics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedStats {
    pub messages_per_hour: f64,
    pub top_platforms: Vec<PlatformStat>,
    pub provider_success_rate: Vec<ProviderStat>,
}

/// Message count for a single platform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlatformStat {
    pub platform: String,
    pub count: u64,
}

/// Success rate for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderStat {
    pub provider: String,
    pub success_rate: f64,
    pub success: u64,
    pub total: u64,
}

/// Aggregates raw counter data from [`MetricsCollector`] into human-readable stats.
pub struct StatsAggregator;

impl StatsAggregator {
    pub fn aggregate(collector: &mut MetricsCollector, uptime_seconds: u64) -> AggregatedStats {
        let total_messages = collector.counter("messages.total").value();
        let hours = (uptime_seconds as f64 / 3600.0).max(1.0);
        let messages_per_hour = total_messages as f64 / hours;

        let mut platforms: Vec<PlatformStat> = Vec::new();
        let snapshot = collector.snapshot();
        if let Some(counters) = snapshot.get("counters").and_then(|v| v.as_object()) {
            for (key, val) in counters {
                if let Some(platform) = key.strip_prefix("messages.") {
                    if platform == "total" { continue; }
                    if let Some(count) = val.as_u64() {
                        platforms.push(PlatformStat { platform: platform.to_string(), count });
                    }
                }
            }
        }
        platforms.sort_by(|a, b| b.count.cmp(&a.count));

        let mut provider_stats: Vec<ProviderStat> = Vec::new();
        let mut provider_totals: HashMap<String, u64> = HashMap::new();
        let mut provider_successes: HashMap<String, u64> = HashMap::new();

        if let Some(counters) = snapshot.get("counters").and_then(|v| v.as_object()) {
            for (key, val) in counters {
                if let Some(rest) = key.strip_prefix("provider.") {
                    let parts: Vec<&str> = rest.split('.').collect();
                    if parts.len() == 2 {
                        let provider_id = parts[0].to_string();
                        let metric_type = parts[1];
                        if let Some(count) = val.as_u64() {
                            match metric_type {
                                "total" => { provider_totals.insert(provider_id.clone(), count); }
                                "success" => { provider_successes.insert(provider_id.clone(), count); }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        for (provider, total) in provider_totals {
            let success = provider_successes.get(&provider).copied().unwrap_or(0);
            let success_rate = if total > 0 { (success as f64 / total as f64) * 100.0 } else { 0.0 };
            provider_stats.push(ProviderStat { provider, success_rate, success, total });
        }
        provider_stats.sort_by(|a, b| b.total.cmp(&a.total));

        AggregatedStats { messages_per_hour, top_platforms: platforms, provider_success_rate: provider_stats }
    }

    pub fn aggregate_json(collector: &mut MetricsCollector, uptime_seconds: u64) -> Value {
        let stats = Self::aggregate(collector, uptime_seconds);
        json!({
            "messages_per_hour": stats.messages_per_hour,
            "top_platforms": stats.top_platforms,
            "provider_success_rate": stats.provider_success_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_basic() {
        let mut collector = MetricsCollector::new();
        collector.increment_message_count("qq");
        collector.increment_message_count("qq");
        collector.increment_message_count("qq");
        collector.increment_message_count("telegram");
        collector.increment_message_count("telegram");
        collector.increment_message_count("wechat");
        collector.increment_provider_call("openai", true);
        collector.increment_provider_call("openai", true);
        collector.increment_provider_call("openai", false);
        collector.increment_provider_call("anthropic", true);
        let stats = StatsAggregator::aggregate(&mut collector, 3600);
        assert!((stats.messages_per_hour - 6.0).abs() < 0.01);
        assert_eq!(stats.top_platforms.len(), 3);
        assert_eq!(stats.top_platforms[0].platform, "qq");
        assert_eq!(stats.top_platforms[0].count, 3);
        assert_eq!(stats.provider_success_rate.len(), 2);
        let openai = stats.provider_success_rate.iter().find(|p| p.provider == "openai").unwrap();
        assert_eq!(openai.total, 3);
        assert_eq!(openai.success, 2);
        assert!((openai.success_rate - 66.6667).abs() < 0.1);
    }

    #[test]
    fn test_aggregate_json() {
        let mut collector = MetricsCollector::new();
        collector.increment_message_count("qq");
        let val = StatsAggregator::aggregate_json(&mut collector, 3600);
        assert_eq!(val["messages_per_hour"].as_f64(), Some(1.0));
    }

    #[test]
    fn test_aggregate_zero_uptime_clamped() {
        let mut collector = MetricsCollector::new();
        collector.increment_message_count("qq");
        let stats = StatsAggregator::aggregate(&mut collector, 0);
        assert_eq!(stats.messages_per_hour, 1.0);
    }
}
