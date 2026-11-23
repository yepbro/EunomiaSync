//! The assignment-toggle bench's pins: what the third §12.2 row
//! measures is the store's own write, so there is no mirrored
//! production spelling to tripwire — the pins guard the bench's own
//! premise instead, against oracles that do not run through the
//! toggle's plumbing.
//!
//! * The premise: the generator's first pair exists and holds the
//!   variant D-109's closed form promises at `p = 0, k = 0` (variant
//!   ordinal 0) — verified through the store's own read, and its
//!   refusal is typed when the row holds something else.
//! * The alternation: every [`AssignmentToggle::save`] is a real
//!   write, A→B→A…, verified through `get_assignment` on the
//!   database — never through the toggle's own return value alone.
//! * The no-op diagnostic truly targets the held variant: the row it
//!   returns is byte-identical to the row the store read just before
//!   (a stale target would be a real flip and would differ).

use eunomia_load::{AssignmentToggle, LoadError, LoadFixture, PremiseHalf};
use eunomia_store::data_sets::DataSetSpec;
use eunomia_store::library::{get_assignment, reassign, unassign};

/// A small profile with the shape the toggle needs: several
/// instructions, three variants each (a sibling to flip to),
/// assignments dense enough that project 0's `k = 0` assignment
/// exists. The premise is scale-independent (D-109's closed form at
/// the origin), so the pins run on this — set L itself is the bench's
/// fixture, and the generator's own suite pins the closed form at L
/// scale.
fn toggle_spec() -> DataSetSpec {
    DataSetSpec {
        projects: 4,
        headings: 2,
        instructions: 12,
        variants_per_instruction: 3,
        assignments_per_project: 6,
    }
}

/// The first pair's ids, owned, straight from the generated rows:
/// `(project, instruction, variant 0, variant 1)` — never through
/// the toggle, which is the code under test.
fn first_pair(fixture: &LoadFixture) -> (String, String, String, String) {
    let generated = fixture.generated();
    (
        generated.project_ids[0].clone(),
        generated.instruction_ids[0].clone(),
        generated.variant_ids[0][0].clone(),
        generated.variant_ids[0][1].clone(),
    )
}

/// The premise: on a freshly generated fixture the first pair's
/// assignment exists and holds the generator's variant ordinal 0,
/// and the sibling it will flip to is a *different* variant of the
/// *same* instruction.
#[test]
fn the_first_pair_holds_the_promised_variant() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&toggle_spec(), "eunomia-load-toggle-premise")?;
    let (project, instruction, main, sibling) = first_pair(&fixture);
    assert_ne!(
        main, sibling,
        "the flip target must be a different variant of the same instruction"
    );

    let held = get_assignment(fixture.connection(), &project, &instruction)
        .map_err(LoadError::Library)?
        .expect("the generator's k = 0 assignment of project 0 exists");
    assert_eq!(
        held.variant_id, main,
        "D-109's closed form at p = 0, k = 0: variant ordinal (0 + 0) mod V = 0"
    );

    AssignmentToggle::new(&mut fixture).expect("the premise holds on a fresh fixture");
    Ok(())
}

/// A premise violation is refused with the held and the promised id:
/// after moving the pair to the sibling through the store, the
/// constructor must not silently accept the changed row.
#[test]
fn a_premise_mismatch_is_refused_with_both_ids() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&toggle_spec(), "eunomia-load-toggle-mismatch")?;
    let (project, instruction, main, sibling) = first_pair(&fixture);
    reassign(fixture.connection(), &project, &instruction, &sibling).map_err(LoadError::Library)?;

    match AssignmentToggle::new(&mut fixture) {
        Err(LoadError::TogglePremise {
            half: PremiseHalf::VariantMismatch,
            held,
            expected,
        }) => {
            assert_eq!(held, sibling, "the refusal names what the row holds");
            assert_eq!(expected, main, "the refusal names what was promised");
        }
        other => panic!("a moved row must refuse the premise, got {other:?}"),
    }
    Ok(())
}

