//! Preserve duplicate evidence that derived struct readers reject. Maps keep
//! serde's behavior. A field the derived reader ignores is skipped the same
//! way, with `IgnoredAny`, so both accept the same documents (Issue 312).
//! Acceptance uses Value end to end.
use super::{Shape, pointer, shape_defect};
use archon_workflow::defect::ValidationDefect;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

pub(super) fn parse(
    bytes: &[u8],
    shape: &Shape,
    defects: &mut Vec<ValidationDefect>,
) -> Result<Value, serde_json::Error> {
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    let value = Seed {
        shape: Some(shape),
        at: String::new(),
        defects,
    }
    .deserialize(&mut reader)?;
    reader.end()?;
    Ok(value)
}

struct Seed<'a> {
    shape: Option<&'a Shape>,
    at: String,
    defects: &'a mut Vec<ValidationDefect>,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(mut self, reader: D) -> Result<Value, D::Error> {
        while let Some(Shape::Nullable(inner)) = self.shape {
            self.shape = Some(inner);
        }
        match self.shape {
            Some(Shape::Object(_) | Shape::List(_) | Shape::Map(_)) => reader.deserialize_any(self),
            Some(_) => Value::deserialize(reader),
            // No shape: the derived reader ignores this field (an unknown key)
            // or refuses it before reading it (a denied key, an extra
            // positional item). Skip it as serde skips it: JSON syntax only,
            // no recursion limit, string or number check. Keep `null` so the
            // walk still sees the key or the item.
            None => IgnoredAny::deserialize(reader).map(|IgnoredAny| Value::Null),
        }
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
        Ok(Value::from(value))
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        loop {
            let shape = match self.shape {
                Some(Shape::List(inner)) => Some(*inner),
                Some(Shape::Object(object)) => object
                    .sequence_override
                    .unwrap_or(object)
                    .fields
                    .get(items.len())
                    .map(|field| &field.shape),
                _ => None,
            };
            let Some(value) = seq.next_element_seed(Seed {
                shape,
                at: pointer(&self.at, &items.len().to_string()),
                defects: self.defects,
            })?
            else {
                break;
            };
            items.push(value);
        }
        Ok(Value::Array(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut fields = Map::new();
        // Per key: copies read so far, and the defects the latest copy added.
        let mut copies: HashMap<String, (usize, Range<usize>)> = HashMap::new();
        // Defects that name a copy; the message gets the total once known.
        let mut labels = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let (shape, closed) = match self.shape {
                Some(Shape::Object(object)) => (
                    object
                        .fields
                        .iter()
                        .find(|field| field.name == key)
                        .map(|field| &field.shape),
                    true,
                ),
                Some(Shape::Map(inner)) => (Some(*inner), false),
                _ => (None, false),
            };
            let at = pointer(&self.at, &key);
            let start = self.defects.len();
            let value = map.next_value_seed(Seed {
                shape,
                at: at.clone(),
                defects: self.defects,
            })?;
            let added = start..self.defects.len();
            let previous = fields.insert(key.clone(), value);
            let (count, latest) = copies.entry(key.clone()).or_insert((0, 0..0));
            *count += 1;
            let earlier = std::mem::replace(latest, added);
            let Some(previous) = previous else {
                continue;
            };
            // The reader stops at the first invalid copy or, for a struct, at
            // the first repeated copy. Deleting or repairing any one copy
            // exposes the next, so every copy is its own problem: its defects
            // and its repeat are counted once each and never merge with an
            // equal defect of another copy. A map reads every value.
            let walked = self.defects.len();
            if let Some(shape) = shape {
                shape.walk(Some(&previous), &at, self.defects);
            }
            let copy = *count - 1;
            for index in earlier.chain(walked..self.defects.len()) {
                overwritten(&mut self.defects[index], &at, copy);
                labels.push((index, key.clone(), copy, false));
            }
            // Closed structs refuse a repeated field, known or forbidden.
            if closed && (shape.is_some() || denies_unknown(self.shape)) {
                let mut defect = shape_defect(&at, "");
                defect.identity.location = format!("shape/duplicate/{count}");
                labels.push((self.defects.len(), key, *count, true));
                self.defects.push(defect);
            }
        }
        for (index, key, copy, repeat) in labels {
            let total = copies[&key].0;
            let defect = &mut self.defects[index];
            defect.message = if repeat {
                format!(
                    "{}: copy {copy} repeats the field ({total} copies); keep one valid copy",
                    defect.identity.subject
                )
            } else {
                format!(
                    "{} (in copy {copy} of {total} of a duplicated field; the reader checks it)",
                    defect.message
                )
            };
        }
        Ok(Value::Object(fields))
    }
}

fn denies_unknown(shape: Option<&Shape>) -> bool {
    matches!(shape, Some(Shape::Object(object)) if object.deny_unknown)
}

/// Give a defect the identity of copy `copy` of the duplicated field at `at`.
/// The field's depth names which enclosing field was duplicated (it is a
/// prefix of the subject), so nested duplicated copies stay distinct without
/// putting submitted keys into the location. Counts are additive per copy,
/// so renumbering after a deletion never hides the deleted copy's defects.
fn overwritten(defect: &mut ValidationDefect, at: &str, copy: usize) {
    let depth = at.split('/').count();
    let identity = &mut defect.identity;
    identity.location = format!("shape/overwritten/{depth}/{copy}/{}", identity.location);
}
