//! Round 11 regressions and generated single-defect repair corpus.
use super::*;
use crate::command::workflow_freeze_candidate::acceptance_candidate_for_validation;
use archon_workflow::task_set_contract::AcceptanceContract;
use archon_workflow::task_skeleton::TaskSkeleton;
use serde_json::{Value, json};

#[path = "workflow_freeze_shape_fixtures.rs"]
mod fixtures;

#[path = "workflow_freeze_duplicate_tests.rs"]
mod duplicates;

fn defects(value: &Value, shape: &ElementShape) -> Vec<String> {
    element_shape_defects(&serde_json::to_vec(value).unwrap(), shape)
        .into_iter()
        .map(|d| d.identity.subject)
        .collect()
}
fn entry() -> Value {
    json!({"id":"AC-X-001", "criterion":"output exists",
        "check":{"kind":"command", "command":"test -f out.json", "cwd":"project_root"}})
}
fn legacy(entry: Value) -> Value {
    json!({"schema_version":1,"prd":{"path":"","digest":""},"gap_policy":{},"acceptance":[entry]})
}
fn accepts(value: &Value, shape: &ElementShape) -> bool {
    let bytes = serde_json::to_vec(value).unwrap();
    if matches!(shape, ElementShape::Tasks) {
        serde_json::from_slice::<TaskSkeleton>(&bytes).is_ok()
    } else {
        acceptance_candidate_for_validation(&bytes)
            .ok()
            .is_some_and(|bytes| serde_json::from_slice::<AcceptanceContract>(&bytes).is_ok())
    }
}
fn decreases(before: &Value, after: &Value, shape: &ElementShape) {
    let b = defects(before, shape);
    let a = defects(after, shape);
    assert!(
        !b.is_empty() && a.len() < b.len(),
        "repair must decrease: {} -> {}; before={before}; after={after}; identities={b:?} -> {a:?}",
        b.len(),
        a.len()
    );
    if a.is_empty() {
        assert!(accepts(after, shape), "zero defects must assemble: {after}");
    }
}

#[test]
fn workflow_freeze_round11_optional_fields_do_not_collapse_or_hide() {
    for invalid_required in [false, true] {
        let mut candidate = json!({"entries":[{"id":"AC-X-001","criterion":"output exists","check":{"kind":"floor","contract":{
            "kind":"file","artifact_path":"out.json","registry_path":0,"instance_source_path":0,"typed_verifier_command":0,"artifact_format":0}}}]});
        if invalid_required {
            candidate["entries"][0]["criterion"] = json!(false);
        }
        let names = defects(&candidate, &ENTRY_SHAPE);
        assert_eq!(names.len(), 4 + usize::from(invalid_required), "{names:?}");
        for field in [
            "registry_path",
            "instance_source_path",
            "typed_verifier_command",
            "artifact_format",
        ] {
            let mut repaired = candidate.clone();
            repaired["entries"][0]["check"]["contract"][field] = json!("field");
            decreases(&candidate, &repaired, &ENTRY_SHAPE);
            candidate = repaired;
        }
    }
}

#[test]
fn workflow_freeze_round11_containers_and_tags_never_reveal_defects() {
    let mut candidate = json!({"entries":[entry()]});
    candidate["entries"][0]["check"] = json!({"kind":"floor","contract":false});
    let mut repaired = candidate.clone();
    repaired["entries"][0]["check"]["contract"] = json!({});
    decreases(&candidate, &repaired, &ENTRY_SHAPE);
    for check in [
        json!({}),
        json!({"kind":"unknown"}),
        json!({"command":"true","cwd":"project_root","contract":{"kind":"file","artifact_path":"a"}}),
    ] {
        candidate["entries"][0]["check"] = check;
        for kind in ["command", "floor"] {
            repaired = candidate.clone();
            repaired["entries"][0]["check"]["kind"] = json!(kind);
            decreases(&candidate, &repaired, &ENTRY_SHAPE);
        }
    }
}

