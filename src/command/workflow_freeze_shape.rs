//! The required-field table of each candidate element, walked on the JSON
//! before serde (Issue 261). serde stops at an element's first error and
//! would hide the rest; the table names every missing or invalid required
//! field by its JSON pointer, so repairing fields one per attempt lowers the
//! defect count. The table is the one declared schema the precheck reads, and
//! a drift guard (workflow_freeze_shape_tests.rs) proves serde requires
//! exactly the fields it names: for every field of a complete instance of
//! each serde type, removing it or writing an undeclared value into it is
//! refused by serde exactly when the table reports it.
use crate::command::workflow_freeze_candidate::{candidate_document, judgment_placeholder};
use archon_workflow::defect::ValidationDefect;
use serde_json::{Map, Value};

/// The fields one JSON object must carry, as serde reads it.
pub(crate) struct ObjectShape {
    /// Required string fields.
    pub(crate) strings: &'static [&'static str],
    /// Required string fields with a closed vocabulary.
    pub(crate) choices: &'static [(&'static str, &'static [&'static str])],
    /// Required objects. A missing one reports the fields it requires.
    pub(crate) objects: &'static [(&'static str, &'static ObjectShape)],
    /// Optional lists of objects.
    pub(crate) lists: &'static [(&'static str, &'static ObjectShape)],
    /// An internally tagged union: the tag field, and the fields each of its
    /// values requires beside the tag.
    pub(crate) tagged: Option<(
        &'static str,
        &'static [(&'static str, &'static ObjectShape)],
    )>,
}

const OBJECT: ObjectShape = ObjectShape {
    strings: &[],
    choices: &[],
    objects: &[],
    lists: &[],
    tagged: None,
};

/// One kind of candidate element: the lists it appears in, its table, and
/// serde's own read of it, run when the table passes an element.
pub(crate) struct ElementShape {
    pub(crate) lists: &'static [&'static str],
    pub(crate) element: &'static ObjectShape,
    pub(crate) serde_read: fn(&Value) -> Result<(), String>,
}

const DELIVERABLE_CONTRACT: ObjectShape = ObjectShape {
    strings: &["kind", "artifact_path"],
    ..OBJECT
};

const CONSUMED_ARTIFACT: ObjectShape = ObjectShape {
    strings: &["artifact_path"],
    ..OBJECT
};

const DEPENDENCY: ObjectShape = ObjectShape {
    strings: &["task_id"],
    lists: &[("consumes", &CONSUMED_ARTIFACT)],
    ..OBJECT
};

const TASK: ObjectShape = ObjectShape {
    strings: &["task_id", "file_name"],
    lists: &[
        ("depends_on", &DEPENDENCY),
        ("deliverable_contracts", &DELIVERABLE_CONTRACT),
    ],
    ..OBJECT
};

const COMMAND_CHECK: ObjectShape = ObjectShape {
    strings: &["command"],
    choices: &[("cwd", &["project_root", "repo_root"])],
    ..OBJECT
};

const FLOOR_CHECK: ObjectShape = ObjectShape {
    objects: &[("contract", &DELIVERABLE_CONTRACT)],
    ..OBJECT
};

const CHECK: ObjectShape = ObjectShape {
    tagged: Some((
        "kind",
        &[("command", &COMMAND_CHECK), ("floor", &FLOOR_CHECK)],
    )),
    ..OBJECT
};

/// `judgment` is host-owned and stamped later, so it is not authored here.
const ENTRY: ObjectShape = ObjectShape {
    strings: &["id", "criterion"],
    objects: &[("check", &CHECK)],
    ..OBJECT
};

pub(crate) const TASK_SHAPE: ElementShape = ElementShape {
    lists: &["tasks"],
    element: &TASK,
    serde_read: |item| read::<archon_workflow::task_skeleton::FrozenTask>(item.clone()),
};

/// Authored acceptance entries, read as assembly reads them: with the host's
/// judgment placeholder stamped over whatever the author wrote there.
pub(crate) const ENTRY_SHAPE: ElementShape = ElementShape {
    lists: &["entries", "supplementary", "acceptance"],
    element: &ENTRY,
    serde_read: |item| {
        let mut item = item.clone();
        if let Some(object) = item.as_object_mut() {
            object.insert("judgment".into(), judgment_placeholder());
        }
        read::<archon_workflow::task_set_contract::AcceptanceCriterion>(item)
    },
};

