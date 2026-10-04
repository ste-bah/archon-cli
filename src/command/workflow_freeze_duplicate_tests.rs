//! Generate raw duplicate keys at every object-field pointer. Value cannot
//! represent these mutations, so serialize them explicitly before prechecking.
use super::*;

fn encode(value: &Value, at: &str, duplicates: &[String]) -> String {
    match value {
        Value::Object(map) => {
            let fields: Vec<_> = map
                .iter()
                .flat_map(|(key, child)| {
                    let path = format!("{at}/{}", key.replace('~', "~0").replace('/', "~1"));
                    let field = format!("{}:{}", json!(key), encode(child, &path, duplicates));
                    let copies = 1 + usize::from(duplicates.contains(&path));
                    vec![field; copies]
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let fields: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(i, child)| encode(child, &format!("{at}/{i}"), duplicates))
                .collect();
            format!("[{}]", fields.join(","))
        }
        _ => value.to_string(),
    }
}

fn raw_accepts(bytes: &[u8], shape: &ElementShape) -> bool {
    if matches!(shape, ElementShape::Tasks) {
        serde_json::from_slice::<TaskSkeleton>(bytes).is_ok()
    } else {
        acceptance_candidate_for_validation(bytes)
            .ok()
            .is_some_and(|b| serde_json::from_slice::<AcceptanceContract>(&b).is_ok())
    }
}

pub(super) fn check(sample: &Value, shape: &ElementShape) -> (usize, usize) {
    let mut paths = Vec::new();
    pointers(sample, "", &mut paths);
    let mut rejected = Vec::new();
    let mut mutations = 0;
    for path in paths {
        let (parent, _) = path.rsplit_once('/').unwrap();
        if !sample.pointer(parent).unwrap().is_object() {
            continue;
        }
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
    let before = encode(sample, "", &rejected);
    assert_eq!(
        element_shape_defects(before.as_bytes(), shape).len(),
        rejected.len()
    );
    let mut repairs = mutations;
    for path in &rejected {
        let remaining: Vec<_> = rejected.iter().filter(|p| *p != path).cloned().collect();
        let bytes = encode(sample, "", &remaining);
        assert_eq!(
            element_shape_defects(bytes.as_bytes(), shape).len(),
            rejected.len() - 1
        );
        repairs += 1;
    }
    while !rejected.is_empty() {
        rejected.pop();
        let bytes = encode(sample, "", &rejected);
        assert_eq!(
            element_shape_defects(bytes.as_bytes(), shape).len(),
            rejected.len()
        );
        assert_eq!(raw_accepts(bytes.as_bytes(), shape), rejected.is_empty());
        repairs += 1;
    }
    (mutations, repairs)
}
