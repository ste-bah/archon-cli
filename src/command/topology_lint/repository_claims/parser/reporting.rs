/// A path immediately following a reporting clause is quoted or recounted
/// prose, not the body's own repository assertion.
pub(super) fn reported_claim_clause(before: &str) -> bool {
    let lowered = before.to_ascii_lowercase();
    let reported = [
        "claim that ",
        "claims that ",
        "statement that ",
        "assumption that ",
        "assertion that ",
        "says ",
        "says that ",
        "said ",
        "said that ",
        "states ",
        "states that ",
        "stated ",
        "stated that ",
        "asserts ",
        "asserts that ",
        "claims ",
        "claims that ",
        "assumes ",
        "assumes that ",
    ];
    if reported
        .iter()
        .any(|lead| ends_with_whole_word_lead(&lowered, lead))
    {
        return true;
    }

    let mut prefix = lowered.trim_end();
    loop {
        let separator = [", and", ", or", "and", "or", ","]
            .iter()
            .find(|separator| {
                prefix
                    .get(prefix.len().saturating_sub(separator.len())..)
                    .is_some_and(|suffix| suffix.eq_ignore_ascii_case(separator))
            });
        let Some(separator) = separator else {
            return false;
        };
        let preceding = prefix[..prefix.len() - separator.len()].trim_end();
        let Some(before_path) = preceding.strip_suffix('`') else {
            return false;
        };
        let Some((before_open, _path)) = before_path.rsplit_once('`') else {
            return false;
        };
        if reported
            .iter()
            .any(|lead| ends_with_whole_word_lead(before_open, lead))
        {
            return true;
        }
        prefix = before_open.trim_end();
    }
}

fn ends_with_whole_word_lead(text: &str, lead: &str) -> bool {
    let Some(start) = text.len().checked_sub(lead.len()) else {
        return false;
    };
    text.get(start..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(lead))
        && text[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_alphanumeric())
}
