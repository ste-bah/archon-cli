//! Generate raw duplicate keys at every object-field pointer. Value cannot
//! represent these mutations, so serialize them explicitly before prechecking.
use super::*;
use std::collections::HashMap;

/// A field's copies as written: (key, value) pairs.
type Copies = Vec<(String, Value)>;

/// Encode `value`, writing each field as the copies `copies` returns for its
/// pointer and key (one copy keeps the field, more copies repeat it).
fn encode_with(value: &Value, at: &str, copies: &dyn Fn(&str, &str, &Value) -> Copies) -> String {
    match value {
        Value::Object(map) => {
            let fields: Vec<_> = map
                .iter()
                .flat_map(|(key, child)| {
                    let path = format!("{at}/{}", key.replace('~', "~0").replace('/', "~1"));
                    copies(&path, key, child)
                        .iter()
                        .map(|(name, copy)| {
                            format!("{}:{}", json!(name), encode_with(copy, &path, copies))
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let fields: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(i, child)| encode_with(child, &format!("{at}/{i}"), copies))
                .collect();
            format!("[{}]", fields.join(","))
        }
        _ => value.to_string(),
    }
}

fn encode(value: &Value, at: &str, duplicates: &[String]) -> String {
    encode_with(value, at, &|path, key, child| {
        let copy = (key.to_string(), child.clone());
        vec![copy; 1 + usize::from(duplicates.iter().any(|p| p == path))]
    })
}

/// The reader's verdict, without the position (copies differ in length).
fn raw_outcome(bytes: &[u8], shape: &ElementShape) -> Result<(), String> {
    let text = |error: serde_json::Error| {
        let text = error.to_string();
        text.split(" at line ")
            .next()
            .unwrap_or_default()
            .to_string()
    };
    if matches!(shape, ElementShape::Tasks) {
        serde_json::from_slice::<TaskSkeleton>(bytes)
            .map(drop)
            .map_err(text)
    } else {
        let bytes = acceptance_candidate_for_validation(bytes).map_err(|e| e.to_string())?;
        serde_json::from_slice::<AcceptanceContract>(&bytes)
            .map(drop)
            .map_err(text)
    }
}

fn raw_accepts(bytes: &[u8], shape: &ElementShape) -> bool {
    raw_outcome(bytes, shape).is_ok()
}

/// A value the reader refuses at `path` when written once, if any.
fn invalid_value(sample: &Value, path: &str, shape: &ElementShape) -> Option<Value> {
    [
        json!(false),
        json!(0),
        json!("invalid-enum"),
        json!({}),
        json!([]),
    ]
    .into_iter()
    .find(|probe| {
        let mut changed = sample.clone();
        *changed.pointer_mut(path).unwrap() = probe.clone();
        !raw_accepts(&serde_json::to_vec(&changed).unwrap(), shape)
    })
}

/// Each copy of the field at `path`: (renamed to an ignored key, invalid).
type State = Vec<(bool, bool)>;

/// Write three copies at `path` in every valid/invalid mix, then delete,
/// rename (where the parent ignores unknown keys) or repair each copy, down
/// to one copy. A change that alters the reader's verdict must change the
/// count; a repair never raises it; where the parent refuses repeats, every
/// deletion or rename lowers it. Returns (states, transitions) checked.
fn copy_repairs(sample: &Value, path: &str, shape: &ElementShape) -> (usize, usize) {
    let Some(invalid) = invalid_value(sample, path, shape) else {
        return (0, 0);
    };
    let valid = sample.pointer(path).unwrap();
    let renamed = "renamed_copy";
    let judge = |state: &State| {
        let bytes = encode_with(sample, "", &|at, key, child| {
            if at != path {
                return vec![(key.to_string(), child.clone())];
            }
            let name = |r: bool| if r { renamed } else { key }.to_string();
            let value = |bad: bool| if bad { &invalid } else { valid }.clone();
            state
                .iter()
                .map(|&(r, bad)| (name(r), value(bad)))
                .collect()
        });
        let defects = element_shape_defects(bytes.as_bytes(), shape);
        let outcome = raw_outcome(bytes.as_bytes(), shape);
        assert_eq!(defects.is_empty(), outcome.is_ok(), "{bytes}: {defects:?}");
        (defects.len(), outcome)
    };
    let mut open = sample.clone();
    let (parent, _) = path.rsplit_once('/').unwrap();
    open.pointer_mut(parent).unwrap()[renamed] = json!(0);
    let open = raw_accepts(&serde_json::to_vec(&open).unwrap(), shape);
    let refuses_repeats = judge(&vec![(false, false); 2]).1.is_err();
    let mut seen = HashMap::new();
    let mut work: Vec<State> = (0..8)
        .map(|bits| (0..3).map(|i| (false, bits >> i & 1 == 1)).collect())
        .collect();
    let mut transitions = 0;
    while let Some(state) = work.pop() {
        if seen.contains_key(&state) {
            continue;
        }
        let before = judge(&state);
        for i in 0..state.len() {
            let mut steps = Vec::new();
            if state.len() > 1 {
                let mut next = state.clone();
                next.remove(i);
                steps.push(("delete", next));
                if open {
                    let mut next = state.clone();
                    next[i].0 = true;
                    steps.push(("rename", next));
                }
            }
            if state[i].1 {
                let mut next = state.clone();
                next[i].1 = false;
                steps.push(("repair", next));
            }
            for (step, next) in steps {
                let after = seen.get(&next).cloned().unwrap_or_else(|| judge(&next));
                let changed = after.1 != before.1;
                let lowered = after.0 < before.0;
                let ok = match step {
                    "repair" => after.0 <= before.0 && (!changed || lowered),
                    _ => (!changed || after.0 != before.0) && (!refuses_repeats || lowered),
                };
                assert!(
                    ok,
                    "{step} copy {} of {state:?} at {path}: {} -> {} defects; reader {:?} -> {:?}",
                    i + 1,
                    before.0,
                    after.0,
                    before.1,
                    after.1
                );
                if step != "rename" {
                    work.push(next);
                }
                transitions += 1;
            }
        }
        seen.insert(state, before);
    }
    (seen.len(), transitions)
}

pub(super) fn check(sample: &Value, shape: &ElementShape) -> (usize, usize) {
    let mut paths = Vec::new();
    pointers(sample, "", &mut paths);
    let mut rejected = Vec::new();
    let mut mutations = 0;
    let mut mixed_repairs = 0;
    for path in paths {
        let (parent, _) = path.rsplit_once('/').unwrap();
        if !sample.pointer(parent).unwrap().is_object() {
            continue;
        }
        let (mixed, repairs) = copy_repairs(sample, &path, shape);
        mutations += mixed;
        mixed_repairs += repairs;
        let bytes = encode(sample, "", std::slice::from_ref(&path));
        let defects = element_shape_defects(bytes.as_bytes(), shape);
        let accepted = raw_accepts(bytes.as_bytes(), shape);
        assert_eq!(
            defects.is_empty(),
            accepted,
            "duplicate {path}: {defects:?}"
        );
        if !accepted {
            assert_eq!(defects.len(), 1, "duplicate {path}: {defects:?}");
            assert_eq!(defects[0].identity.subject, &path[1..]);
            rejected.push(path);
        }
        mutations += 1;
    }
    // Many simultaneous duplicates: repairing any single pointer, then all
    // pointers in order, must strictly lower the count and end in assembly.
    // Each copy is read, so a duplicate inside a duplicated parent counts once
    // per parent copy.
    let expected = |set: &[String]| -> usize {
        set.iter()
            .map(|path| {
                let parents = set
                    .iter()
                    .filter(|parent| path.starts_with(&format!("{parent}/")))
                    .count();
                1 << parents
            })
            .sum()
    };
    let before = encode(sample, "", &rejected);
    assert_eq!(
        element_shape_defects(before.as_bytes(), shape).len(),
        expected(&rejected)
    );
    let mut repairs = mutations + mixed_repairs;
    for path in &rejected {
        let remaining: Vec<_> = rejected.iter().filter(|p| *p != path).cloned().collect();
        let bytes = encode(sample, "", &remaining);
        let count = element_shape_defects(bytes.as_bytes(), shape).len();
        assert_eq!(count, expected(&remaining));
        assert!(count < expected(&rejected), "repair {path}");
        repairs += 1;
    }
    while !rejected.is_empty() {
        let count = expected(&rejected);
        rejected.pop();
        let bytes = encode(sample, "", &rejected);
        let after = element_shape_defects(bytes.as_bytes(), shape).len();
        assert_eq!(after, expected(&rejected));
        assert!(after < count);
        assert_eq!(raw_accepts(bytes.as_bytes(), shape), rejected.is_empty());
        repairs += 1;
    }
    (mutations, repairs)
}
