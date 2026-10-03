//! # Carry-On math core (first prototype)
//!
//! A progressive block-sum encoder over a finite abelian group with a strict
//! partition hierarchy. This crate implements the one **PROVED** component of
//! the Carry-On engine spec: the all-but-one-child construction of §5.9, whose
//! worst-case single-coordinate update changes exactly `q + 1` persistent
//! symbols (§5.8 lower bound).
//!
//! Scope is the math core only — no network, platform, adapter, transport, or
//! C ABI (those are later engine phases). This is a pure Rust library plus
//! tests.
//!
//! ## Module map
//!
//! | Module | Spec |
//! |---|---|
//! | [`group`] | §5.1 finite abelian group `Z/kZ` |
//! | [`hierarchy`] | §5.2 strict partition hierarchy + validation |
//! | [`layout`] | §5.7/§5.9 view-pure symbol layout (the subtle part) |
//! | [`encoder`] | §5.9 `AllButOneChildEncoder`: encode/decode/checkpoint/update/verify |
//! | [`testvectors`] | §5.12 deterministic test-vector export |
//! | [`api`] | §5.12 named-op facade + §5.11 applicability gate |
//!
//! ## Quick example
//!
//! ```
//! use carryon_math_core::api::*;
//! use carryon_math_core::layout::OmissionPolicy;
//!
//! // Z/5Z, n = 4, hierarchy {{0..3}} ≺ {{0,1},{2,3}} ≺ singletons  (q = 2).
//! let g = create_group(5).unwrap();
//! let h = create_hierarchy(4, &[
//!     vec![0, 0, 0, 0],
//!     vec![0, 0, 1, 1],
//!     vec![0, 1, 2, 3],
//! ]).unwrap();
//! let enc = create_all_but_one_encoder(g, h, OmissionPolicy::LastChild);
//!
//! let x = vec![1, 2, 3, 4];
//! let word = encode(&enc, &x).unwrap();
//! assert_eq!(decode_full(&enc, &word).unwrap(), x);
//!
//! // A single-coordinate update changes at most q + 1 = 3 symbols.
//! let upd = apply_coordinate_delta(&enc, &word, 0, 1).unwrap();
//! assert!(upd.endpoint_changed_symbols <= upd.expected_upper_bound);
//! assert!(applicability_gate(&enc).all_pass());
//! ```

pub mod api;
pub mod encoder;
pub mod error;
pub mod group;
pub mod hierarchy;
pub mod layout;
pub mod testvectors;

pub use error::{MathError, Result};
