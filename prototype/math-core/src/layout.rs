//! Symbol layout for the all-but-one-child construction (spec §5.9) and the
//! view-pure prefix structure (spec §5.7). **This is the subtle part.**
//!
//! # What is stored
//!
//! The construction stores exactly `n` group symbols:
//!
//! 1. **Level-0 segment**: the sum of every block of `P_0`, one symbol per block,
//!    in block order. These `|P_0|` symbols are precisely the coarsest view
//!    `F_0(x)` (spec §5.3).
//! 2. **Refinement segments**, for each level `t = 1..=q`: for every parent block
//!    of `P_{t-1}` (in parent block order), store the sums of **all but one** of
//!    its child blocks in `P_t`. The omitted child's sum is recoverable as
//!    `parent_sum − Σ stored_sibling_sums`.
//!
//! The symbol count telescopes to `n`:
//! `|P_0| + Σ_{t≥1}(|P_t| − |P_{t-1}|) = |P_q| = n`.
//!
//! # Slot order and why the prefix is view-pure
//!
//! Slots are laid out level by level: all of level 0, then all stored children
//! of level 1, then level 2, and so on. Within a level, parents are visited in
//! parent-block order and children in child-block order, skipping the omitted
//! child. This gives the checkpoint bounds
//!
//! ```text
//! b_0 = |P_0|,   b_t = b_{t-1} + (stored symbols at level t),   b_q = n.
//! ```
//!
//! The prefix `[0, b_t)` holds exactly the symbols needed to reconstruct
//! `F_0 … F_t` and nothing finer: recovering an omitted child at level `t` reads
//! only its parent sum (in a strictly-earlier segment) and its stored siblings
//! (same segment). No later symbol, and no finer coordinate, ever leaks into the
//! prefix — so two source states with the same view `F_t` share an identical
//! length-`b_t` prefix (spec §5.7, MATH-004). Because every refined parent has
//! `≥ 2` children and omits exactly one, each refinement segment is nonempty, so
//! `b_0 < b_1 < … < b_{q-1} < n` strictly.

use crate::hierarchy::Hierarchy;

/// Which child a parent omits. Default stores all children except the last in
/// child-block order (spec §5.9 "configurable omitted child per parent").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OmissionPolicy {
    /// Omit the last child (by child-block order) of every parent.
    LastChild,
    /// Omit the first child of every parent.
    FirstChild,
    /// Explicit per-(level, parent-block) omitted child index, given as the
    /// omitted child's position **within that parent's child list**.
    Explicit(Vec<Vec<usize>>),
}

/// A stored symbol slot: this symbol holds the group sum of `block` at `level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub level: usize,
    /// Block id within `level` whose sum this symbol stores.
    pub block: usize,
}

/// One parent's child layout at a refinement level.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParentChildren {
    /// Child block ids (in child-block order) of this parent.
    children: Vec<usize>,
    /// Position within `children` of the omitted child.
    omitted_pos: usize,
}

/// The full symbol layout for an encoder: the `index → (level, block)` map plus
/// per-level parent/child structure used by encode, decode, and update.
#[derive(Debug, Clone)]
pub struct Layout {
    n: usize,
    q: usize,
    /// `slots[r]` = what stored symbol `r` means. Length `n`.
    slots: Vec<Slot>,
    /// `checkpoint_bounds[t]` = `b_t` (prefix length that encodes `F_t`).
    /// Length `q + 1`; `checkpoint_bounds[q] == n`.
    checkpoint_bounds: Vec<usize>,
    /// `symbol_of[level][block]` = slot index storing that block's sum, or
    /// `None` if that block is an omitted child (not stored).
    symbol_of: Vec<Vec<Option<usize>>>,
    /// `refine[t]` (for `t ≥ 1`) = per-parent child structure, indexed by parent
    /// block id of level `t-1`. `refine[0]` is empty/unused.
    refine: Vec<Vec<ParentChildren>>,
}

