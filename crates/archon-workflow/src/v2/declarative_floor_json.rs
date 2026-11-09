//! Evidence uses strict JSON; non-finite numeric tokens are invalid.
use serde_json::Value;

pub(super) fn parse(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonfinite_tokens_fail_without_changing_string_values() {
        for text in ["NaN", "Infinity", "-Infinity", "[NaN]", "{\"n\":Infinity}"] {
            assert!(parse(text.as_bytes()).is_err(), "{text}");
        }
        assert_eq!(parse(br#"{"n":"NaN"}"#).unwrap()["n"], "NaN");
    }

    #[test]
    fn tagged_and_untagged_float_payloads_keep_standard_number_deserialization() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        #[serde(tag = "kind")]
        enum Tagged {
            Float { value: f64 },
        }
        #[derive(serde::Deserialize, Debug, PartialEq)]
        #[serde(untagged)]
        enum Untagged {
            Float { value: f64 },
            Text(String),
        }
        assert_eq!(
            serde_json::from_str::<Tagged>(r#"{"kind":"Float","value":1.5}"#).unwrap(),
            Tagged::Float { value: 1.5 }
        );
        assert_eq!(
            serde_json::from_str::<Untagged>(r#"{"value":1.5}"#).unwrap(),
            Untagged::Float { value: 1.5 }
        );
    }
}
