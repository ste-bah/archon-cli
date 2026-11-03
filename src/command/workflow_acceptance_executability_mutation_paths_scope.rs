use super::*;

#[derive(Clone, Default)]
pub(super) struct ScopeState {
    pub directory: Option<PathBuf>,
    pub variables: BTreeMap<String, String>,
    pub previous_directory: Option<PathBuf>,
    pub directory_stack: Vec<(Option<PathBuf>, Option<PathBuf>)>,
    pub flow_stack: Vec<VariableFlow>,
    pub conditional_variables: std::collections::BTreeSet<String>,
    pub conditional_list_changed: bool,
    pub conditional_list_exits: bool,
    pub condition_changed_directory: bool,
    pub group_depth: usize,
    pub conditional_exit_directory: Option<PathBuf>,
}

impl ScopeState {
    pub fn child(&self) -> Self {
        Self {
            directory: self.directory.clone(),
            previous_directory: self.previous_directory.clone(),
            variables: self.variables.clone(),
            ..Self::default()
        }
    }
}
