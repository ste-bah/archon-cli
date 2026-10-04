//! Generate raw duplicate keys at every object-field pointer. Value cannot
//! represent these mutations, so serialize them explicitly before prechecking.
use super::*;

/// Encode `value`, writing each field as the copies `copies` returns for its
/// pointer (one copy keeps the field, two or more duplicate its key).
fn encode_with(value: &Value, at: &str, copies: &dyn Fn(&str, &Value) -> Vec<Value>) -> String {
    match value {
        Value::Object(map) => {
            let fields: Vec<_> = map
                .iter()
                .flat_map(|(key, child)| {
                    let path = format!("{at}/{}", key.replace('~', "~0").replace('/', "~1"));
                    copies(&path, child)
                        .iter()
                        .map(|copy| format!("{}:{}", json!(key), encode_with(copy, &path, copies)))
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
    encode_with(value, at, &|path, child| {
        vec![child.clone(); 1 + usize::from(duplicates.iter().any(|p| p == path))]
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

/// Up to two values the reader refuses at `path` when written once.
fn invalid_values(sample: &Value, path: &str, shape: &ElementShape) -> Vec<Value> {
    [
        json!(false),
        json!(0),
        json!("invalid-enum"),
        json!({}),
        json!([]),
    ]
    .into_iter()
    .filter(|probe| {
        let mut changed = sample.clone();
        *changed.pointer_mut(path).unwrap() = probe.clone();
        !raw_accepts(&serde_json::to_vec(&changed).unwrap(), shape)
    })
    .take(2)
    .collect()
}

/// Write two different copies at `path`: invalid+invalid, valid+invalid,
/// invalid+valid, valid+valid. Repairing one copy never raises the count, and
/// must lower it whenever the reader's verdict changes.
fn mixed_copies(sample: &Value, path: &str, shape: &ElementShape) -> (usize, usize) {
    let invalid = invalid_values(sample, path, shape);
    let (Some(first), Some(second)) = (invalid.first(), invalid.last()) else {
        return (0, 0);
    };
    let valid = sample.pointer(path).unwrap();
    let judge = |a: &Value, b: &Value| {
        let bytes = encode_with(sample, "", &|at, child| {
            if at == path {
                vec![a.clone(), b.clone()]
            } else {
                vec![child.clone()]
            }
        });
        let defects = element_shape_defects(bytes.as_bytes(), shape);
        let outcome = raw_outcome(bytes.as_bytes(), shape);
        assert_eq!(
            defects.is_empty(),
            outcome.is_ok(),
            "copies {a} then {b} at {path}: {defects:?}"
        );
        (defects.len(), outcome)
    };
    let both = judge(first, second);
    let fixed_first = judge(valid, second);
    let fixed_second = judge(first, valid);
    let fixed = judge(valid, valid);
    for (before, after) in [
        (&both, &fixed_first),
        (&both, &fixed_second),
        (&fixed_first, &fixed),
        (&fixed_second, &fixed),
    ] {
        assert!(
            after.0 <= before.0 && (after.1 == before.1 || after.0 < before.0),
            "repairing one copy at {path}: {} -> {} defects; reader {:?} -> {:?}",
            before.0,
            after.0,
            before.1,
            after.1
        );
    }
    (3, 4)
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
        let (mixed, repairs) = mixed_copies(sample, &path, shape);
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