#[test]
fn workflow_freeze_round11_legacy_judgment_is_validated_as_assembled() {
    let mut e = entry();
    e["judgment"] = json!({});
    let mut candidate = legacy(e);
    let fields = [
        ("verdict", "accepted"),
        ("counterexample", ""),
        ("reason", ""),
        ("host_call_id", ""),
    ];
    assert_eq!(defects(&candidate, &ENTRY_SHAPE).len(), 4);
    for (field, fill) in fields {
        let mut repaired = candidate.clone();
        repaired["acceptance"][0]["judgment"][field] = json!(fill);
        decreases(&candidate, &repaired, &ENTRY_SHAPE);
        candidate = repaired;
    }
}

#[test]
fn workflow_freeze_round11_discarded_acceptance_remains_resumable() {
    let mut ignored = entry();
    ignored["criterion"] = json!("ignored");
    ignored["covers"] = json!(0);
    let candidate = json!({"entries":[entry()],"acceptance":[ignored]});
    assert!(accepts(&candidate, &ENTRY_SHAPE));
    assert_eq!(defects(&candidate, &ENTRY_SHAPE), Vec::<String>::new());
}

fn pointers(value: &Value, at: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let p = format!("{at}/{}", key.replace('~', "~0").replace('/', "~1"));
                out.push(p.clone());
                pointers(child, &p, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let p = format!("{at}/{index}");
                out.push(p.clone());
                pointers(child, &p, out);
            }
        }
        _ => {}
    }
}
fn restore(value: &mut Value, sample: &Value, pointer: &str) {
    let original = sample.pointer(pointer).unwrap().clone();
    if let Some(target) = value.pointer_mut(pointer) {
        *target = original;
    } else {
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[key] = original;
    }
}
fn remove(value: &mut Value, pointer: &str) -> bool {
    if pointer.is_empty() {
        return false;
    }
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    if let Some(map) = value.pointer_mut(parent).and_then(Value::as_object_mut) {
        map.remove(key);
        true
    } else {
        false
    }
}

#[test]
fn workflow_freeze_round11_generated_repairs_strictly_decrease_and_zero_assembles() {
    generated_repairs(false);
}

#[test]
#[ignore = "full repeated-element corpus for manual runs"]
fn workflow_freeze_round12_exhaustive_generated_repairs() {
    generated_repairs(true);
}

// Identical list members repeat all the same mutations and multiply whole-
// document validation costs. Keep every distinct member, field and probe.
fn compact(value: &mut Value) {
    match value {
        Value::Array(items) => {
            if items.first().is_some_and(|v| v.is_object() || v.is_array())
                && items.iter().all(|v| v == &items[0])
            {
                items.truncate(1);
            }
            for item in items {
                compact(item);
            }
        }
        Value::Object(map) => {
            for child in map.values_mut() {
                compact(child);
            }
        }
        _ => {}
    }
}

