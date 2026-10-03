//! §26.1 hierarchy validation; MATH-001 (reject invalid / non-strict hierarchy).

mod common;

use carryon_math_core::error::MathError;
use carryon_math_core::hierarchy;

#[test]
fn accepts_valid_strict_hierarchies() {
    for parts in [
        common::h4_balanced(),
        common::h8_deep(),
        common::h5_unbalanced(),
    ] {
        let n = parts[0].len();
        let h = hierarchy::create(n, &parts).expect("valid");
        assert!(hierarchy::validate(&h).ok);
    }
}

#[test]
fn rejects_non_singleton_top() {
    let e = hierarchy::create(4, &[vec![0, 0, 0, 0], vec![0, 0, 1, 1]]).unwrap_err();
    assert_eq!(e, MathError::NonSingletonTop);
}

#[test]
fn rejects_non_refinement() {
    // Level 1 blocks cross level-0 block boundaries.
    let e =
        hierarchy::create(4, &[vec![0, 0, 1, 1], vec![0, 1, 1, 2], vec![0, 1, 2, 3]]).unwrap_err();
    assert!(matches!(e, MathError::NonStrictHierarchy { .. }));
}

#[test]
fn rejects_parent_not_split_into_two() {
    // Identical coarse and next level: the single block is never split.
    let e = hierarchy::create(3, &[vec![0, 0, 0], vec![0, 0, 0], vec![0, 1, 2]]).unwrap_err();
    assert!(matches!(e, MathError::NonStrictHierarchy { .. }));
}

#[test]
fn rejects_bad_assignment_length() {
    let e = hierarchy::create(4, &[vec![0, 0, 0]]).unwrap_err();
    assert!(matches!(
        e,
        MathError::InvalidPartition { .. } | MathError::NonSingletonTop
    ));
}

#[test]
fn rejects_zero_n() {
    assert!(hierarchy::create(0, &[vec![]]).is_err());
}
