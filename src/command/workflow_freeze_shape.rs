//! Report every serde shape defect before assembly. No first-error serde
//! fallback: the exhaustive mutation corpus guards the complete field schema.
use crate::command::workflow_freeze_candidate::candidate_document;
use archon_workflow::defect::ValidationDefect;
use serde_json::{Map, Value};

#[path = "workflow_freeze_schema.rs"]
mod schema;

#[path = "workflow_freeze_shape_parse.rs"]
mod parse;

#[derive(Clone, Copy)]
enum Shape {
    String,
    Bool,
    Unsigned(u64),
    Any,
    Choice(&'static [&'static str], bool),
    Nullable(&'static Shape),
    List(&'static Shape),
    Map(&'static Shape),
    Object(&'static ObjectShape),
    Tagged(
        &'static str,
        &'static [(&'static str, &'static ObjectShape)],
    ),
}
struct Field {
    name: &'static str,
    required: bool,
    shape: Shape,
}
struct ObjectShape {
    fields: &'static [Field],
    deny_unknown: bool,
    sequence_override: Option<&'static ObjectShape>,
}

pub(crate) enum ElementShape {
    Tasks,
    Acceptance,
}
pub(crate) const TASK_SHAPE: ElementShape = ElementShape::Tasks;
pub(crate) const ENTRY_SHAPE: ElementShape = ElementShape::Acceptance;

fn pointer(at: &str, field: &str) -> String {
    let field = field.replace('~', "~0").replace('/', "~1");
    if at.is_empty() {
        field
    } else {
        format!("{at}/{field}")
    }
}
fn shape_defect(at: &str, problem: &str) -> ValidationDefect {
    ValidationDefect::new(
        "invalid_candidate_shape",
        at,
        "shape",
        format!("{at} {problem}"),
    )
}

impl ObjectShape {
    fn walk(
        &self,
        value: Option<&Value>,
        at: &str,
        allowed: &[&str],
        out: &mut Vec<ValidationDefect>,
    ) {
        self.walk_parts(
            value.and_then(Value::as_object),
            value.and_then(Value::as_array).map(Vec::as_slice),
            at,
            allowed,
            0,
            None,
            out,
        );
    }

    fn walk_parts(
        &self,
        object: Option<&Map<String, Value>>,
        items: Option<&[Value]>,
        at: &str,
        allowed: &[&str],
        offset: usize,
        extra_start: Option<usize>,
        out: &mut Vec<ValidationDefect>,
    ) {
        // An invalid authored entry can be repaired into either representation.
        // Reserve the positional representation's required judgment before that
        // repair; only an actual object is guaranteed to be stamped by assembly.
        let shape = if object.is_none() {
            self.sequence_override.unwrap_or(self)
        } else {
            self
        };
        for (index, field) in shape.fields.iter().enumerate() {
            let value = object
                .and_then(|map| map.get(field.name))
                .or_else(|| items.and_then(|items| items.get(index)));
            if value.is_some() || field.required {
                let key = if items.is_some() {
                    (index + offset).to_string()
                } else {
                    field.name.to_string()
                };
                field.shape.walk(value, &pointer(at, &key), out);
            }
        }
        if let Some(items) = items {
            for index in extra_start.unwrap_or(shape.fields.len())..items.len() {
                out.push(shape_defect(
                    &pointer(at, &(index + offset).to_string()),
                    "is an extra positional field; remove it",
                ));
            }
        }
        if self.deny_unknown
            && let Some(object) = object
        {
            for key in object.keys() {
                if !allowed.contains(&key.as_str()) && !self.fields.iter().any(|f| f.name == key) {
                    out.push(shape_defect(
                        &pointer(at, key),
                        "is an unknown field; remove it",
                    ));
                }
            }
        }
    }
}
impl Shape {
    fn walk(&self, value: Option<&Value>, at: &str, out: &mut Vec<ValidationDefect>) {
        let valid = match self {
            Self::String => value.is_some_and(Value::is_string),
            Self::Bool => value.is_some_and(Value::is_boolean),
            Self::Unsigned(max) => value.and_then(Value::as_u64).is_some_and(|n| n <= *max),
            Self::Any => value.is_some(),
            // Internally tagged checks use serde's buffered ContentDeserializer,
            // whose unit reader also accepts an empty map. Direct enum reads do not.
            Self::Choice(choices, buffered) => value.is_some_and(|value| {
                value.as_str().is_some_and(|s| choices.contains(&s))
                    || value.as_object().is_some_and(|map| {
                        map.len() == 1
                            && map.iter().any(|(name, payload)| {
                                choices.contains(&name.as_str())
                                    && (payload.is_null()
                                        || (*buffered
                                            && payload
                                                .as_object()
                                                .is_some_and(|map| map.is_empty())))
                            })
                    })
            }),
            Self::Nullable(inner) => {
                if value.is_some_and(Value::is_null) {
                    return;
                }
                inner.walk(value, at, out);
                return;
            }
            Self::List(inner) => {
                if let Some(Value::Array(items)) = value {
                    for (index, item) in items.iter().enumerate() {
                        inner.walk(Some(item), &pointer(at, &index.to_string()), out);
                    }
                    return;
                }
                false
            }
            Self::Map(inner) => {
                if let Some(Value::Object(items)) = value {
                    for (key, item) in items {
                        inner.walk(Some(item), &pointer(at, key), out);
                    }
                    return;
                }
                false
            }
            Self::Object(shape) => {
                let object = value.and_then(Value::as_object);
                shape.walk(value, at, &[], out);
                // A missing or mistyped object has its container defect AND every required
                // leaf. Restoring {} removes the container defect; restoring a
                // complete object removes all of them. Missing and invalid containers
                // use the same identities, including every required leaf.
                if object.is_none() && !value.is_some_and(Value::is_array) {
                    out.push(shape_defect(
                        at,
                        "is missing or not a struct object or sequence",
                    ));
                }
                return;
            }
            Self::Tagged(tag, variants) => {
                walk_tagged(value, at, tag, variants, out);
                return;
            }
        };
        if !valid {
            out.push(shape_defect(
                at,
                "is missing or has an invalid type or value",
            ));
        }
    }
}

fn walk_tagged(
    value: Option<&Value>,
    at: &str,
    tag: &str,
    variants: &[(&str, &ObjectShape)],
    out: &mut Vec<ValidationDefect>,
) {
    let start = out.len();
    let object = value.and_then(Value::as_object);
    let items = value.and_then(Value::as_array);
    let rest = items.map(|items| items.get(1..).unwrap_or_default());
    let tag_at = pointer(at, if items.is_some() { "0" } else { tag });
    let name = object
        .and_then(|o| o.get(tag))
        .or_else(|| items.and_then(|items| items.first()))
        .and_then(Value::as_str);
    if let Some((_, shape)) = variants.iter().find(|(kind, _)| Some(*kind) == name) {
        shape.walk_parts(
            object,
            rest,
            at,
            &[tag],
            usize::from(items.is_some()),
            None,
            out,
        );
        return;
    }
    out.push(shape_defect(&tag_at, "is missing or not a known variant"));
    if object.is_none() && items.is_none() {
        out.push(shape_defect(at, "is not a tagged object or sequence"));
    }
    let mut allowed = vec![tag];
    for (_, shape) in variants {
        allowed.extend(shape.fields.iter().map(|f| f.name));
    }
    allowed.sort_unstable();
    allowed.dedup();
    let max_fields = variants
        .iter()
        .map(|(_, shape)| shape.fields.len())
        .max()
        .unwrap_or(0);
    // Before a tag is resolved, every variant's required leaves are reachable.
    // Reserve possible forbidden fields, even when absent: selecting ANY tag
    // then removes at least the tag defect, and cannot reveal unknown fields.
    // Positional variants can share a slot with different types, so their leaf
    // identities stay distinct until the tag selects one interpretation.
    for (kind, shape) in variants {
        let start = out.len();
        shape.walk_parts(
            object,
            rest,
            at,
            &allowed,
            usize::from(items.is_some()),
            rest.map(|_| max_fields),
            out,
        );
        if items.is_some() {
            for defect in &mut out[start..] {
                if (0..shape.fields.len()).any(|index| {
                    let prefix = pointer(at, &(index + 1).to_string());
                    defect.identity.subject == prefix
                        || defect.identity.subject.starts_with(&format!("{prefix}/"))
                }) {
                    defect.identity.location =
                        format!("shape/variant/{kind}/{}", defect.identity.location);
                }
            }
        }
        if shape.deny_unknown {
            let forbidden: Vec<_> = if items.is_some() {
                (shape.fields.len()..max_fields)
                    .map(|index| pointer(at, &(index + 1).to_string()))
                    .collect()
            } else {
                allowed
                    .iter()
                    .filter(|field| {
                        **field != tag && !shape.fields.iter().any(|f| f.name == **field)
                    })
                    .map(|field| pointer(at, field))
                    .collect()
            };
            for at in forbidden {
                let mut defect = shape_defect(
                    &at,
                    &format!("would be forbidden by {tag}={kind}; resolve {tag} first"),
                );
                defect.identity.location = format!("shape/forbidden_if/{kind}");
                out.push(defect);
            }
        }
    }
    // Keep the reserved identities/counts until the tag is known, but never
    // instruct the author to add a different variant's required fields.
    let choices = variants
        .iter()
        .map(|(kind, _)| *kind)
        .collect::<Vec<_>>()
        .join(", ");
    for defect in &mut out[start..] {
        defect.message = format!(
            "{}: resolve {tag_at} first (choices: {choices}); requirements depend on the selected variant",
            defect.identity.subject
        );
    }
}

/// The tasks document as the derived skeleton reader reads it (Issue 312):
/// every read field in full, and every field the reader ignores skipped the
/// way serde skips it and kept as `null`. Inspections of the same candidate
/// read this, so they accept exactly the documents the reader accepts.
pub(crate) fn skeleton_document(document: &[u8]) -> Result<Value, serde_json::Error> {
    parse::parse(document, &Shape::Object(&schema::SKELETON), &mut Vec::new())
}

/// The refusal of a candidate whose bytes are not one readable JSON document.
pub(crate) fn invalid_json_defect(message: String) -> ValidationDefect {
    ValidationDefect::new("invalid_json", "candidate", "parse", message)
}

pub(crate) fn element_shape_defects(
    candidate: &[u8],
    shape: &ElementShape,
) -> Vec<ValidationDefect> {
    let document = candidate_document(candidate);
    let mut out = Vec::new();
    let parsed = match shape {
        ElementShape::Tasks => parse::parse(&document, &Shape::Object(&schema::SKELETON), &mut out),
        ElementShape::Acceptance => serde_json::from_slice::<Value>(&document),
    };
    let value = match parsed {
        Ok(value) => value,
        Err(error) => return vec![invalid_json_defect(error.to_string())],
    };
    // Assembly replaces the entire contract when entries is present, including
    // acceptance, PRD and gap policy, and stamps BOTH authored entry lists.
    // Legacy documents are retained verbatim, judgments included.
    let root = match shape {
        ElementShape::Tasks => &schema::SKELETON,
        ElementShape::Acceptance if value.get("entries").is_some() => &schema::AUTHORED,
        ElementShape::Acceptance => &schema::LEGACY,
    };
    Shape::Object(root).walk(Some(&value), "", &mut out);
    out.sort_by(|a, b| a.identity.cmp(&b.identity));
    out.dedup_by(|a, b| a.identity == b.identity);
    out
}
