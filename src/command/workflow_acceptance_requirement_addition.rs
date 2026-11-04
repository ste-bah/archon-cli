//! Match a requirement's add verb to the name in the same clause.
pub(super) fn asks_to_add(text: &str, name: &str) -> bool {
    let text = text.replace(". ", ";").replace(", but ", ";");
    text.split([';', '\n', '!', '?', ',']).any(|clause| {
        let clause = clause.trim().trim_end_matches('.');
        let boundary = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.');
        let named = clause.match_indices(name).any(|(at, _)| {
            !clause[..at].chars().next_back().is_some_and(boundary)
                && !clause[at + name.len()..]
                    .chars()
                    .next()
                    .is_some_and(boundary)
        });
        named
            && clause.split(|c: char| !c.is_alphanumeric()).any(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "add"
                        | "adds"
                        | "added"
                        | "adding"
                        | "create"
                        | "creates"
                        | "created"
                        | "creating"
                        | "introduce"
                        | "introduces"
                        | "introduced"
                        | "introducing"
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(non_snake_case)]
    fn D9_add_verb_names_the_same_path_in_the_same_clause() {
        // Input requirement and name -> expected addition match.
        for (text, name, expected) in [
            ("add X", "X", true),
            ("add X.", "X", true),
            ("address Y", "Y", false),
            ("add logging; X must exist", "X", false),
            ("Create newcrate/Cargo.toml", "newcrate/Cargo.toml", true),
            ("Add logging. X must exist", "X", false),
        ] {
            assert_eq!(asks_to_add(text, name), expected, "{text}");
        }
    }
}
