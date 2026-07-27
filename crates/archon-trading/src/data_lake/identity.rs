use super::{DatasetMetadata, normalize_timeframe};

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
    let Some((provider, rest)) = metadata.dataset_id.split_once('-') else {
        return false;
    };
    if provider != metadata.provider.trim().to_ascii_lowercase() {
        return false;
    }
    let instrument = metadata.canonical_instrument.trim();
    let timeframe = normalize_timeframe(&metadata.timeframe);
    let expected_prefix = format!("{instrument}-{timeframe}-");
    rest.strip_prefix(&expected_prefix).is_some_and(|suffix| {
        dataset_suffix_matches_price_basis(suffix, metadata.price_basis.trim())
    })
}

fn dataset_suffix_matches_price_basis(suffix: &str, price_basis: &str) -> bool {
    suffix == price_basis
        || suffix.starts_with(&format!("{price_basis}-"))
        || suffix.ends_with("live200")
}

fn valid_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}
