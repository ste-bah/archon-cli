#[derive(Debug, Clone, Copy)]
enum DynamicSourceKind {
    NoopProof,
    Implementation,
    Remediation,
    ReviewRemediation,
    FocusedVerification,
    ReviewVerification,
}

impl std::fmt::Display for DynamicSourceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DynamicSourceKind::NoopProof => formatter.write_str("noop proof"),
            DynamicSourceKind::Implementation => formatter.write_str("implementation"),
            DynamicSourceKind::Remediation => formatter.write_str("remediation"),
            DynamicSourceKind::ReviewRemediation => formatter.write_str("review remediation"),
            DynamicSourceKind::FocusedVerification => formatter.write_str("focused verification"),
            DynamicSourceKind::ReviewVerification => formatter.write_str("review verification"),
        }
    }
}

fn dynamic_source_kind(execution: &WorkflowV2CallExecution) -> Option<DynamicSourceKind> {
    if execution.call.id.starts_with("noop-proof-verification-")
        || execution.call.id.starts_with("noop-proof-reverification-")
    {
        return Some(DynamicSourceKind::NoopProof);
    }
    if execution.call.id.starts_with("verification-wave-") {
        return Some(DynamicSourceKind::FocusedVerification);
    }
    if execution.call.id.starts_with("review-verification-wave-") {
        return Some(DynamicSourceKind::ReviewVerification);
    }
    let write_fanout = execution.call.method == WorkflowV2HostMethod::Fanout
        && matches!(
            execution.call.write_mode,
            Some(WorkflowV2WriteMode::Coordinated | WorkflowV2WriteMode::Worktree)
        )
        && execution
            .call
            .options
            .item_kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("implementation"));
    if !write_fanout {
        return None;
    }
    if execution.call.id.starts_with("implementation-wave-") {
        return Some(DynamicSourceKind::Implementation);
    }
    if execution.call.id.starts_with("remediation-wave-") {
        return Some(DynamicSourceKind::Remediation);
    }
    if execution.call.id.starts_with("review-remediation-wave-") {
        return Some(DynamicSourceKind::ReviewRemediation);
    }
    None
}

#[derive(Debug, Clone, Default)]
struct TaskUniverse {
    canonical: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
}

impl TaskUniverse {
    fn from_authoritative(task_universe: &WorkflowV2TaskUniverse) -> Self {
        let mut out = Self::default();
        for task in &task_universe.tasks {
            out.add_canonical(task.canonical_task_id.clone());
            for alias in &task.aliases {
                out.aliases
                    .insert(alias.clone(), task.canonical_task_id.clone());
            }
        }
        out
    }

    fn is_empty(&self) -> bool {
        self.canonical.is_empty()
    }

    fn add_canonical(&mut self, task_id: String) {
        let canonical = task_id.trim().to_string();
        if canonical.is_empty() {
            return;
        }
        self.canonical.insert(canonical.clone());
        self.aliases.insert(canonical.clone(), canonical.clone());
        if let Some(short) = short_task_alias(&canonical) {
            self.aliases.insert(short, canonical);
        }
    }

    fn resolve(&self, value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if self.canonical.contains(trimmed) {
            return Some(trimmed.to_string());
        }
        self.aliases.get(trimmed).cloned()
    }
}

impl TaskUniverse {
    fn canonical_ids(&self) -> Vec<String> {
        self.canonical.iter().cloned().collect()
    }
}

