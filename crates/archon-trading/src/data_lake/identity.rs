use super::{DatasetMetadata, normalize_timeframe};

pub(crate) const DATASET_SCHEMA_V1: &str = "archon-trading-dataset-v1";

pub(crate) fn dataset_id(metadata: &DatasetMetadata) -> Option<String> {
    let provider = identity_component(&metadata.provider)?.to_ascii_lowercase();
    let instrument = identity_component(&metadata.canonical_instrument)?;
    let normalized_timeframe = normalize_timeframe(&metadata.timeframe);
    let timeframe = identity_component(&normalized_timeframe)?;
    let price_basis = identity_component(&metadata.price_basis)?;
    Some(format!("{provider}-{instrument}-{timeframe}-{price_basis}"))
}

pub(crate) fn raw_bound_version(created_at: &str, raw_sha256: &str) -> Option<String> {
    let bytes = created_at.as_bytes();
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') || bytes.get(10) != Some(&b'T') {
        return None;
    }
    let date = format!(
        "{}{}{}",
        created_at.get(0..4)?,
        created_at.get(5..7)?,
        created_at.get(8..10)?
    );
    let hash = raw_sha256.get(..8)?;
    (date.chars().all(|c| c.is_ascii_digit()) && hash.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("{}-{}", date, hash.to_ascii_lowercase()))
}

fn identity_component(value: &str) -> Option<&str> {
    (!value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric())).then_some(value)
}

pub(super) fn valid_dataset_id(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().all(valid_identifier_char)
}

pub(super) fn valid_version(value: &str) -> bool {
    let Some((date, suffix)) = value.split_once('-') else {
        return false;
    };
    date.len() == 8
        && date.chars().all(|c| c.is_ascii_digit())
        && !suffix.is_empty()
        && suffix.chars().all(valid_identifier_char)
}

pub(super) fn dataset_id_matches_metadata(metadata: &DatasetMetadata) -> bool {
    if metadata.symbol_map.is_empty() {
        return false;
    }
    dataset_id(metadata).is_some_and(|expected| metadata.dataset_id == expected)
        || diagnostic_id_is_allowed(metadata)
}

fn diagnostic_id_is_allowed(metadata: &DatasetMetadata) -> bool {
    !metadata.production_eligible
        && metadata.quality_status.eq_ignore_ascii_case("degraded")
        && metadata
            .dataset_id
            .starts_with(&format!("{}-", metadata.provider.to_ascii_lowercase()))
}

fn valid_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}
