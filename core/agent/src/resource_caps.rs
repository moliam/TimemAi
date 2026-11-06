//! Instance resource cap configuration.
//!
//! The caps bound how much of the machine's total CPU capacity and total
//! memory the current Timem instance's managed job tree may consume. They
//! are percentages of system totals (default 85) and are applied once at
//! host startup through the platform backend; the kernel enforces the bound,
//! so a runaway job tree can no longer starve the whole machine. Each
//! dimension carries an independent enable flag: a disabled dimension is
//! written back to the kernel as unbounded, so the switch reflects the
//! actual kernel state.
//!
//! Environment surface:
//! - `TIMEM_SYSTEM_CPU_CAP_PERCENT`
//! - `TIMEM_SYSTEM_MEM_CAP_PERCENT`
//!
//! The env surface only seeds percentages (enable flags default to on);
//! invalid values warn on stderr (in the `[config_error]` style) and fall
//! back to the default so a typo can never silently remove the kernel-side
//! bound.

use crate::os::{self, InstanceResourceCaps, ResourceCapsApplication};
use std::collections::HashMap;

// Platform caps machinery, forwarded so applications stay inside the Core
// capability domain instead of reaching into the platform alias directly.
pub use crate::os::{
    apply_resource_cap_escalation_plan, instance_resource_caps_application,
    probe_instance_resource_caps, JobResourceUsage, ResourceCapEscalationAction,
    ResourceCapReadiness, ResourceCapStatus, ResourceCapsReadiness, SessionResourceUsage,
    SystemResourceUsage, SHARED_INSTANCE_ROOT_REASON,
};

/// Read-only resource snapshot of one Session's managed job scope. Returns
/// `Ok(None)` when the platform has no accounting scope for that Session.
pub fn session_resource_usage(session_id: &str) -> std::io::Result<Option<SessionResourceUsage>> {
    os::session_resource_usage(session_id)
}

/// Logical CPUs online, when the platform can report them.
pub fn system_cpu_cores() -> Option<u32> {
    os::system_cpu_cores()
}

/// Total system memory in bytes, when the platform can report it.
pub fn system_mem_total_bytes() -> Option<u64> {
    os::system_mem_total_bytes()
}

/// Machine-wide CPU/memory counters for the Session Insights dashboard.
pub fn system_resource_usage() -> std::io::Result<Option<SystemResourceUsage>> {
    os::system_resource_usage()
}

pub const DEFAULT_SYSTEM_CPU_CAP_PERCENT: f64 = 85.0;
pub const DEFAULT_SYSTEM_MEM_CAP_PERCENT: f64 = 85.0;
pub const SYSTEM_CPU_CAP_ENV: &str = "TIMEM_SYSTEM_CPU_CAP_PERCENT";
pub const SYSTEM_MEM_CAP_ENV: &str = "TIMEM_SYSTEM_MEM_CAP_PERCENT";

/// Resolved cap configuration for this instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemResourceCapConfig {
    pub cpu_enabled: bool,
    pub cpu_percent: f64,
    pub mem_enabled: bool,
    pub mem_percent: f64,
}

impl SystemResourceCapConfig {
    /// Validate explicitly submitted values (WebUI system settings). Unlike
    /// the env fallback path, an invalid explicit request is rejected so a
    /// typo can never silently change what the kernel enforces. Percentages
    /// stay validated even while a dimension is disabled: the stored value
    /// is the preset the switch flips back to.
    pub fn validated(
        cpu_enabled: bool,
        cpu_percent: f64,
        mem_enabled: bool,
        mem_percent: f64,
    ) -> Result<Self, String> {
        fn check(value: f64, name: &str) -> Result<(), String> {
            if !value.is_finite() || value <= 0.0 || value > 100.0 {
                return Err(format!("{name} cap {value} is outside (0, 100]"));
            }
            Ok(())
        }
        check(cpu_percent, "cpu")?;
        check(mem_percent, "mem")?;
        Ok(Self {
            cpu_enabled,
            cpu_percent,
            mem_enabled,
            mem_percent,
        })
    }
}

/// Configuration plus every warning produced while reading it. Warnings are
/// reported by the caller instead of printed here so callers stay testable.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemResourceCapConfigLoad {
    pub config: SystemResourceCapConfig,
    pub warnings: Vec<String>,
}

