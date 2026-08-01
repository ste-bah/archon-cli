use std::collections::{BTreeMap, BTreeSet};

/// Computes a deterministic checksum for a sorted index and its member sets.
///
/// Callers canonicalize unordered map/set buckets before invoking this helper,
/// which makes equivalent indexes produce the same checksum regardless of
/// their original iteration order.
#[must_use]
pub fn deterministic_index_checksum(index: &BTreeMap<String, BTreeSet<String>>) -> u64 {
    index.iter().fold(0xcbf2_9ce4_8422_2325, |checksum, (key, members)| {
        let checksum = checksum_part(checksum, key);
        members
            .iter()
            .fold(checksum, |checksum, member| checksum_part(checksum, member))
    })
}

fn checksum_part(checksum: u64, value: &str) -> u64 {
    value.bytes().chain([0xff]).fold(checksum, |checksum, byte| {
        checksum.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(byte)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_checksum_detects_changed_members() {
        let left = BTreeMap::from([(
            "tag-0".to_owned(),
            BTreeSet::from(["agent-0@1.0.0".to_owned()]),
        )]);
        let right = BTreeMap::from([(
            "tag-0".to_owned(),
            BTreeSet::from(["agent-999@9.9.9".to_owned()]),
        )]);

        assert_ne!(
            deterministic_index_checksum(&left),
            deterministic_index_checksum(&right)
        );
    }

    #[test]
    fn index_checksum_is_independent_of_input_order_after_canonicalization() {
        let first = BTreeMap::from([
            (
                "tag-1".to_owned(),
                BTreeSet::from(["agent-1@1.0.0".to_owned(), "agent-0@1.0.0".to_owned()]),
            ),
            ("tag-0".to_owned(), BTreeSet::from(["agent-2@1.0.0".to_owned()])),
        ]);
        let second = BTreeMap::from([
            ("tag-0".to_owned(), BTreeSet::from(["agent-2@1.0.0".to_owned()])),
            (
                "tag-1".to_owned(),
                BTreeSet::from(["agent-0@1.0.0".to_owned(), "agent-1@1.0.0".to_owned()]),
            ),
        ]);

        assert_eq!(
            deterministic_index_checksum(&first),
            deterministic_index_checksum(&second)
        );
    }
}
