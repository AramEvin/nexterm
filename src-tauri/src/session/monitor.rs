//! Live resource readings for the bar under a terminal (Phase 6).
//!
//! Protocol-neutral: SSH fills it from `/proc` over a separate exec channel
//! (`ssh::monitor`); a local terminal could fill it from the OS later.

use serde::Serialize;

/// One reading or a state change, sent to the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResourceEvent {
    #[serde(rename_all = "camelCase")]
    Sample(ResourceSample),
    /// The server can't be read this way (not Linux, no `/bin/sh`, exec refused).
    Unsupported { reason: String },
    /// Readings stopped (channel closed, connection lost).
    Stopped { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSample {
    /// Busy CPU in percent across all cores. `None` on the first reading
    /// (CPU usage needs two readings to compare).
    pub cpu_percent: Option<f32>,
    pub cores: u32,
    pub mem_used_kib: u64,
    pub mem_total_kib: u64,
    pub swap_used_kib: u64,
    pub swap_total_kib: u64,
    /// Root file system (`/`); `None` if `df` gave nothing usable.
    pub disk_used_kib: Option<u64>,
    pub disk_total_kib: Option<u64>,
    /// 1, 5 and 15 minute load averages.
    pub load: [f32; 3],
    pub uptime_secs: u64,
}

/// Where a monitor delivers its events.
pub trait MonitorSink: Send + Sync + 'static {
    /// Returns `false` once the receiver is gone (the monitor then stops).
    fn event(&self, event: ResourceEvent) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_kind_tag() {
        let json = serde_json::to_value(ResourceEvent::Unsupported { reason: "x".into() }).unwrap();
        assert_eq!(json["kind"], "unsupported");
        let sample = ResourceEvent::Sample(ResourceSample {
            cpu_percent: Some(12.5),
            cores: 4,
            mem_used_kib: 1,
            mem_total_kib: 2,
            swap_used_kib: 0,
            swap_total_kib: 0,
            disk_used_kib: None,
            disk_total_kib: None,
            load: [0.1, 0.2, 0.3],
            uptime_secs: 5,
        });
        let json = serde_json::to_value(sample).unwrap();
        assert_eq!(json["kind"], "sample");
        assert_eq!(json["cpuPercent"], 12.5);
        assert_eq!(json["memTotalKib"], 2);
    }
}
