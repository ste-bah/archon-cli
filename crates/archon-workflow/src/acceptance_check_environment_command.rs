use super::*;

/// The policy-selected check environment for an agent-authored host verifier.
/// Uses the same builder as direct/scratch acceptance, with the host HOME.
/// Environment construction must succeed before a command can be built.
pub struct CommandEnvironment {
    pub(super) variables: BTreeMap<String, String>,
    withheld: BTreeSet<String>,
    remedy: &'static str,
}

impl CommandEnvironment {
    pub fn capture(policy: Option<&CheckPolicy>) -> Result<Self, String> {
        Self::capture_with_dispatch(policy, &[])
    }

    /// Capture the OS environment once and apply approved dispatch locators.
    pub fn capture_with_dispatch(
        policy: Option<&CheckPolicy>,
        dispatch: &[(String, String)],
    ) -> Result<Self, String> {
        // One OS snapshot for bindings and withheld names. A non-Unicode
        // value cannot be forwarded, but its Unicode name still needs a note.
        let host: Vec<_> = std::env::vars_os().collect();
        let names: BTreeSet<String> = host
            .iter()
            .filter_map(|(name, _)| name.to_str().map(str::to_owned))
            .collect();
        let unicode = unicode_environment(host);
        let mut effective = policy
            .cloned()
            .unwrap_or_else(|| CheckPolicy::default_for(&unicode));
        effective.bind_dispatch(dispatch);
        let mut environment = Self::from_host(&unicode, Some(&effective))?;
        let withheld =
            withheld::withheld_names(names.iter().map(String::as_str), &environment.variables);
        environment.withheld.extend(withheld);
        Ok(environment)
    }

    /// The policy is operator-owned, never taken from verifier/task text.
    /// `None` means the operator configured no policy.
    pub fn from_host(
        host: &BTreeMap<String, String>,
        policy: Option<&CheckPolicy>,
    ) -> Result<Self, String> {
        let site: Vec<(&str, &Path)> = lookup(host, "HOME")
            .map(|(_, home)| ("HOME", Path::new(home)))
            .into_iter()
            .collect();
        // Host verifiers share the leased machine toolchains. A data allowlist
        // must not select a cold cache. Start with the builder's approved host
        // bindings, then apply explicit operator/dispatch bindings. Scratch
        // environments still use their own site bindings and directory homes.
        let mut effective = CheckPolicy::default_for(host);
        if let Some(policy) = policy {
            effective.toolchain_path = policy.toolchain_path.clone();
            for (name, value) in &policy.bound {
                while let Some((previous, _)) = lookup(&effective.bound, name) {
                    let previous = previous.clone();
                    effective.bound.remove(&previous);
                }
                effective.bound.insert(name.clone(), value.clone());
            }
            effective.forwarded = policy.forwarded.clone();
        }
        let variables = check_environment(host, &effective, &site)
            .map_err(|reason| format!("check command environment could not be built: {reason}"))?;
        Ok(Self {
            withheld: withheld(host, &variables),
            remedy: "Supply the needed names in the operator-owned CheckPolicy.forwarded passed to this verifier",
            variables,
        })
    }

    /// Describe the actual operator policy source this runner consumes.
    pub fn with_remedy(mut self, remedy: &'static str) -> Self {
        self.remedy = remedy;
        self
    }

    /// Every child still passes through the process-wide spawn boundary.
    pub fn command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        let mut command = archon_shell::spawn::command(program);
        archon_shell::spawn::replace_environment(&mut command, &self.variables);
        command
    }

    pub fn tokio_command(&self, program: impl AsRef<OsStr>) -> tokio::process::Command {
        tokio::process::Command::from(self.command(program))
    }

    /// Call only for a failed verifier. Persist this separate diagnostic with
    /// the result; stdout/stderr must never change the verdict (Issue 349).
    pub fn note(&self, outputs: &[&[u8]]) -> Option<String> {
        let note = withheld::withheld_note_with_remedy(outputs, &self.withheld, self.remedy);
        if let Some(note) = &note {
            tracing::warn!(target: "archon_workflow::check_environment", "{note}");
        }
        note
    }
}