fn read<T: serde::de::DeserializeOwned>(item: Value) -> Result<(), String> {
    serde_json::from_value::<T>(item)
        .map(drop)
        .map_err(|error| error.to_string())
}

fn shape_defect(pointer: String, problem: &str) -> ValidationDefect {
    let message = format!("{pointer} {problem}; write it exactly as the required shape names it");
    ValidationDefect::new("invalid_candidate_shape", &pointer, "shape", message)
}

impl ObjectShape {
    fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        let keyed = |list: &'static [(&'static str, &'static ObjectShape)]| {
            list.iter().map(|(name, _)| *name)
        };
        (self.strings.iter().copied())
            .chain(self.choices.iter().map(|(name, _)| *name))
            .chain(keyed(self.objects))
            .chain(keyed(self.lists))
    }

    /// Every defect of `object` (`None` when it is missing) at `at`.
    fn walk(&self, object: Option<&Map<String, Value>>, at: &str, out: &mut Vec<ValidationDefect>) {
        let get = |field: &str| object.and_then(|object| object.get(field));
        for field in self.strings {
            if !get(field).is_some_and(Value::is_string) {
                out.push(shape_defect(
                    format!("{at}/{field}"),
                    "is missing or not a string",
                ));
            }
        }
        for (field, values) in self.choices {
            if !get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| values.contains(&value))
            {
                let problem = format!("is missing or not one of: {}", values.join(", "));
                out.push(shape_defect(format!("{at}/{field}"), &problem));
            }
        }
        for (field, shape) in self.objects {
            let pointer = format!("{at}/{field}");
            match get(field) {
                None => shape.walk(None, &pointer, out),
                Some(Value::Object(inner)) => shape.walk(Some(inner), &pointer, out),
                Some(_) => out.push(shape_defect(pointer, "is not an object")),
            }
        }
        for (field, shape) in self.lists {
            let pointer = format!("{at}/{field}");
            match get(field) {
                None => {}
                Some(Value::Array(items)) => shape.walk_items(items, &pointer, out),
                Some(_) => out.push(shape_defect(pointer, "is not a list")),
            }
        }
        if let Some((tag, variants)) = self.tagged {
            let named = get(tag)
                .and_then(Value::as_str)
                .and_then(|value| variants.iter().find(|(name, _)| *name == value));
            if named.is_none() {
                let names: Vec<_> = variants.iter().map(|(name, _)| *name).collect();
                let problem = format!("is missing or not one of: {}", names.join(", "));
                out.push(shape_defect(format!("{at}/{tag}"), &problem));
            }
            // Without a valid tag, the one variant whose fields are present
            // still has its other fields checked, so naming the tag later
            // does not reveal defects this attempt could have reported.
            let present = |shape: &ObjectShape| shape.names().any(|name| get(name).is_some());
            let mut inferred = variants.iter().filter(|(_, shape)| present(shape));
            let variant = named.or_else(|| match (inferred.next(), inferred.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            });
            if let Some((_, shape)) = variant {
                shape.walk(object, at, out);
            }
        }
    }

    fn walk_items(&self, items: &[Value], at: &str, out: &mut Vec<ValidationDefect>) {
        for (index, item) in items.iter().enumerate() {
            out.extend(table_defects(item, &format!("{at}/{index}"), self));
        }
    }
}

/// Every table defect of one element at `at`, without serde's read.
pub(crate) fn table_defects(item: &Value, at: &str, shape: &ObjectShape) -> Vec<ValidationDefect> {
    let mut out = Vec::new();
    match item {
        Value::Object(object) => shape.walk(Some(object), at, &mut out),
        _ => out.push(shape_defect(at.to_string(), "is not an object")),
    }
    out
}

/// Every element shape defect of `candidate` under `shape`.
pub(crate) fn element_shape_defects(
    candidate: &[u8],
    shape: &ElementShape,
) -> Vec<ValidationDefect> {
    let document = candidate_document(candidate);
    let Ok(value) = serde_json::from_slice::<Value>(&document) else {
        return Vec::new();
    };
    let mut defects = Vec::new();
    for list in shape.lists {
        let items = value.get(*list).and_then(Value::as_array);
        for (index, item) in items.into_iter().flatten().enumerate() {
            let at = format!("{list}/{index}");
            let mut found = table_defects(item, &at, shape.element);
            if found.is_empty()
                && let Err(error) = (shape.serde_read)(item)
            {
                found.push(shape_defect(
                    at,
                    &format!("does not match the required shape ({error})"),
                ));
            }
            defects.extend(found);
        }
    }
    defects
}
