//! Finite abelian group `A = Z/kZ` under addition mod `k` (spec §5.1).
//!
//! The prototype supports `Z/kZ` only. That is enough to exercise the full
//! theorem: it is a nontrivial finite abelian group for every `k ≥ 2`, and all
//! block sums `F_t(x)_B = Σ x_i` are group sums (spec §5.3). Group elements are
//! stored as `u64` in canonical form `[0, k)`.

use crate::error::{MathError, Result};

/// `Z/kZ` under addition mod `k`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    k: u64,
}

/// A group element, always held in canonical form `[0, k)` by construction.
pub type Elem = u64;

impl Group {
    /// Create `Z/kZ`. Rejects `k < 2` (spec §5.1: `A` must be nontrivial).
    pub fn new(k: u64) -> Result<Self> {
        if k < 2 {
            return Err(MathError::InvalidGroup { k });
        }
        Ok(Group { k })
    }

    /// The modulus `k` (alphabet size).
    #[inline]
    pub fn modulus(&self) -> u64 {
        self.k
    }

    /// The additive identity `0`.
    #[inline]
    pub fn zero(&self) -> Elem {
        0
    }

    /// Reduce an arbitrary value into canonical `[0, k)`.
    #[inline]
    pub fn canon(&self, v: u64) -> Elem {
        v % self.k
    }

    /// `a + b mod k`. Inputs assumed canonical; the result is canonical.
    #[inline]
    pub fn add(&self, a: Elem, b: Elem) -> Elem {
        // k < 2^63 in any realistic use, so a+b cannot overflow u64 for canonical inputs;
        // reduce defensively regardless.
        (a % self.k + b % self.k) % self.k
    }

    /// `-a mod k`.
    #[inline]
    pub fn neg(&self, a: Elem) -> Elem {
        let a = a % self.k;
        if a == 0 {
            0
        } else {
            self.k - a
        }
    }

    /// `a - b mod k`.
    #[inline]
    pub fn sub(&self, a: Elem, b: Elem) -> Elem {
        self.add(a, self.neg(b))
    }

    /// True if `v` is a legal symbol value (canonical member of the alphabet).
    #[inline]
    pub fn in_alphabet(&self, v: u64) -> bool {
        v < self.k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_trivial_moduli() {
        assert!(Group::new(0).is_err());
        assert!(Group::new(1).is_err());
        assert!(Group::new(2).is_ok());
    }

    #[test]
    fn arithmetic_mod_k() {
        let g = Group::new(5).unwrap();
        assert_eq!(g.add(3, 4), 2);
        assert_eq!(g.neg(2), 3);
        assert_eq!(g.sub(1, 3), 3);
        assert_eq!(g.add(g.neg(4), 4), 0);
    }
}
