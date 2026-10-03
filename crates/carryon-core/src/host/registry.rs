//! Adapter registry + manifest verification (spec §7.3/§10.3/§10.4).
//!
//! Only allowlisted, compiled-in adapters are admitted. The manifest is verified
//! before registration: `executable` MUST be the `"compiled-in"` sentinel (no
//! remote code execution, §3.8; ADP-002 by construction), `network_access` MUST
//! be false, the integration level must be Phase-1-supported, and the declared
//! `max_object_bytes` must be within the core cap.

use crate::error::{AdapterCode, CoreError, Result, SchemaCode};
use carryon_adapter_api::{Adapter, AdapterInfo, IntegrationLevel};

/// Core cap on any single object (16 MiB in Phase 1; a declared manifest may set
/// a smaller `max_object_bytes`, never larger).
pub const CORE_MAX_OBJECT_BYTES: u64 = 16 * 1024 * 1024;

/// A registered adapter: its verified info plus the boxed implementation.
pub struct RegisteredAdapter {
    pub info: AdapterInfo,
    pub adapter: Box<dyn Adapter>,
}

/// Verify an adapter's declared info before admission. Returns the info on
/// success or a typed error (ADP-001/002/006).
pub fn verify_manifest(info: &AdapterInfo) -> Result<()> {
    if info.executable != AdapterInfo::COMPILED_IN {
        return Err(CoreError::adapter(
            AdapterCode::Malformed,
            format!(
                "executable must be '{}' (no remote code execution); got '{}'",
                AdapterInfo::COMPILED_IN,
                info.executable
            ),
        ));
    }
    if info.network_access {
        return Err(CoreError::adapter(
            AdapterCode::Incompatible,
            "network_access must be false in Phase 1",
        ));
    }
    match info.integration_level {
        // L4 (single-writer authority transfer) is admitted in Phase 4. L5
        // (ExplicitMerge) remains out of scope (§6.7).
        IntegrationLevel::L0
        | IntegrationLevel::L1
        | IntegrationLevel::L3
        | IntegrationLevel::L4 => {}
        other => {
            return Err(CoreError::schema(
                SchemaCode::Unsupported,
                format!("integration level {other:?} not supported"),
            ));
        }
    }
    if info.max_object_bytes > CORE_MAX_OBJECT_BYTES {
        return Err(CoreError::schema(
            SchemaCode::Invalid,
            format!(
                "max_object_bytes {} exceeds core cap {}",
                info.max_object_bytes, CORE_MAX_OBJECT_BYTES
            ),
        ));
    }
    if info.adapter_id.is_empty() {
        return Err(CoreError::schema(SchemaCode::Invalid, "empty adapter_id"));
    }
    Ok(())
}
