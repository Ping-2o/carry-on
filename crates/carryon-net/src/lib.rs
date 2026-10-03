//! # Carry-On Phase 2 transport (`carryon-net`)
//!
//! Authenticated cross-device transport for the Carry-On engine (spec §8.6/§18).
//! Real TLS 1.3 with mutual certificate **pinning** established at pairing, blocking
//! length-delimited control frames, per-session sequence/replay protection, and a
//! content-addressed chunk-transfer protocol that preserves the two-phase
//! digest-verify discipline over the wire.
//!
//! ## Boundary (spec §7.4)
//!
//! This crate has **no dependency on `carryon-core`**. It defines the wire model
//! and the transport session; the core drives it (source serves a sealed cut's
//! bytes, destination pulls and re-verifies them before publishing). Nothing here
//! can select an executable path, class name, or shell string — a transfer names a
//! content digest, never a path (§3.8, no remote code execution).
//!
//! ## Evidence honesty (spec §2/§30)
//!
//! Loopback between two in-process cores on `127.0.0.1` is **local evidence**, not
//! physical cross-device evidence. No platform is "supported" until Section 30
//! passes on real hardware. Live mDNS discovery is a shell TARGET (see
//! [`discovery`]).

pub mod discovery;
pub mod error;
pub mod frame;
pub mod identity;
pub mod pairing;
pub mod tls;
pub mod transport;
pub mod trust;
pub mod wire;

pub use discovery::{DeviceClass, DiscoveredPeer, Discovery, StaticDiscovery, TxtRecord};
pub use error::{AuthCode, NetError, ProtoCode, Result, TransferCode};
pub use identity::{DeviceIdentity, Pin};
pub use pairing::{pair_devices, PairingData};
pub use transport::Session;
pub use trust::TrustStore;
pub use wire::{Envelope, Message, PROTOCOL_MAJOR, PROTOCOL_MINOR};
