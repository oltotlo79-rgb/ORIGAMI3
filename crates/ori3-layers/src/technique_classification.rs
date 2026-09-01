//! Deterministic classification boundary for named folding techniques.
//!
//! Geometry-specific recognizers are intentionally not implemented here.  A
//! caller must supply an exact [`TechniqueWitness`] produced from an aligned
//! motion.  Until a recognizer can prove exactly one named technique, the
//! caller receives `None` and must present the move as a grabbed move.

use ori3_model::TechniqueKind;

use crate::flat_motion::FlatMotionInput;

/// Evidence emitted by a geometry-specific recognizer.
///
/// `LayerOperation` deliberately never produces an automatic name: it is a
/// broad interaction family and may only be named when the user selects it
/// manually. `Insufficient` records that a recognizer could not prove its
/// proposed name; it therefore prevents automatic classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TechniqueWitness {
    Pleat,
    InsideReverse,
    OutsideReverse,
    Squash,
    Petal,
    OpenSink,
    Swivel,
    Twist,
    LayerOperation,
    Insufficient,
}

impl TechniqueWitness {
    fn named_kind(self) -> Option<TechniqueKind> {
        match self {
            Self::Pleat => Some(TechniqueKind::Pleat),
            Self::InsideReverse => Some(TechniqueKind::InsideReverse),
            Self::OutsideReverse => Some(TechniqueKind::OutsideReverse),
            Self::Squash => Some(TechniqueKind::Squash),
            Self::Petal => Some(TechniqueKind::Petal),
            Self::OpenSink => Some(TechniqueKind::OpenSink),
            Self::Swivel => Some(TechniqueKind::Swivel),
            Self::Twist => Some(TechniqueKind::Twist),
            Self::LayerOperation | Self::Insufficient => None,
        }
    }
}

/// Classifies an aligned motion only when its witnesses prove one name.
///
/// `Some` is a named technique. `None` means "つかんで動かした折り": there
/// were no matches, more than one distinct match, a layer operation, or
/// insufficient proof. The result depends on neither witness order nor the
/// floating-point representation used by the motion.
pub fn classify_aligned_motion(
    motion: &FlatMotionInput,
    witnesses: &[TechniqueWitness],
) -> Option<TechniqueKind> {
    if motion.parts.is_empty()
        || witnesses.iter().any(|witness| {
            matches!(
                witness,
                TechniqueWitness::LayerOperation | TechniqueWitness::Insufficient
            )
        })
    {
        return None;
    }

    let mut candidate = None;
    for &witness in witnesses {
        let Some(kind) = witness.named_kind() else {
            continue;
        };
        match candidate {
            None => candidate = Some(kind),
            Some(existing) if existing == kind => {}
            Some(_) => return None,
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use ori3_model::{FoldDirection, TechniqueKind};

    use super::{classify_aligned_motion, TechniqueWitness};
    use crate::flat_motion::{FlatMotionInput, MotionPart};

    fn aligned_motion() -> FlatMotionInput {
        FlatMotionInput {
            parts: vec![MotionPart::fold(
                vec![0],
                [[0.0, 0.0], [0.0, 1.0]],
                [1.0, 0.0],
                FoldDirection::Up,
            )],
            kind: TechniqueKind::Simple,
        }
    }

    macro_rules! names_exactly_one_witness {
        ($name:ident, $witness:expr, $kind:expr) => {
            #[test]
            fn $name() {
                let motion = aligned_motion();
                assert_eq!(classify_aligned_motion(&motion, &[$witness]), Some($kind));
            }
        };
    }

    names_exactly_one_witness!(
        classifies_pleat,
        TechniqueWitness::Pleat,
        TechniqueKind::Pleat
    );
    names_exactly_one_witness!(
        classifies_inside_reverse,
        TechniqueWitness::InsideReverse,
        TechniqueKind::InsideReverse
    );
    names_exactly_one_witness!(
        classifies_outside_reverse,
        TechniqueWitness::OutsideReverse,
        TechniqueKind::OutsideReverse
    );
    names_exactly_one_witness!(
        classifies_squash,
        TechniqueWitness::Squash,
        TechniqueKind::Squash
    );
    names_exactly_one_witness!(
        classifies_petal,
        TechniqueWitness::Petal,
        TechniqueKind::Petal
    );
    names_exactly_one_witness!(
        classifies_open_sink,
        TechniqueWitness::OpenSink,
        TechniqueKind::OpenSink
    );
    names_exactly_one_witness!(
        classifies_swivel,
        TechniqueWitness::Swivel,
        TechniqueKind::Swivel
    );
    names_exactly_one_witness!(
        classifies_twist,
        TechniqueWitness::Twist,
        TechniqueKind::Twist
    );

    #[test]
    fn returns_grabbed_move_for_zero_matches_or_a_layer_operation() {
        let motion = aligned_motion();
        assert_eq!(classify_aligned_motion(&motion, &[]), None);
        assert_eq!(
            classify_aligned_motion(&motion, &[TechniqueWitness::LayerOperation]),
            None
        );
    }

    #[test]
    fn returns_grabbed_move_for_ambiguous_matches() {
        let motion = aligned_motion();
        assert_eq!(
            classify_aligned_motion(
                &motion,
                &[TechniqueWitness::Pleat, TechniqueWitness::InsideReverse],
            ),
            None
        );
    }

    #[test]
    fn returns_grabbed_move_when_proof_is_insufficient() {
        let motion = aligned_motion();
        assert_eq!(
            classify_aligned_motion(
                &motion,
                &[TechniqueWitness::Pleat, TechniqueWitness::Insufficient]
            ),
            None
        );
    }

    #[test]
    fn classification_is_repeatable_and_independent_of_witness_order() {
        let motion = aligned_motion();
        let witnesses = [TechniqueWitness::Pleat, TechniqueWitness::Pleat];
        let first = classify_aligned_motion(&motion, &witnesses);
        assert_eq!(first, Some(TechniqueKind::Pleat));
        for _ in 0..32 {
            assert_eq!(classify_aligned_motion(&motion, &witnesses), first);
            assert_eq!(
                classify_aligned_motion(
                    &motion,
                    &[TechniqueWitness::Pleat, TechniqueWitness::Pleat]
                ),
                first
            );
        }
    }
}