fn source_task_graph_from_items(
    values: &[serde_json::Value],
    universe: &TaskUniverse,
    authoritative_task_universe: &WorkflowV2TaskUniverse,
    wave_kind: DynamicSourceKind,
    target_repository_root: Option<&str>,
) -> Option<WorkflowV2SourceTaskGraph> {
    if universe.is_empty() {
        return None;
    }
    let mut raw_items = Vec::new();
    let mut item_to_tasks: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for value in values {
        let normalized = normalize_generated_item_value(value, Some(authoritative_task_universe));
        if matches!(
            wave_kind,
            DynamicSourceKind::FocusedVerification | DynamicSourceKind::ReviewVerification
        ) && normalized.issues.iter().any(|issue| {
            issue.kind == GeneratedContractIssueKind::EvidenceRepair
                && matches!(
                    issue.field.as_str(),
                    "source_residual_gap_ids" | "failed_predicate"
                )
        })
        {
            return None;
        }
        let normalized_value = normalized.value;
        let value = &normalized_value;
        let item_id = item_id(value)?;
        if matches!(wave_kind, DynamicSourceKind::Remediation)
            && !remediation_item_has_required_fields(value)
        {
            return None;
        }
        if matches!(wave_kind, DynamicSourceKind::ReviewRemediation)
            && !review_remediation_item_has_required_fields(value)
        {
            return None;
        }
        if matches!(wave_kind, DynamicSourceKind::NoopProof)
            && !noop_item_has_required_fields(value)
        {
            return None;
        }
        if matches!(wave_kind, DynamicSourceKind::FocusedVerification)
            && !verification_item_has_required_fields(value)
        {
            return None;
        }
        if matches!(wave_kind, DynamicSourceKind::ReviewVerification)
            && !review_verification_item_has_required_fields(value)
        {
            return None;
        }
        let canonical_task_ids = sorted_unique(
            raw_task_refs(value)
                .into_iter()
                .filter_map(|task_id| universe.resolve(&task_id))
                .collect(),
        );
        if canonical_task_ids.is_empty() {
            return None;
        }
        item_to_tasks.insert(item_id.clone(), canonical_task_ids.clone());
        for task_id in &canonical_task_ids {
            item_to_tasks.insert(task_id.clone(), vec![task_id.clone()]);
            if let Some(alias) = short_task_alias(task_id) {
                item_to_tasks.insert(alias, vec![task_id.clone()]);
            }
        }
        raw_items.push((item_id, canonical_task_ids, normalized_value));
    }
    let canonical_universe = universe.canonical_ids();
    let mut items = Vec::new();
    for (item_id, canonical_task_ids, value) in raw_items {
        let dependency_ids = normalize_dependency_refs(&value, universe, &item_to_tasks);
        let raw_target_files = sorted_unique(non_empty_strings(
            value
                .get("target_files")
                .or_else(|| value.get("targetFiles")),
        ));
        let (declared_target_files, evidence_target_files) = source_graph_target_files(
            raw_target_files,
            target_repository_root,
            authoritative_task_universe,
        );
        let expanded_targets = if declared_target_files.is_empty() {
            None
        } else {
            Some(
                expand_declared_rust_module_targets(
                    &item_id,
                    &declared_target_files,
                    target_repository_root,
                )
                .ok()?,
            )
        };
        let target_files = expanded_targets
            .as_ref()
            .map(|expanded| expanded.target_files.clone())
            .unwrap_or_default();
        let declared_target_files = expanded_targets
            .as_ref()
            .map(|expanded| expanded.declared_target_files.clone())
            .unwrap_or(declared_target_files);
        let target_file_expansions = expanded_targets
            .as_ref()
            .map(|expanded| {
                expanded
                    .target_file_expansions
                    .iter()
                    .map(|expansion| WorkflowV2SourceTargetExpansion {
                        source: expansion.source.clone(),
                        expanded: expansion.expanded.clone(),
                        notes: expansion.notes.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        items.push(WorkflowV2SourceTaskItem {
            item_id,
            canonical_task_ids,
            dependency_ids,
            target_files,
            declared_target_files,
            target_file_expansions,
            acceptance_criteria: sorted_unique(non_empty_strings(
                value
                    .get("acceptance_criteria")
                    .or_else(|| value.get("acceptanceCriteria")),
            )),
            focused_verification: sorted_unique(non_empty_strings(
                value
                    .get("focused_verification")
                    .or_else(|| value.get("focused_tests"))
                    .or_else(|| value.get("focusedTests")),
            )),
            expected_evidence: sorted_unique(non_empty_strings(
                value
                    .get("expected_evidence")
                    .or_else(|| value.get("expectedEvidence")),
            )),
            artifact_requirements: sorted_unique({
                let mut values = non_empty_strings(
                    value
                        .get("artifact_requirements")
                        .or_else(|| value.get("artifacts")),
                );
                values.extend(evidence_target_files);
                values
            }),
        });
    }
    items.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    Some(WorkflowV2SourceTaskGraph::new(
        canonical_universe,
        items,
        Vec::new(),
    ))
}

fn source_graph_target_files(
    targets: Vec<String>,
    repository_root: Option<&str>,
    task_universe: &WorkflowV2TaskUniverse,
) -> (Vec<String>, Vec<String>) {
    let Some(root) = repository_root else {
        return (targets, Vec::new());
    };
    let mut source_targets = Vec::new();
    let mut evidence_targets = Vec::new();
    for target in targets {
        if is_task_source_target(&target, task_universe) || is_project_artifact_target(&target) {
            evidence_targets.push(target);
            continue;
        }
        source_targets.push(target);
    }
    if source_targets.is_empty() {
        return (source_targets, evidence_targets);
    }
    match archon_workflow::normalize_targets_for_repository(
        "source_graph",
        &source_targets,
        Some(root),
    ) {
        Ok(normalized) => (normalized, evidence_targets),
        Err(_) => (source_targets, evidence_targets),
    }
}

fn is_task_source_target(target: &str, task_universe: &WorkflowV2TaskUniverse) -> bool {
    let path = std::path::Path::new(target);
    path.is_absolute()
        && task_universe.source_roots.iter().any(|root| {
            let root = std::path::Path::new(root);
            !root.as_os_str().is_empty() && path.starts_with(root)
        })
}

fn is_project_artifact_target(target: &str) -> bool {
    std::path::Path::new(target)
        .components()
        .any(|component| component.as_os_str() == ".archon")
}
