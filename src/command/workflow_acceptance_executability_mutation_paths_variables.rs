use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub(super) enum VariableFlow {
    Branch {
        start: Option<PathBuf>,
        previous: Option<PathBuf>,
        variables: BTreeMap<String, String>,
        variable_ends: Vec<BTreeMap<String, String>>,
        ends: Vec<Option<PathBuf>>,
        previous_ends: Vec<Option<PathBuf>>,
        has_else: bool,
    },
    Loop {
        start: Option<PathBuf>,
        previous: Option<PathBuf>,
        changed: bool,
        variables: BTreeMap<String, String>,
        assigned: BTreeSet<String>,
    },
}

pub(super) fn valid_variable(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(super) fn join_variables(states: &[BTreeMap<String, String>]) -> BTreeMap<String, String> {
    let Some(first) = states.first() else {
        return BTreeMap::new();
    };
    first
        .iter()
        .filter(|(name, value)| states.iter().all(|state| state.get(*name) == Some(*value)))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(super) fn apply_assignments(
    words: &[String],
    variables: &mut BTreeMap<String, String>,
    flows: &mut [VariableFlow],
    conditional_variables: &mut BTreeSet<String>,
    conditional: bool,
) {
    let skip = usize::from(words.first().is_some_and(|word| word == "export"));
    for word in words.iter().skip(skip) {
        if let Some((name, value)) = word.split_once('=')
            && valid_variable(name)
        {
            if !value.contains(['$', '`']) {
                variables.insert(name.to_string(), value.to_string());
            } else {
                variables.remove(name);
            }
            for flow in flows.iter_mut() {
                if let VariableFlow::Loop { assigned, .. } = flow {
                    assigned.insert(name.to_string());
                }
            }
            if conditional {
                conditional_variables.insert(name.to_string());
            }
        }
    }
}