fn generated_repairs(exhaustive: bool) {
    assert_eq!(
        serde_json::to_value(fixtures::contract())
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        48
    );
    let mut corpus = vec![(&TASK_SHAPE, fixtures::skeleton(fixtures::task()))];
    // Fixed representatives cover both cwd values, both verdicts and both
    // check variants. Their Cartesian product repeats identical field/probe
    // checks; keep that product in the ignored exhaustive corpus only.
    for (index, e) in fixtures::entries().into_iter().enumerate() {
        if !exhaustive && ![0, 3, 4].contains(&index) {
            continue;
        }
        corpus.push((
            &ENTRY_SHAPE,
            json!({"entries":[e.clone(), e.clone()],"supplementary":[e.clone(), e.clone()]}),
        ));
        corpus.push((&ENTRY_SHAPE, fixtures::legacy(e)));
    }
    corpus.extend(
        fixtures::positional_samples()
            .into_iter()
            .map(|(task, value)| (if task { &TASK_SHAPE } else { &ENTRY_SHAPE }, value)),
    );
    let probes = [
        Value::Null,
        json!(0),
        json!(-1),
        json!(1.5),
        json!(true),
        json!({}),
        json!([]),
        json!("invalid-enum"),
        json!(255),
        json!(256),
        json!(u32::MAX),
        json!(u64::MAX),
        json!(18446744073709551616.0),
    ];
    let (mut mutations, mut sequences, mut failures) = (0, 0, Vec::new());
    let (mut duplicate_mutation_count, mut duplicate_repair_count) = (0, 0);
    // Parent/leaf kinds whose full duplicate-copy walk already ran.
    let mut kinds = std::collections::HashSet::new();
    for (shape, mut sample) in corpus {
        if !exhaustive {
            compact(&mut sample);
        }
        let (duplicate_mutations, duplicate_repairs) =
            duplicates::check(&sample, shape, &mut kinds, exhaustive);
        mutations += duplicate_mutations;
        sequences += duplicate_repairs;
        duplicate_mutation_count += duplicate_mutations;
        duplicate_repair_count += duplicate_repairs;
        assert!(accepts(&sample, shape));
        assert!(defects(&sample, shape).is_empty());
        let mut paths = vec![String::new()];
        pointers(&sample, "", &mut paths);
        let mut independent = Vec::new();
        for pointer in paths {
            let mut variants = Vec::new();
            let mut missing = sample.clone();
            if remove(&mut missing, &pointer) {
                variants.push(missing);
            }
            for probe in &probes {
                let mut changed = sample.clone();
                *changed.pointer_mut(&pointer).unwrap() = probe.clone();
                variants.push(changed);
            }
            if sample.pointer(&pointer).unwrap().is_object() {
                for key in ["unknown_a", "unknown_b"] {
                    let mut changed = sample.clone();
                    changed.pointer_mut(&pointer).unwrap()[key] = json!(0);
                    variants.push(changed);
                }
            }
            // Unit enums have both string and externally tagged map forms.
            if let Some(name) =
                sample.pointer(&pointer).unwrap().as_str().filter(|name| {
                    ["project_root", "repo_root", "accepted", "refuted"].contains(name)
                })
            {
                for payload in [Value::Null, json!({}), json!(false)] {
                    let mut changed = sample.clone();
                    *changed.pointer_mut(&pointer).unwrap() = json!({name: payload});
                    variants.push(changed);
                }
                let mut changed = sample.clone();
                *changed.pointer_mut(&pointer).unwrap() = json!({name: null, "unknown": null});
                variants.push(changed);
            }
            if let Some(items) = sample.pointer(&pointer).unwrap().as_array() {
                for length in 0..items.len() {
                    let mut changed = sample.clone();
                    changed
                        .pointer_mut(&pointer)
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                        .truncate(length);
                    variants.push(changed);
                }
                let mut changed = sample.clone();
                changed
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_array_mut()
                    .unwrap()
                    .extend([Value::Null, Value::Null]);
                variants.push(changed);
            }
            let mut independent_added = false;
            for changed in variants {
                mutations += 1;
                let names = defects(&changed, shape);
                let accepted = accepts(&changed, shape);
                if names.is_empty() != accepted && failures.len() < 12 {
                    failures.push(format!("completeness {pointer}: serde={accepted}, defects={names:?}, changed={changed}"));
                }
                if !accepted {
                    sequences += 1;
                    // Restoring only an invalid container's type must also
                    // decrease, even when its required leaves remain missing.
                    let original = sample.pointer(&pointer).unwrap();
                    let changed_value = changed.pointer(&pointer);
                    let empty = if original.is_object()
                        && !changed_value.is_some_and(Value::is_object)
                    {
                        Some(json!({}))
                    } else if original.is_array() && !changed_value.is_some_and(Value::is_array) {
                        Some(json!([]))
                    } else {
                        None
                    };
                    // Objects and positional arrays can both be valid struct
                    // containers. Changing between those representations does
                    // not repair a defect unless this container was reported.
                    if let Some(empty) = empty.filter(|_| {
                        names
                            .iter()
                            .any(|name| name == pointer.trim_start_matches('/'))
                    }) {
                        let mut partial = changed.clone();
                        if let Some(target) = partial.pointer_mut(&pointer) {
                            *target = empty;
                        } else {
                            let (parent, key) = pointer.rsplit_once('/').unwrap();
                            partial.pointer_mut(parent).unwrap()[key] = empty;
                        }
                        sequences += 1;
                        let after = defects(&partial, shape).len();
                        if after >= names.len() && failures.len() < 12 {
                            failures.push(format!(
                                "container repair {pointer}: {} -> {after}",
                                names.len()
                            ));
                        }
                        if after == 0 && !accepts(&partial, shape) && failures.len() < 12 {
                            failures.push(format!("container zero must assemble {pointer}"));
                        }
                    }
                    if names.is_empty() && failures.len() < 12 {
                        failures.push(format!("repair {pointer}: 0 -> 0"));
                    }
                    if !independent_added
                        && sample
                            .pointer(&pointer)
                            .is_some_and(|v| !v.is_object() && !v.is_array())
                    {
                        independent.push((pointer.clone(), changed.pointer(&pointer).cloned()));
                        independent_added = true;
                    }
                }
            }
        }
        // Reachable inputs with many simultaneous faults: every possible next
        // leaf repair must decrease, not just one convenient repair ordering.
        let mut broken = sample.clone();
        for (pointer, value) in &independent {
            if let Some(value) = value {
                *broken.pointer_mut(pointer).unwrap() = value.clone();
            } else {
                remove(&mut broken, pointer);
            }
        }
        let broken_count = defects(&broken, shape).len();
        for (pointer, _) in &independent {
            let mut repaired = broken.clone();
            restore(&mut repaired, &sample, pointer);
            sequences += 1;
            let b = broken_count;
            let a = defects(&repaired, shape).len();
            if a >= b && failures.len() < 12 {
                failures.push(format!("combined repair {pointer}: {b} -> {a}"));
            }
        }
        let mut before_count = broken_count;
        for (pointer, _) in independent {
            let mut repaired = broken.clone();
            restore(&mut repaired, &sample, &pointer);
            sequences += 1;
            let b = before_count;
            let a = defects(&repaired, shape).len();
            if a >= b && failures.len() < 12 {
                failures.push(format!("sequence {pointer}: {b} -> {a}"));
            }
            before_count = a;
            broken = repaired;
        }
        assert!(accepts(&broken, shape));
        // Multiple denied unknown fields must not collapse into serde's first error.
        let mut paths = vec![String::new()];
        pointers(&sample, "", &mut paths);
        for pointer in paths {
            if sample.pointer(&pointer).unwrap().is_array() {
                let mut changed = sample.clone();
                changed
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_array_mut()
                    .unwrap()
                    .extend([Value::Null, Value::Null]);
                if !accepts(&changed, shape) {
                    for _ in 0..2 {
                        let mut repaired = changed.clone();
                        repaired
                            .pointer_mut(&pointer)
                            .unwrap()
                            .as_array_mut()
                            .unwrap()
                            .pop();
                        sequences += 1;
                        let b = defects(&changed, shape).len();
                        let a = defects(&repaired, shape).len();
                        if a >= b && failures.len() < 12 {
                            failures.push(format!("extra element repair {pointer}: {b} -> {a}"));
                        }
                        changed = repaired;
                    }
                }
            }
            if !sample.pointer(&pointer).unwrap().is_object() {
                continue;
            }
            let mut changed = sample.clone();
            changed.pointer_mut(&pointer).unwrap()["unknown_a"] = json!(0);
            changed.pointer_mut(&pointer).unwrap()["unknown_b"] = json!(0);
            if accepts(&changed, shape) {
                continue;
            }
            for key in ["unknown_a", "unknown_b"] {
                let mut repaired = changed.clone();
                repaired
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
                sequences += 1;
                let b = defects(&changed, shape).len();
                let a = defects(&repaired, shape).len();
                if a >= b && failures.len() < 12 {
                    failures.push(format!("unknown repair {pointer}/{key}: {b} -> {a}"));
                }
                changed = repaired;
            }
        }
    }
    eprintln!(
        "round12 corpus (exhaustive={exhaustive}): {mutations} mutations, {sequences} repair checks"
    );
    eprintln!(
        "duplicates: {duplicate_mutation_count} mutations, {duplicate_repair_count} repair checks"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