/// A profile without a sibling variant refuses with the shape
/// discriminator: the generator accepts `variants_per_instruction:
/// 1`, so the constructor — not the fixture — must catch the missing
/// flip target, and the refusal must say *which* half failed, not
/// just empty ids (round-1 review's P3-3).
#[test]
fn a_profile_without_a_sibling_refuses_the_shape() -> Result<(), LoadError> {
    let siblingless = DataSetSpec {
        projects: 2,
        headings: 1,
        instructions: 4,
        variants_per_instruction: 1,
        assignments_per_project: 2,
    };
    let mut fixture = LoadFixture::create(&siblingless, "eunomia-load-toggle-shape")?;
    match AssignmentToggle::new(&mut fixture) {
        Err(LoadError::TogglePremise {
            half: PremiseHalf::MissingShape,
            held,
            expected,
        }) => {
            assert!(held.is_empty(), "no row is named — none exists");
            assert!(expected.is_empty(), "no sibling is named — none exists");
        }
        other => panic!("a siblingless profile must refuse the shape, got {other:?}"),
    }
    Ok(())
}

/// A pair without its assignment refuses with the no-assignment
/// discriminator and still names the promised variant: the ids stay
/// meaningful even when the row is gone.
#[test]
fn an_unassigned_pair_refuses_with_the_promised_variant() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&toggle_spec(), "eunomia-load-toggle-unassigned")?;
    let (project, instruction, main, _sibling) = first_pair(&fixture);
    unassign(fixture.connection(), &project, &instruction).map_err(LoadError::Library)?;

    match AssignmentToggle::new(&mut fixture) {
        Err(LoadError::TogglePremise {
            half: PremiseHalf::NoAssignment,
            held,
            expected,
        }) => {
            assert!(held.is_empty(), "no row exists to name");
            assert_eq!(expected, main, "the promised variant is still named");
        }
        other => panic!("an unassigned pair must refuse the premise, got {other:?}"),
    }
    Ok(())
}

/// The alternation: every save is a real write. Verified through the
/// database (`get_assignment`), which is independent of the toggle's
/// own bookkeeping: save one flips to the sibling, save two flips
/// back, and the row's variant after each save is exactly the other
/// one — a save that silently hit the no-op path would leave the row
/// unchanged and fail here.
#[test]
fn every_save_is_a_real_write_and_alternates() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&toggle_spec(), "eunomia-load-toggle-alternates")?;
    let (project, instruction, main, sibling) = first_pair(&fixture);
    let mut toggle = AssignmentToggle::new(&mut fixture)?;

    for (ordinal, expected) in [&sibling, &main, &sibling, &main, &sibling, &main]
        .into_iter()
        .enumerate()
    {
        let returned = toggle.save(fixture.connection())?;
        let stored = get_assignment(fixture.connection(), &project, &instruction)
            .map_err(LoadError::Library)?
            .expect("the toggled pair keeps its assignment");
        assert_eq!(
            &returned.variant_id, expected,
            "save {ordinal}: the returned row names the target"
        );
        assert_eq!(
            &stored.variant_id, expected,
            "save {ordinal}: the database row carries the real write"
        );
    }
    Ok(())
}

/// The no-op diagnostic targets the held variant: after an odd number
/// of saves the row holds the sibling, and `noop_save`'s row is
/// byte-identical to the one the store read just before — a stale
/// target (the toggle's state desynced from the row) would be a real
/// flip, and the rows would differ in `variant_id` and `updated_at`.
#[test]
fn the_noop_targets_the_held_variant() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&toggle_spec(), "eunomia-load-toggle-noop")?;
    let (project, instruction, _, sibling) = first_pair(&fixture);
    let mut toggle = AssignmentToggle::new(&mut fixture)?;
    toggle.save(fixture.connection())?;

    let before = get_assignment(fixture.connection(), &project, &instruction)
        .map_err(LoadError::Library)?
        .expect("the toggled pair keeps its assignment");
    assert_eq!(
        before.variant_id, sibling,
        "one save leaves the sibling held"
    );
    let unchanged = toggle.noop_save(fixture.connection())?;
    assert_eq!(
        before, unchanged,
        "the no-op writes nothing: identical row, no restamp"
    );
    Ok(())
}
