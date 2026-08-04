// Write-wave fan-out width: the one place a Phase 8 structural knob reaches the
// running lifecycle. Split out of `..._tests_b.rs` to hold the 500-line ceiling.

use crate::v2::lifecycle_policy::verify_options::write_wave_parallelism;

/// Constraint 3, at the one place a structural knob touches the running
/// lifecycle: the cargo pin is a floor, not a preference. Concurrent `cargo`
/// invocations contend on the target-directory lock, and the branches that lose
/// report build failures the repair loop then chases — failures the knob would
/// have manufactured. No learned width may widen past it.
#[test]
fn no_learned_width_can_widen_a_cargo_serialised_wave() {
    let items = vec![serde_json::json!({
        "item_id": "write-cargo",
        "focused_verification": ["cargo test focused"]
    })];

    for width in [1usize, 2, 4, 64] {
        assert_eq!(
            write_wave_parallelism(&items, Some(width)),
            1,
            "a learned width of {width} must not unpin a cargo wave"
        );
    }
}

/// Absent a learned width the options are byte-identical to what every run got
/// before the knob existed — the string `"configured"`, not a number.
#[test]
fn a_wave_with_no_learned_width_still_defers_to_the_configured_cap() {
    let items = vec![serde_json::json!({ "item_id": "write-plain" })];

    assert_eq!(
        write_wave_parallelism(&items, None),
        serde_json::Value::String("configured".to_string())
    );
}

#[test]
fn a_learned_width_reaches_a_non_cargo_wave_and_never_falls_below_one() {
    let items = vec![serde_json::json!({ "item_id": "write-plain" })];

    assert_eq!(write_wave_parallelism(&items, Some(2)), 2);
    assert_eq!(
        write_wave_parallelism(&items, Some(0)),
        1,
        "a wave that dispatches nothing completes nothing"
    );
}