impl Layout {
    /// Build the layout for a validated hierarchy under an omission policy.
    pub fn build(h: &Hierarchy, policy: &OmissionPolicy) -> Layout {
        let n = h.n();
        let q = h.q();

        let mut slots: Vec<Slot> = Vec::with_capacity(n);
        let mut symbol_of: Vec<Vec<Option<usize>>> = h
            .levels()
            .iter()
            .map(|lv| vec![None; lv.num_blocks()])
            .collect();
        let mut checkpoint_bounds = vec![0usize; q + 1];
        let mut refine: Vec<Vec<ParentChildren>> = vec![Vec::new(); q + 1];

        // Level 0: store every block sum, in block order. This is F_0.
        for (b, slot_ref) in symbol_of[0].iter_mut().enumerate() {
            *slot_ref = Some(slots.len());
            slots.push(Slot { level: 0, block: b });
        }
        checkpoint_bounds[0] = slots.len();

        // Levels 1..=q: all-but-one-child per parent.
        for t in 1..=q {
            let parent = h.level(t - 1);
            let child = h.level(t);

            // Group child blocks by parent, preserving child-block order.
            let mut children_by_parent: Vec<Vec<usize>> = vec![Vec::new(); parent.num_blocks()];
            for cb in 0..child.num_blocks() {
                let any_coord = child.blocks[cb][0];
                let p = parent.block_of[any_coord];
                children_by_parent[p].push(cb);
            }

            let mut level_refine: Vec<ParentChildren> = Vec::with_capacity(parent.num_blocks());
            for (p, children) in children_by_parent.into_iter().enumerate() {
                let omitted_pos = omitted_position(policy, t, p, children.len());
                // Store every child except the omitted one, in child-block order.
                for (pos, &cb) in children.iter().enumerate() {
                    if pos == omitted_pos {
                        continue;
                    }
                    symbol_of[t][cb] = Some(slots.len());
                    slots.push(Slot {
                        level: t,
                        block: cb,
                    });
                }
                level_refine.push(ParentChildren {
                    children,
                    omitted_pos,
                });
            }
            refine[t] = level_refine;
            checkpoint_bounds[t] = slots.len();
        }

        debug_assert_eq!(slots.len(), n, "construction must use exactly n symbols");
        debug_assert_eq!(checkpoint_bounds[q], n);

        Layout {
            n,
            q,
            slots,
            checkpoint_bounds,
            symbol_of,
            refine,
        }
    }

    #[inline]
    pub fn n(&self) -> usize {
        self.n
    }

    #[inline]
    pub fn q(&self) -> usize {
        self.q
    }

    /// `index → (level, block)` map (spec §5.9 "stable symbol ordering").
    #[inline]
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// Prefix length `b_t` that encodes exactly `F_t`. `t` in `0..=q`.
    #[inline]
    pub fn checkpoint_bound(&self, t: usize) -> usize {
        self.checkpoint_bounds[t]
    }

    /// All checkpoint bounds `b_0..=b_q`.
    #[inline]
    pub fn checkpoint_bounds(&self) -> &[usize] {
        &self.checkpoint_bounds
    }

    /// Slot index storing `(level, block)`'s sum, or `None` if it is an omitted
    /// child (reconstructed, not stored).
    #[inline]
    pub fn symbol_of(&self, level: usize, block: usize) -> Option<usize> {
        self.symbol_of[level][block]
    }

    /// For level `t ≥ 1` and a parent block id of level `t-1`, return the child
    /// block ids (child-block order) and the omitted position.
    #[inline]
    pub(crate) fn parent_children(&self, t: usize, parent_block: usize) -> (&[usize], usize) {
        let pc = &self.refine[t][parent_block];
        (&pc.children, pc.omitted_pos)
    }
}

/// Which child position a parent omits, given the policy. Falls back to the last
/// child when an `Explicit` entry is missing or out of range (fail-safe default).
fn omitted_position(
    policy: &OmissionPolicy,
    level: usize,
    parent: usize,
    num_children: usize,
) -> usize {
    debug_assert!(num_children >= 2);
    match policy {
        OmissionPolicy::LastChild => num_children - 1,
        OmissionPolicy::FirstChild => 0,
        OmissionPolicy::Explicit(table) => table
            .get(level)
            .and_then(|row| row.get(parent))
            .copied()
            .filter(|&pos| pos < num_children)
            .unwrap_or(num_children - 1),
    }
}