/// Parse one percentage env value. Absent values keep the default silently;
/// present-but-invalid values warn and fall back to the default.
fn parse_percent(value: &str, env_name: &str, default: f64) -> (f64, Option<String>) {
    match value.trim().parse::<f64>() {
        Ok(parsed) if parsed.is_finite() && parsed > 0.0 && parsed <= 100.0 => (parsed, None),
        Ok(parsed) => (
            default,
            Some(format!(
                "invalid {env_name} '{value}': {parsed} is outside (0, 100]; using default {default}"
            )),
        ),
        Err(_) => (
            default,
            Some(format!(
                "invalid {env_name} '{value}': not a number; using default {default}"
            )),
        ),
    }
}

impl SystemResourceCapConfigLoad {
    pub fn from_env_map(env: &HashMap<String, String>) -> Self {
        let (cpu_percent, cpu_warning) = match env.get(SYSTEM_CPU_CAP_ENV) {
            Some(value) => parse_percent(value, SYSTEM_CPU_CAP_ENV, DEFAULT_SYSTEM_CPU_CAP_PERCENT),
            None => (DEFAULT_SYSTEM_CPU_CAP_PERCENT, None),
        };
        let (mem_percent, mem_warning) = match env.get(SYSTEM_MEM_CAP_ENV) {
            Some(value) => parse_percent(value, SYSTEM_MEM_CAP_ENV, DEFAULT_SYSTEM_MEM_CAP_PERCENT),
            None => (DEFAULT_SYSTEM_MEM_CAP_PERCENT, None),
        };
        let warnings = [cpu_warning, mem_warning]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Self {
            config: SystemResourceCapConfig {
                cpu_enabled: true,
                cpu_percent,
                mem_enabled: true,
                mem_percent,
            },
            warnings,
        }
    }

    pub fn from_process_env() -> Self {
        Self::from_env_map(&std::env::vars().collect::<HashMap<_, _>>())
    }
}

/// Serde default helpers: an absent persisted setting seeds itself from the
/// environment (falling back to 85), so the env surface stays the initial
/// default while an explicit WebUI value wins afterwards.
pub fn system_cpu_cap_percent_default_from_env() -> f64 {
    SystemResourceCapConfigLoad::from_process_env()
        .config
        .cpu_percent
}

pub fn system_mem_cap_percent_default_from_env() -> f64 {
    SystemResourceCapConfigLoad::from_process_env()
        .config
        .mem_percent
}

/// Enable flags default to on: absent persisted fields keep the dimension
/// managed, matching the historical behavior before switches existed.
pub fn system_cap_enabled_default() -> bool {
    true
}

/// Remove the runtime's managed resource-limit scopes after a graceful shutdown
/// (best effort; see the platform contract for the exact retention rules).
/// Intended once the Web runtime finished its own cleanup, right before the
/// process exits.
pub fn cleanup_instance_resource_limits() {
    crate::os::cleanup_instance_resource_limits();
}

/// Apply the caps at host startup from the persisted system settings (the
/// `WebMemSettings` values, which themselves seed from the environment).
/// Intended after the settings load and before any managed job can run.
/// A not-reached-intent outcome is reported loudly on stderr because
/// silently losing the kernel-side bound is forbidden.
pub fn configure_instance_resource_caps_from_config(
    config: SystemResourceCapConfig,
) -> std::sync::Arc<ResourceCapsApplication> {
    let application = os::apply_instance_resource_caps(InstanceResourceCaps {
        cpu_enabled: config.cpu_enabled,
        cpu_percent: config.cpu_percent,
        mem_enabled: config.mem_enabled,
        mem_percent: config.mem_percent,
    });
    if !application.fully_applied() {
        eprintln!(
            "[config_error] instance resource caps not fully applied: cpu {:?}; mem {:?}; scope {}",
            application.cpu, application.mem, application.scope
        );
    }
    application
}

/// Update the caps at runtime from explicitly submitted values (WebUI
/// system settings). Validation failures and kernel failures are both
/// rejected with an error, so callers never persist a setting that is not
/// what the kernel enforces.
pub fn update_system_resource_caps(
    cpu_enabled: bool,
    cpu_percent: f64,
    mem_enabled: bool,
    mem_percent: f64,
) -> Result<ResourceCapsApplication, String> {
    let config =
        SystemResourceCapConfig::validated(cpu_enabled, cpu_percent, mem_enabled, mem_percent)?;
    os::update_instance_resource_caps(InstanceResourceCaps {
        cpu_enabled: config.cpu_enabled,
        cpu_percent: config.cpu_percent,
        mem_enabled: config.mem_enabled,
        mem_percent: config.mem_percent,
    })
}
