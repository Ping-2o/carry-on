//! # Carry-On shared core (Phase 1, local only)
//!
//! Session/cut/object graph, content-addressed object store, hash-chained
//! journal, preparation-policy interface, and the in-process adapter host.
//! Wraps `carryon-math-core` (unchanged) as a progressive block-sum view.
//!
//! **Phase 1 is local only**: no network, TLS, pairing, or cross-device transfer
//! (Phase 2). Adapters are compiled-in Rust, in-process. Every Phase-2 boundary
//! is labeled `TARGET` in code and never silently faked (spec §2/§30).

pub mod authority_xfer;
pub mod core;
pub mod db;
pub mod error;
pub mod evidence;
pub mod host;
pub mod ids;
pub mod journal;
pub mod mathview;
pub mod model;
pub mod policy;
pub mod prepare;
pub mod recovery;
pub mod store;
pub mod transfer;

pub use crate::authority_xfer::{AuthorityReceiptSet, SourcePending};
pub use crate::core::{Core, CreateSessionReq};
pub use crate::transfer::{ImportOutcome, ImportResume};
pub use error::{CoreError, Result};

/// Phase 2 transport re-export so callers drive handoff with one dependency.
pub use carryon_net;
