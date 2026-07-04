fn annotate_write_ownership_expansions(
    branch_results: &mut [WorkflowV2Result],
    plan: &WorkflowV2WritePlan,
) {
    for result in branch_results {
        if !should_scan_for_ownership_expansion(result) {
            continue;
        }
        let owned = owned_targets_for_result(result, plan);
        let proposals = proposed_ownership_expansions(result, &owned);
        if !proposals.is_empty() {
            tag_ownership_expansion_required(result, proposals);
        }
    }
}

fn should_scan_for_ownership_expansion(result: &WorkflowV2Result) -> bool {
    matches!(
        result.status,
        WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview | WorkflowV2Status::Failed
    ) && !matches!(
        failure_kind_from_write_result(result),
        Some(BranchFailureKind::Safety | BranchFailureKind::Execution)
    )
}

fn owned_targets_for_result(
    result: &WorkflowV2Result,
    plan: &WorkflowV2WritePlan,
) -> BTreeSet<String> {
    let branch_id = result
        .data
        .get("branch_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            result
                .data
                .get("item_id")
                .and_then(serde_json::Value::as_str)
        });
    let mut targets = BTreeSet::new();
    for assignment in plan.waves.iter().flat_map(|wave| &wave.assignments) {
        if branch_id.is_none_or(|id| id == assignment.item_id) {
            targets.extend(assignment.owned_targets.iter().cloned());
        }
    }
    if targets.is_empty() {
        for assignment in plan.waves.iter().flat_map(|wave| &wave.assignments) {
            targets.extend(assignment.owned_targets.iter().cloned());
        }
    }
    targets
}

fn proposed_ownership_expansions(
    result: &WorkflowV2Result,
    owned: &BTreeSet<String>,
) -> Vec<serde_json::Value> {
    let mut paths = BTreeSet::new();
    for text in ownership_evidence_strings(result) {
        paths.extend(path_tokens_from_text(&text));
    }
    paths
        .into_iter()
        .filter(|path| !owned.iter().any(|owned_path| paths_overlap(owned_path, path)))
        .map(|path| {
            serde_json::json!({
                "path": path,
                "role": ownership_role_for_path(&path),
                "reason": "branch evidence referenced an explicit repo path outside declared ownership"
            })
        })
        .collect()
}

fn ownership_evidence_strings(result: &WorkflowV2Result) -> Vec<String> {
    let mut values = Vec::new();
    for evidence in &result.evidence {
        values.extend(evidence.source.clone());
    }
    for file in result.files_read.iter().chain(result.files_changed.iter()) {
        values.push(file.path.clone());
    }
    for gap in &result.residual_gaps {
        if text_describes_ownership_gap(&gap.id) || text_describes_ownership_gap(&gap.description) {
            values.push(gap.description.clone());
        }
    }
    values
}

fn text_describes_ownership_gap(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("ownership")
        || lower.contains("target_files")
        || lower.contains("target files")
        || lower.contains("undeclared")
        || lower.contains("outside declared")
        || lower.contains("outside current owned")
}

fn path_tokens_from_text(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !is_path_char(ch))
        .filter_map(normalize_repo_path_token)
        .collect()
}

fn normalize_repo_path_token(raw: &str) -> Option<String> {
    let token = raw.trim_matches(|ch: char| matches!(ch, ',' | ':' | ';' | ')' | '(' | '"' | '\''));
    let token = token.strip_suffix('.').unwrap_or(token);
    if !looks_like_repo_path(token) || token.starts_with(".archon/") || token.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in token.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('*') {
            return None;
        }
        parts.push(part);
    }
    Some(parts.join("/"))
}

fn looks_like_repo_path(token: &str) -> bool {
    token.contains('/')
        && !token.starts_with('-')
        && !token.contains("://")
        && token
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.'))
}

fn is_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '@' | '+')
}

fn ownership_role_for_path(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    if lockfile_name(name) {
        "lockfile"
    } else if manifest_name(name) {
        "manifest"
    } else if path_role_contains(path, &["test", "tests", "spec", "specs"]) {
        "test"
    } else if path_role_contains(path, &["docs", ".github", "config"]) {
        "docs_config"
    } else {
        "source"
    }
}

fn lockfile_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("package-lock.json")
        || name.eq_ignore_ascii_case("pnpm-lock.yaml")
        || name.eq_ignore_ascii_case("yarn.lock")
        || name.ends_with(".lock")
}

fn manifest_name(name: &str) -> bool {
    matches!(
        name,
        "package.json" | "pyproject.toml" | "go.mod" | "pom.xml" | "build.gradle"
    )
}

fn path_role_contains(path: &str, needles: &[&str]) -> bool {
    path.split('/').any(|part| {
        needles
            .iter()
            .any(|needle| part.eq_ignore_ascii_case(needle))
    })
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn tag_ownership_expansion_required(
    result: &mut WorkflowV2Result,
    proposals: Vec<serde_json::Value>,
) {
    let mut object = result.data.as_object().cloned().unwrap_or_default();
    object.insert(
        "ownership_expansion_required".to_string(),
        serde_json::Value::Bool(true),
    );
    object.insert(
        "ownership_policy_version".to_string(),
        serde_json::Value::String("workflow-v2-ownership-expansion-v1".to_string()),
    );
    object.insert(
        "proposed_ownership_expansions".to_string(),
        serde_json::Value::Array(proposals),
    );
    object.insert(
        "failure_kind".to_string(),
        serde_json::to_value(BranchFailureKind::Semantic).unwrap_or_default(),
    );
    result.data = serde_json::Value::Object(object);
    push_ownership_expansion_gap(result);
}

fn push_ownership_expansion_gap(result: &mut WorkflowV2Result) {
    if result
        .residual_gaps
        .iter()
        .any(|gap| gap.id == "ownership_expansion_required")
    {
        return;
    }
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "ownership_expansion_required".to_string(),
        description: "branch evidence requires JS-owned review of explicit repo ownership expansion before more write remediation".to_string(),
        severity: Some("review".to_string()),
    });
}
