//! Preparation budgets (spec §6.8). Budget rejection is a normal outcome and
//! SHALL be visible in logs and UI (§6.8).

use serde::{Deserialize, Serialize};

/// Battery/thermal restriction on preparation work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThermalLimit {
    Unrestricted,
    Restricted,
    Blocked,
}

/// Explicit budgets a preparation policy must respect (spec §6.8).
/// Not `Eq` because `cpu_duty` is `f32`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub total_net_bytes: u64,
    pub avg_net_rate: u64,
    pub burst_bytes: u64,
    pub total_cpu_ms: u64,
    pub cpu_duty: f32,
    pub peak_prep_mem: u64,
    pub storage_quota: u64,
    pub battery_thermal: ThermalLimit,
    pub elapsed_opportunity_ms: u64,
    pub max_nonpreemptible_ms: u64,
}

impl Budget {
    /// A generous default suitable for local Phase-1 preparation.
    pub fn local_default() -> Self {
        Budget {
            total_net_bytes: u64::MAX,
            avg_net_rate: u64::MAX,
            burst_bytes: u64::MAX,
            total_cpu_ms: 60_000,
            cpu_duty: 1.0,
            peak_prep_mem: 512 * 1024 * 1024,
            storage_quota: u64::MAX,
            battery_thermal: ThermalLimit::Unrestricted,
            elapsed_opportunity_ms: 60_000,
            max_nonpreemptible_ms: 5_000,
        }
    }

    /// A conservative background budget derived from `active`: foreground-first
    /// means a large import is admitted only in the foreground (§20.2). Each limit
    /// is the smaller of the active value and a tight background cap; thermal is at
    /// least `Restricted`. Mobile background work is bounded and not guaranteed
    /// (§11.2).
    pub fn background_floor(active: &Budget) -> Self {
        /// Tight background network cap (256 KiB): small refreshes only.
        const BG_NET_BYTES: u64 = 256 * 1024;
        const BG_CPU_MS: u64 = 1_000;
        const BG_PEAK_MEM: u64 = 32 * 1024 * 1024;
        const BG_NONPREEMPT_MS: u64 = 500;
        Budget {
            total_net_bytes: active.total_net_bytes.min(BG_NET_BYTES),
            avg_net_rate: active.avg_net_rate.min(BG_NET_BYTES),
            burst_bytes: active.burst_bytes.min(BG_NET_BYTES),
            total_cpu_ms: active.total_cpu_ms.min(BG_CPU_MS),
            cpu_duty: active.cpu_duty.min(0.25),
            peak_prep_mem: active.peak_prep_mem.min(BG_PEAK_MEM),
            storage_quota: active.storage_quota,
            battery_thermal: match active.battery_thermal {
                ThermalLimit::Unrestricted => ThermalLimit::Restricted,
                other => other,
            },
            elapsed_opportunity_ms: active.elapsed_opportunity_ms.min(BG_CPU_MS),
            max_nonpreemptible_ms: active.max_nonpreemptible_ms.min(BG_NONPREEMPT_MS),
        }
    }
}
