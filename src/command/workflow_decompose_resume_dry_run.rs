//! Read-only admission and script planning for a fixed decomposition resume.

use super::*;

pub(crate) async fn dry_run_fixed_decomposition(
    cwd: &Path,
    run_id: &str,
    config: &ArchonConfig,
) -> Result<String> {
    // Admission stays read-only: unlike resume, this path never takes a lease,
    // repairs a stale owner, or advances lifecycle state.
    if config.workflow.gate_mode == GateMode::Off {
        return Err(anyhow!(DECOMPOSE_GATE_OFF_REMEDY));
    }
    let project_root = canonical_existing(cwd, "project root")?;
    let store = WorkflowStore::project(&project_root);
    let run = store.load_state(run_id)?;
    crate::command::workflow_decompose_owner::require_owner(&store, run_id, None)?;
    if !matches!(
        run.status,
        archon_workflow::RunStatus::Paused | archon_workflow::RunStatus::Cancelled
    ) {
        return Err(anyhow!(
            "fixed decomposition {run_id} can dry-run only from paused or cancelled status, found {:?}",
            run.status
        ));
    }
    let state = read_fixed_state(&store, run_id)?;
    let healed_groups =
        crate::command::workflow_host_command_groups::require_no_running_groups_read_only(
            &store.run_dir(run_id),
            run_id,
        )?;
    if state.run_kind != WorkflowRunKind::FixedDecompositionV1 {
        return Err(anyhow!(
            "workflow {run_id} is not a FixedDecompositionV1 run"
        ));
    }
    archon_workflow::WorkflowBundle::verify(&store, run_id)?;
    let compiled_path = store
        .run_dir(run_id)
        .join(archon_workflow::bundle::COMPILED_SPEC_FILE);
    let compiled_spec: archon_workflow::WorkflowSpec =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&compiled_path).with_context(|| {
            format!(
                "reading verified fixed workflow spec {}",
                compiled_path.display()
            )
        })?)?;
    if run.spec != compiled_spec {
        return Err(anyhow!(
            "fixed decomposition mutable run spec differs from the verified workflow bundle"
        ));
    }
    let prd_path = canonical_existing(Path::new(&state.identity.prd_identity), "persisted PRD")?;
    let task_root = canonical_existing(
        Path::new(&state.identity.task_root_identity),
        "persisted task root",
    )?;
    let canonical_persisted_project = canonical_existing(
        Path::new(&state.identity.project_root_identity),
        "persisted project root",
    )?;
    let catalog = fixed_decomposition_catalog(&state.identity.starting_binary_revision)?;
    let current_identity = FixedRunIdentityV1 {
        template_version: FIXED_DECOMPOSITION_TEMPLATE_VERSION.to_string(),
        starting_binary_revision: env!("ARCHON_GIT_HASH").to_string(),
        script_digest: workflow_scaffold_hash(FIXED_SCRIPT_SOURCE),
        catalog_digest: catalog.digest.clone(),
        project_root_identity: path_text(&canonical_persisted_project),
        prd_identity: path_text(&prd_path),
        task_root_identity: path_text(&task_root),
    };
    archon_workflow::verify_fixed_resume_identity(&state.identity, &current_identity)?;
    if canonical_persisted_project != project_root {
        return Err(anyhow!(
            "fixed decomposition resume project root differs from the invoking project; run the command from {}",
            canonical_persisted_project.display()
        ));
    }
    let run_dir = store.run_dir(run_id);
    let recorded_source = std::fs::read_to_string(archon_workflow::bundle::record_path(&run_dir))?;
    if workflow_scaffold_hash(&recorded_source) != state.identity.script_digest {
        return Err(anyhow!(
            "fixed decomposition resume paused: identity.script_digest does not match workflow.js"
        ));
    }
    let (prd_bytes, prd_digest, acceptance_criteria) =
        crate::command::workflow_task_set::validate_prd_input(&prd_path)?;
    let prd_text = std::str::from_utf8(&prd_bytes)?;
    let requirement_texts =
        archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text);
    let arguments: serde_json::Value = read_run_json(&store, run_id, FIXED_ARGUMENTS_PATH)?;
    let frozen_chain = arguments
        .get("frozenChain")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            upgrade::unmapped(
                "decomposition/arguments.json.frozenChain",
                "missing or not an object",
            )
        })?;
    let _: crate::command::workflow_decompose_frozen_chain::FrozenChainSnapshot = upgrade::decode(
        frozen_chain.clone(),
        "decomposition/arguments.json.frozenChain",
    )?;
    let repository_root = archon_workflow::repository_record::read_repository_record(&task_root)?
        .map(|record| PathBuf::from(record.repository_root))
        .ok_or_else(|| anyhow!("fixed decomposition {run_id} task root {} carries no {}; the launch that created the run recorded one", task_root.display(), archon_workflow::repository_record::REPOSITORY_LOCK_FILE))?;
    let expected_arguments = super::fixed_script_arguments(
        &project_root,
        &prd_path,
        &prd_digest,
        acceptance_criteria,
        config,
        &task_root,
        &repository_root,
        frozen_chain,
    );
    upgrade::require_equal(
        &arguments,
        &expected_arguments,
        "decomposition/arguments.json",
    )?;
    let persisted_catalog: archon_workflow::CommandCapabilityCatalog =
        read_run_json(&store, run_id, FIXED_CATALOG_PATH)?;
    if !crate::command::workflow_host_command_exec::identity::catalog_schema_readable(
        persisted_catalog.schema_version,
        catalog.schema_version,
    ) {
        return Err(upgrade::unmapped(
            "command-catalog.schema_version",
            &format!(
                "found {}; this binary reads schemas 1..={}",
                persisted_catalog.schema_version, catalog.schema_version
            ),
        ));
    }
    let mut verified_catalog = persisted_catalog.clone();
    verified_catalog.recompute_digest()?;
    if verified_catalog.digest != persisted_catalog.digest
        || persisted_catalog.digest != state.identity.catalog_digest
        || persisted_catalog.starting_binary_revision != state.identity.starting_binary_revision
    {
        return Err(anyhow!(
            "fixed decomposition resume paused: command-catalog digest or starting revision differs from identity"
        ));
    }
    let current_route = crate::command::workflow_provider_route::resolve_anthropic_route(
        config.api.base_url.as_deref(),
        crate::command::workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly,
    );
    let persisted_route: crate::command::workflow_provider_route::TrustedProviderRouteSnapshot =
        read_run_json(&store, run_id, FIXED_PROVIDER_ROUTE_PATH)?;
    upgrade::require_equal(
        &serde_json::to_value(&persisted_route)?,
        &serde_json::to_value(&current_route)?,
        "provider route",
    )?;
    let metadata: serde_json::Value = read_run_json(&store, run_id, FIXED_GENERATED_METADATA_PATH)?;
    let expected_metadata = policy::canonical_resume_metadata(
        &state.identity,
        state.identity.script_digest.clone(),
        &expected_arguments,
        prd_text,
        &metadata,
    );
    let anchored_digest = compiled_spec
        .permissions
        .get(FIXED_LAUNCH_DIGEST_PERMISSION)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("verified fixed workflow spec has no launch digest anchor"))?;
    policy::admit(
        &metadata,
        expected_metadata,
        &state.identity,
        &arguments,
        &persisted_catalog,
        &persisted_route,
        anchored_digest,
    )?;
    let metadata_form = if metadata.get("script_args") == Some(&expected_arguments) {
        "legacy"
    } else {
        "enriched"
    };
    upgrade::validate_result_state(&store, run_id)?;
    let criteria = expected_arguments["acceptanceCriteria"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(id, text)| Some((id.clone(), text.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let would_record =
        upgrade::would_record_seed_transition(&store, run_id, &state.identity, &current_identity)?;
    let (seed, seed_case) = if let Some((index, runtime)) = would_record {
        (
            crate::command::workflow_decompose_seed::current_seed_read_only_for_transition(
                &store,
                run_id,
                index,
                &runtime,
                &criteria,
                &requirement_texts,
            )?,
            format!("would-record transition {} (derived in memory)", index + 1),
        )
    } else {
        let seed = crate::command::workflow_decompose_seed::current_seed_read_only(
            &store,
            run_id,
            &criteria,
            &requirement_texts,
        )?;
        let case = seed.as_ref().map_or_else(
            || "none (no harness transition)".to_string(),
            |seed| format!("existing transition {}", seed.transition_index + 1),
        );
        (seed, case)
    };
    let seeded = crate::command::workflow_decompose_seed::seeded_arguments(
        &expected_arguments,
        seed.as_ref(),
    );
    let script_arguments = script_arguments_with_prd_requirement_texts(&seeded, prd_text);
    let calls = archon_workflow::v2::script::dry_run_workflow_plan(
        FIXED_SCRIPT_SOURCE,
        Some(&script_arguments),
    )
    .await?;
    // The planner runs the fixed script with its recording executor, so these
    // are the script's emitted calls and prompts, not a Rust retry simulation.
    let mut output = format!(
        "binary_revision={}\nadmission=admitted arguments.json=equal metadata={metadata_form} launch_digest_anchor=equal\nseed={seed_case}\nhealed_groups_would_remove={}\n",
        env!("ARCHON_GIT_HASH"),
        healed_groups.len(),
    );
    if let Some(seed) = seed.as_ref() {
        for (subject, data) in &seed.subjects {
            match data {
                crate::command::workflow_decompose_seed::SubjectSeed::Entries {
                    carried_entries,
                    gates,
                    unreadable_replies,
                    refuted_ids,
                    ..
                } => {
                    let entries = carried_entries
                        .iter()
                        .filter(|e| !e["id"].as_str().unwrap_or_default().starts_with("SUP-"))
                        .count();
                    let supplementary = carried_entries.len() - entries;
                    output.push_str(&format!("seed subject={subject} carried={} entries={entries} supplementary={supplementary} gates={} unreadable_ids={} refuted_ids={}\n", carried_entries.len(), gates.len(), unreadable_replies.keys().cloned().collect::<Vec<_>>().join(","), refuted_ids.join(",")));
                }
                crate::command::workflow_decompose_seed::SubjectSeed::Artifact { gate, .. } => {
                    output.push_str(&format!(
                        "seed subject={subject} gates={}\n",
                        usize::from(gate.is_some())
                    ))
                }
            }
        }
    }
    for (index, call) in calls.iter().enumerate() {
        let prompt = call.options.task.as_deref().unwrap_or("");
        let entry = prompt
            .split("Author ONLY entry ")
            .nth(1)
            .and_then(|tail| tail.split(':').next())
            .unwrap_or("");
        output.push_str(&format!(
            "call[{index}] id={}{}\n",
            call.id,
            if entry.is_empty() {
                String::new()
            } else {
                let criterion = script_arguments["acceptanceCriteria"][entry]
                    .as_str()
                    .or_else(|| {
                        entry.strip_prefix("SUP-").and_then(|requirement| {
                            script_arguments["prdRequirementTexts"][requirement].as_str()
                        })
                    })
                    .unwrap_or("");
                format!(" entry={entry} criterion={criterion:?}")
            }
        ));
    }
    Ok(output)
}
