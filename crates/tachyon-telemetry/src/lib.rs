//! Route telemetry: measurements, EWMA estimates, route records.
//!
//! Local aggregation only. Estimates predict cheap-operation latency so the
//! router can decide whether evidence fits inside the grace window; route
//! records make every classification auditable. Optional exporters arrive
//! with a later milestone.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::VecDeque;

/// One measurement: named value with a wall-clock timestamp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub name: String,
    pub value_ms: f64,
    pub at_unix_ms: u64,
}

/// Bounded in-memory recorder (ring per name).
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    series: HashMap<String, VecDeque<Measurement>>,
    capacity: usize,
}

impl Recorder {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            series: HashMap::new(),
            capacity: capacity.max(1),
        }
    }

    /// Records a measurement with the current wall time.
    pub fn record(&mut self, name: &str, value_ms: f64) {
        let entry = Measurement {
            name: name.to_owned(),
            value_ms,
            at_unix_ms: unix_ms(),
        };
        let queue = self.series.entry(name.to_owned()).or_default();
        queue.push_back(entry);
        while queue.len() > self.capacity {
            queue.pop_front();
        }
    }

    /// Latest value for `name`, if any.
    #[must_use]
    pub fn latest(&self, name: &str) -> Option<f64> {
        self.series.get(name)?.back().map(|entry| entry.value_ms)
    }

    /// Mean of recorded values for `name`, if any.
    #[must_use]
    pub fn mean(&self, name: &str) -> Option<f64> {
        let queue = self.series.get(name)?;
        if queue.is_empty() {
            return None;
        }
        let count = f64::from(u32::try_from(queue.len()).unwrap_or(u32::MAX));
        Some(queue.iter().map(|entry| entry.value_ms).sum::<f64>() / count)
    }

    /// Snapshots all series (for export/debugging).
    #[must_use]
    pub fn snapshot(&self) -> HashMap<String, Vec<Measurement>> {
        self.series
            .iter()
            .map(|(name, queue)| (name.clone(), queue.iter().cloned().collect()))
            .collect()
    }
}

/// Exponentially-weighted moving average per key.
#[derive(Clone, Debug)]
pub struct Ewma {
    alpha: f64,
    default_ms: f64,
    estimates: HashMap<String, f64>,
}

impl Ewma {
    #[must_use]
    pub fn new(alpha: f64, default_ms: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.0, 1.0),
            default_ms,
            estimates: HashMap::new(),
        }
    }

    /// Current estimate for `key`, or the default when unobserved.
    #[must_use]
    pub fn estimate(&self, key: &str) -> f64 {
        self.estimates.get(key).copied().unwrap_or(self.default_ms)
    }

    /// Folds one sample into the estimate.
    pub fn observe(&mut self, key: &str, sample_ms: f64) {
        let current = self.estimate(key);
        self.estimates
            .insert(key.to_owned(), current + self.alpha * (sample_ms - current));
    }
}

/// One routing decision, recorded for audit and estimate training.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteRecord {
    pub class: String,
    pub confidence: f64,
    pub rules_fired: Vec<String>,
    pub evidence_ops: usize,
    pub model_calls_planned: u32,
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ewma_converges_toward_samples() {
        let mut ewma = Ewma::new(0.5, 100.0);
        assert!((ewma.estimate("op") - 100.0).abs() < f64::EPSILON);
        ewma.observe("op", 0.0);
        assert!((ewma.estimate("op") - 50.0).abs() < f64::EPSILON);
        ewma.observe("op", 0.0);
        assert!((ewma.estimate("op") - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn recorder_bounds_and_means() {
        let mut recorder = Recorder::new(3);
        for value in [10.0, 20.0, 30.0, 40.0] {
            recorder.record("latency", value);
        }
        assert_eq!(recorder.latest("latency"), Some(40.0));
        assert_eq!(recorder.mean("latency"), Some(30.0));
        assert_eq!(recorder.latest("missing"), None);
    }
}
