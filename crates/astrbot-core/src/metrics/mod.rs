//! Simple in-memory metrics collection for AstrBot
//!
//! Provides `Counter`, `Gauge`, and `Histogram` metric types with
//! JSON snapshot export.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A monotonically increasing counter.
#[derive(Debug)]
pub struct Counter {
    value: AtomicU64,
}

impl Counter {
    pub fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.inc_by(1);
    }

    /// Increment by `delta`.
    pub fn inc_by(&self, delta: u64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
    }

    /// Current counter value.
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// A gauge that can be set to an arbitrary `f64` value.
#[derive(Debug)]
pub struct Gauge {
    // Store f64 bits in an atomic u64 for lock-free reads/writes.
    bits: AtomicU64,
}

impl Gauge {
    pub fn new() -> Self {
        Self {
            bits: AtomicU64::new(0),
        }
    }

    /// Set the gauge value.
    pub fn set(&self, value: f64) {
        let bits = value.to_bits();
        self.bits.store(bits, Ordering::Relaxed);
    }

    /// Current gauge value.
    pub fn value(&self) -> f64 {
        f64::from_bits(self.bits.load(Ordering::Relaxed))
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

/// A histogram that records `f64` observations.
#[derive(Debug)]
pub struct Histogram {
    values: Mutex<Vec<f64>>,
}

impl Histogram {
    pub fn new() -> Self {
        Self {
            values: Mutex::new(Vec::new()),
        }
    }

    /// Record a new observation.
    pub fn record(&self, value: f64) {
        let mut vals = self.values.lock().unwrap();
        vals.push(value);
    }

    /// Calculate a percentile (0.0 ..= 1.0).
    ///
    /// Uses linear interpolation between the two nearest ranks.
    pub fn percentile(&self, p: f64) -> Option<f64> {
        if !(0.0..=1.0).contains(&p) {
            return None;
        }
        let mut vals = self.values.lock().unwrap();
        if vals.is_empty() {
            return None;
        }
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let len = vals.len() as f64;
        let rank = p * (len - 1.0);
        let lower = rank.floor() as usize;
        let upper = rank.ceil() as usize;
        let frac = rank - rank.floor();

        let lower_val = vals[lower.min(vals.len() - 1)];
        let upper_val = vals[upper.min(vals.len() - 1)];
        Some(lower_val + (upper_val - lower_val) * frac)
    }

    /// Number of recorded values.
    pub fn count(&self) -> usize {
        self.values.lock().unwrap().len()
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory metrics collector.
#[derive(Debug, Default)]
pub struct MetricsCollector {
    counters: HashMap<String, Counter>,
    gauges: HashMap<String, Gauge>,
    histograms: HashMap<String, Histogram>,
}

impl MetricsCollector {
    /// Create an empty collector.
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            gauges: HashMap::new(),
            histograms: HashMap::new(),
        }
    }

    /// Get or create a counter by name.
    pub fn counter(&mut self, name: &str) -> &mut Counter {
        self.counters
            .entry(name.to_string())
            .or_insert_with(Counter::new)
    }

    /// Get or create a gauge by name.
    pub fn gauge(&mut self, name: &str) -> &mut Gauge {
        self.gauges
            .entry(name.to_string())
            .or_insert_with(Gauge::new)
    }

    /// Get or create a histogram by name.
    pub fn histogram(&mut self, name: &str) -> &mut Histogram {
        self.histograms
            .entry(name.to_string())
            .or_insert_with(Histogram::new)
    }

    /// Export all metrics as a JSON value.
    pub fn snapshot(&self) -> Value {
        let mut counters = Map::new();
        for (name, c) in &self.counters {
            counters.insert(name.clone(), json!(c.value()));
        }

        let mut gauges = Map::new();
        for (name, g) in &self.gauges {
            gauges.insert(name.clone(), json!(g.value()));
        }

        let mut histograms = Map::new();
        for (name, h) in &self.histograms {
            let mut obj = Map::new();
            obj.insert("count".to_string(), json!(h.count()));
            for p in [0.50, 0.90, 0.95, 0.99] {
                if let Some(v) = h.percentile(p) {
                    let label = format!("p{}", (p * 100.0) as u32);
                    obj.insert(label, json!(v));
                }
            }
            histograms.insert(name.clone(), Value::Object(obj));
        }

        json!({
            "counters": counters,
            "gauges": gauges,
            "histograms": histograms,
        })
    }

    /// Increment message count for a platform.
    pub fn increment_message_count(&mut self, platform: &str) {
        let platform_key = format!("messages.{}", platform);
        self.counter(&platform_key).inc();
        self.counter("messages.total").inc();
    }

    /// Increment provider call count.
    pub fn increment_provider_call(&mut self, provider_id: &str, success: bool) {
        let outcome = if success { "success" } else { "failure" };
        let outcome_key = format!("provider.{}.{}", provider_id, outcome);
        let total_key = format!("provider.{}.total", provider_id);
        self.counter(&outcome_key).inc();
        self.counter(&total_key).inc();
    }

    /// Alias for snapshot.
    pub fn get_stats(&self) -> Value {
        self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter() {
        let mut m = MetricsCollector::new();
        m.counter("req").inc();
        m.counter("req").inc_by(4);
        assert_eq!(m.counter("req").value(), 5);
    }

    #[test]
    fn test_gauge() {
        let mut m = MetricsCollector::new();
        m.gauge("temp").set(36.5);
        assert_eq!(m.gauge("temp").value(), 36.5);
        m.gauge("temp").set(37.2);
        assert_eq!(m.gauge("temp").value(), 37.2);
    }

    #[test]
    fn test_histogram_and_snapshot() {
        let mut m = MetricsCollector::new();
        m.histogram("latency").record(10.0);
        m.histogram("latency").record(20.0);
        m.histogram("latency").record(30.0);
        m.histogram("latency").record(40.0);
        m.histogram("latency").record(50.0);

        let h = m.histogram("latency");
        assert_eq!(h.count(), 5);
        assert_eq!(h.percentile(0.0), Some(10.0)); // min
        assert_eq!(h.percentile(1.0), Some(50.0)); // max
        let p50 = h.percentile(0.5).unwrap();
        assert!((p50 - 30.0).abs() < 0.1, "p50 should be ~30, got {}", p50);

        let snap = m.snapshot();
        let latency = snap.get("histograms").unwrap().get("latency").unwrap();
        assert_eq!(latency.get("count").unwrap().as_u64(), Some(5));
        assert!(latency.get("p50").unwrap().as_f64().is_some());
    }

    #[test]
    fn test_increment_message_count() {
        let mut m = MetricsCollector::new();
        m.increment_message_count("qq");
        m.increment_message_count("qq");
        m.increment_message_count("telegram");
        assert_eq!(m.counter("messages.qq").value(), 2);
        assert_eq!(m.counter("messages.telegram").value(), 1);
        assert_eq!(m.counter("messages.total").value(), 3);
    }

    #[test]
    fn test_increment_provider_call() {
        let mut m = MetricsCollector::new();
        m.increment_provider_call("openai", true);
        m.increment_provider_call("openai", true);
        m.increment_provider_call("openai", false);
        m.increment_provider_call("anthropic", true);
        assert_eq!(m.counter("provider.openai.success").value(), 2);
        assert_eq!(m.counter("provider.openai.failure").value(), 1);
        assert_eq!(m.counter("provider.openai.total").value(), 3);
        assert_eq!(m.counter("provider.anthropic.success").value(), 1);
        assert_eq!(m.counter("provider.anthropic.total").value(), 1);
    }
}

pub mod aggregator;
pub use aggregator::{AggregatedStats, PlatformStat, ProviderStat, StatsAggregator};
