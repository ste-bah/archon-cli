//! Explicit directory headings scope the prose up to the next heading.

pub(crate) fn scoped_sections(text: &str) -> Vec<(String, Option<String>)> {
    let mut sections = Vec::new();
    let (mut body, mut scope) = (String::new(), None);
    use super::super::super::fences::{LineKind, classified_lines};
    for (kind, line) in classified_lines(text) {
        if kind == LineKind::Prose && line.trim_start().starts_with('#') {
            sections.push((std::mem::take(&mut body), scope.take()));
            let tokens: Vec<_> = line.split('`').skip(1).step_by(2).collect();
            if let [token] = tokens.as_slice() {
                if token.ends_with('/') {
                    scope = super::repository_directory_token(token);
                }
            }
        }
        body.push_str(line);
        body.push('\n');
    }
    sections.push((body, scope));
    sections
}

pub(super) fn extract_claims(text: &str) -> Vec<super::PathClaim> {
    scoped_sections(text)
        .into_iter()
        .flat_map(|(section, scope)| {
            super::super::assertion::sentences(&section)
                .into_iter()
                .flat_map(|(sentence, carried)| super::claims_in_sentence(sentence, carried))
                .map(move |mut claim| {
                    if claim.scope.is_none() {
                        claim.scope = scope.clone();
                    }
                    claim
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
