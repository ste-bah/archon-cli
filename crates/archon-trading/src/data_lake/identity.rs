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
    dataset_id_parts(&metadata.dataset_id).is_some_and(|parts| {
        parts.provider == metadata.provider.trim().to_ascii_lowercase()
            && parts.instrument == metadata.canonical_instrument.trim()
            && parts.timeframe == normalize_timeframe(&metadata.timeframe)
            && parts.price_basis == metadata.price_basis.trim()
    })
}

struct DatasetIdParts<'a> {
    provider: String,
    instrument: &'a str,
    timeframe: String,
    price_basis: &'a str,
}

fn dataset_id_parts(value: &str) -> Option<DatasetIdParts<'_>> {
    if !valid_dataset_id(value) {
        return None;
    }
    let (provider, rest) = value.split_once('-')?;
    let (prefix, price_basis) = rest.rsplit_once('-')?;
    let (instrument, timeframe) = prefix.rsplit_once('-')?;
    if provider.is_empty()
        || instrument.is_empty()
        || timeframe.is_empty()
        || price_basis.is_empty()
    {
        return None;
    }
    Some(DatasetIdParts {
        provider: provider.to_ascii_lowercase(),
        instrument,
        timeframe: normalize_timeframe(timeframe),
        price_basis,
    })
}

fn valid_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}
