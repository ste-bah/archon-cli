//! Author-step acceptance entry validation (Issue 357): exactly the freeze
//! shape path, run on one entry in the authored envelope. Pure and
//! synchronous: no publication or judge is invoked.
use super::shape::{ENTRY_SHAPE, element_shape_defects, invalid_json_defect};
use archon_workflow::defect::ValidationDefect;
use serde_json::Value;

/// Where the envelope puts the entry. In the real freeze candidate that
/// pointer names a different entry, so no refusal text may carry it.
const ENVELOPE_POINTER: &str = "entries/0";

/// Freeze's shape defects for one entry's JSON text, in the bytes freeze
/// would read for it. The text is never parsed first: JSON.stringify writes an
/// unpaired surrogate as an escape that JSON.parse accepts and serde_json
/// refuses, and that must come back as freeze's own `invalid_json` refusal of
/// the model's reply, not as a validator fault. A deeper reply than serde
/// reads is refused the same way.
pub(crate) fn entry_defects(entry: &str) -> Result<Vec<ValidationDefect>, String> {
    let candidate = format!("{{\"entries\":[{entry}]}}");
    let defects = element_shape_defects(candidate.as_bytes(), &ENTRY_SHAPE);
    if defects
        .iter()
        .any(|defect| defect.identity.code == "invalid_json")
    {
        return Ok(defects);
    }
    // The text must be one JSON value: one that closes the envelope early
    // would validate a different document. That is a caller fault.
    let one_value = serde_json::from_str::<Value>(&candidate).is_ok_and(|document| {
        document.as_object().is_some_and(|root| root.len() == 1)
            && document["entries"]
                .as_array()
                .is_some_and(|list| list.len() == 1)
    });
    if one_value {
        Ok(defects)
    } else {
        Err("the entry argument is not exactly one JSON value".to_string())
    }
}

/// `message` with the envelope pointer made relative to the entry.
fn entry_relative(message: &str) -> String {
    message
        .split(' ')
        .map(|word| match word.strip_prefix(ENVELOPE_POINTER) {
            Some(rest) if rest.starts_with('/') => rest[1..].to_string(),
            Some(rest) if !rest.starts_with(|c: char| c.is_ascii_alphanumeric()) => {
                format!("entry{rest}")
            }
            _ => word.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The refusal text a repair prompt shows every pending entry: it names the
/// refused entry by its id. The measured identity keeps freeze's pointer.
pub(crate) fn refusal_text(id: &str, defect: &ValidationDefect) -> String {
    format!(
        "acceptance entry '{id}' was refused: {}",
        entry_relative(&defect.message)
    )
}

fn binding_error(what: &'static str, message: String) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message(what, "JSON", message)
}

/// Installs `__archonValidateAcceptanceEntry(id, entryJson)`, which returns
/// the JSON list of `{text, deterministic_defect}` refusals (empty: valid).
/// Only a binding fault throws (a non-string argument, or text that is not
/// one JSON value); every defect of the model's reply is a refusal.
pub(crate) fn install_entry_validator<'js>(ctx: &rquickjs::Ctx<'js>) -> rquickjs::Result<()> {
    ctx.globals().set(
        "__archonValidateAcceptanceEntry",
        rquickjs::function::Func::from(
            |id: String, entry: rquickjs::String<'js>| -> rquickjs::Result<String> {
                let defects = match entry.to_string() {
                    Ok(text) => {
                        entry_defects(&text).map_err(|message| binding_error("entry", message))?
                    }
                    // A JS string holding a raw unpaired surrogate is not
                    // UTF-8: the reply is unreadable, as freeze refuses it.
                    Err(rquickjs::Error::Utf8(error)) => vec![invalid_json_defect(format!(
                        "the entry text is not valid UTF-8 (an unpaired surrogate): {error}"
                    ))],
                    Err(error) => return Err(error),
                };
                let refusals: Vec<_> = defects
                    .iter()
                    .map(|defect| {
                        serde_json::json!({
                            "text": refusal_text(&id, defect),
                            "deterministic_defect": defect.identity,
                        })
                    })
                    .collect();
                serde_json::to_string(&refusals)
                    .map_err(|error| binding_error("defects", error.to_string()))
            },
        ),
    )
}
