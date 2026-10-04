//! Preserve duplicate evidence that derived struct readers reject. Maps and
//! ignored fields retain serde's behavior; acceptance uses Value end to end.
use super::{Shape, pointer, shape_defect};
use archon_workflow::defect::ValidationDefect;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use std::fmt;

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
            _ => Value::deserialize(reader),
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
        while let Some(key) = map.next_key::<String>()? {
            let shape = match self.shape {
                Some(Shape::Object(object)) => object
                    .fields
                    .iter()
                    .find(|field| field.name == key)
                    .map(|field| &field.shape),
                Some(Shape::Map(inner)) => Some(*inner),
                _ => None,
            };
            let at = pointer(&self.at, &key);
            let value = map.next_value_seed(Seed {
                shape,
                at: at.clone(),
                defects: self.defects,
            })?;
            if let Some(previous) = fields.insert(key, value)
                && let Some(shape) = shape
            {
                // Invalid overwritten values still fail a streaming reader.
                // Keep their leaf identities so removing either copy cannot
                // reveal defects hidden by Value's last-key-wins behavior.
                shape.walk(Some(&previous), &at, self.defects);
                if matches!(self.shape, Some(Shape::Object(_))) {
                    let mut defect = shape_defect(&at, "is a duplicate field; keep one valid copy");
                    defect.identity.location = "shape/duplicate".into();
                    self.defects.push(defect);
                }
            }
        }
        Ok(Value::Object(fields))
    }
}
