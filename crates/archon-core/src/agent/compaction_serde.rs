use serde::de;
use serde::ser::{SerializeStructVariant, Serializer};

use super::autocompact::{CompactionOutcome, SkipReason};

impl serde::Serialize for CompactionOutcome {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Compacted {
                before_tokens,
                after_estimated_tokens,
                messages_before,
                messages_after,
            } => {
                let mut state =
                    serializer.serialize_struct_variant("CompactionOutcome", 0, "Compacted", 5)?;
                state.serialize_field("before_tokens", before_tokens)?;
                state.serialize_field("after_estimated_tokens", after_estimated_tokens)?;
                state.serialize_field("after_tokens", after_estimated_tokens)?;
                state.serialize_field("messages_before", messages_before)?;
                state.serialize_field("messages_after", messages_after)?;
                state.end()
            }
            Self::Skipped { reason } => {
                let mut state =
                    serializer.serialize_struct_variant("CompactionOutcome", 1, "Skipped", 1)?;
                state.serialize_field("reason", reason)?;
                state.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for CompactionOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        enum WireOutcome {
            Compacted(WireCompacted),
            Skipped { reason: SkipReason },
        }

        #[derive(serde::Deserialize)]
        struct WireCompacted {
            before_tokens: u64,
            #[serde(default)]
            after_estimated_tokens: Option<u64>,
            #[serde(default)]
            after_tokens: Option<u64>,
            messages_before: usize,
            messages_after: usize,
        }

        match WireOutcome::deserialize(deserializer)? {
            WireOutcome::Compacted(wire) => {
                let after_estimated_tokens = wire
                    .after_estimated_tokens
                    .or(wire.after_tokens)
                    .ok_or_else(|| de::Error::missing_field("after_estimated_tokens"))?;
                Ok(Self::Compacted {
                    before_tokens: wire.before_tokens,
                    after_estimated_tokens,
                    messages_before: wire.messages_before,
                    messages_after: wire.messages_after,
                })
            }
            WireOutcome::Skipped { reason } => Ok(Self::Skipped { reason }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacted_outcome_dual_writes_after_token_fields() {
        let outcome = CompactionOutcome::Compacted {
            before_tokens: 100,
            after_estimated_tokens: 40,
            messages_before: 8,
            messages_after: 3,
        };

        let value = serde_json::to_value(&outcome).expect("serialize outcome");
        let compacted = value.get("Compacted").expect("compacted variant");
        assert_eq!(compacted["after_estimated_tokens"], 40);
        assert_eq!(compacted["after_tokens"], 40);

        let round_tripped: CompactionOutcome =
            serde_json::from_value(value).expect("deserialize dual-written outcome");
        assert_eq!(round_tripped, outcome);
    }

    #[test]
    fn compacted_outcome_reads_legacy_after_tokens_field() {
        let legacy = serde_json::json!({
            "Compacted": {
                "before_tokens": 100,
                "after_tokens": 40,
                "messages_before": 8,
                "messages_after": 3
            }
        });

        let outcome: CompactionOutcome =
            serde_json::from_value(legacy).expect("deserialize legacy outcome");
        assert_eq!(
            outcome,
            CompactionOutcome::Compacted {
                before_tokens: 100,
                after_estimated_tokens: 40,
                messages_before: 8,
                messages_after: 3,
            }
        );
    }
}
