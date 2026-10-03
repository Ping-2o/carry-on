//! In-process adapter host (spec §7.3/§10.4). Owns registered adapters, enforces
//! consent, verifies manifests, and contains adapter crashes so a panicking
//! adapter cannot corrupt the core (ADP-007).

pub mod consent;
pub mod registry;

pub use registry::{verify_manifest, RegisteredAdapter, CORE_MAX_OBJECT_BYTES};

use crate::error::{AdapterCode, CoreError, Result};
use carryon_adapter_api::{Adapter, AdapterInfo};
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Registry of in-process adapters, keyed by adapter id.
#[derive(Default)]
pub struct AdapterHost {
    adapters: BTreeMap<String, RegisteredAdapter>,
    /// Adapters quarantined after a crash; refuse further dispatch.
    quarantined: BTreeMap<String, String>,
}

impl AdapterHost {
    pub fn new() -> Self {
        AdapterHost::default()
    }

    /// Register a compiled-in adapter after verifying its manifest. Rejects a
    /// manifest whose declared info does not match the live `get_adapter_info()`.
    pub fn register(&mut self, adapter: Box<dyn Adapter>) -> Result<AdapterInfo> {
        let info = adapter.get_adapter_info();
        verify_manifest(&info)?;
        let id = info.adapter_id.clone();
        self.adapters.insert(
            id,
            RegisteredAdapter {
                info: info.clone(),
                adapter,
            },
        );
        Ok(info)
    }

    /// List registered adapter info.
    pub fn list(&self) -> Vec<AdapterInfo> {
        self.adapters.values().map(|r| r.info.clone()).collect()
    }

    /// Get one adapter's info.
    pub fn info(&self, id: &str) -> Option<AdapterInfo> {
        self.adapters.get(id).map(|r| r.info.clone())
    }

    /// Whether an adapter is registered and not quarantined.
    pub fn is_available(&self, id: &str) -> bool {
        self.adapters.contains_key(id) && !self.quarantined.contains_key(id)
    }

    /// Why an adapter was quarantined, if it was.
    pub fn quarantine_reason(&self, id: &str) -> Option<&str> {
        self.quarantined.get(id).map(|s| s.as_str())
    }

    /// Dispatch a call to an adapter with crash containment. A panic quarantines
    /// the adapter and returns `ADAPTER_Crashed` — never a partial success
    /// (ADP-007). The op's caller must mark the operation Failed.
    pub fn call<T>(
        &mut self,
        id: &str,
        f: impl FnOnce(&mut dyn Adapter) -> Result<T>,
    ) -> Result<T> {
        if let Some(reason) = self.quarantined.get(id) {
            return Err(CoreError::adapter(
                AdapterCode::Crashed,
                format!("adapter '{id}' is quarantined: {reason}"),
            ));
        }
        let reg = self.adapters.get_mut(id).ok_or_else(|| {
            CoreError::adapter(
                AdapterCode::Missing,
                format!("adapter '{id}' not registered"),
            )
        })?;

        let result = catch_unwind(AssertUnwindSafe(|| f(reg.adapter.as_mut())));
        match result {
            Ok(r) => r,
            Err(panic) => {
                let msg = panic_message(panic);
                self.quarantined.insert(id.to_string(), msg.clone());
                Err(CoreError::adapter(
                    AdapterCode::Crashed,
                    format!("adapter '{id}' panicked: {msg}"),
                ))
            }
        }
    }
}

/// Extract a readable message from a panic payload.
fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}
