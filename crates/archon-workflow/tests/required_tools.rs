use archon_workflow::context::stage_input;
use archon_workflow::{WorkflowSpec, WorkflowStore};
use serde_json::json;

fn spec(field: &str) -> Result<WorkflowSpec, archon_workflow::WorkflowError> {
    WorkflowSpec::from_yaml(&format!(
        "schema: archon.workflow.v1\nname: tools\ntask: work\nstages:\n- id: author\n  kind: agent\n  custom: z\n{field}"
    ))
}

#[test]
fn typed_list_scalar_and_absent_preserve_context_contract() {
    for (field, expected) in [
        (
            "  required_tools: [bash, mcp__sample__fetch]\n",
            vec!["bash", "mcp__sample__fetch"],
        ),
        ("  required_tools: bash\n", vec!["bash"]),
        ("", vec![]),
        ("  required_tools: []\n", vec![]),
        ("  required_tools: null\n", vec![]),
        ("  required_tools: ~\n", vec![]),
        ("  required_tools:\n", vec![]),
    ] {
        let spec = spec(field).unwrap();
        let stage = &spec.stages[0];
        assert_eq!(stage.required_tools, expected);
        if expected.is_empty() && !field.is_empty() {
            assert_eq!(
                stage.extra.get("required_tools"),
                Some(&if field.contains("[]") {
                    json!([])
                } else {
                    json!(null)
                })
            );
        } else {
            assert!(!stage.extra.contains_key("required_tools"));
        }
        let mut head_extra = stage.extra.clone();
        if !field.is_empty() {
            head_extra.insert(
                "required_tools".into(),
                if expected.is_empty() && !field.contains("[]") {
                    json!(null)
                } else {
                    json!(expected)
                },
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(dir.path());
        let run = store.create_run(spec.clone()).unwrap();
        let input = stage_input(&store, &run, stage).unwrap();
        assert_eq!(
            serde_json::to_vec(&input["stage_extra"]).unwrap(),
            serde_json::to_vec(&head_extra).unwrap()
        );
        let reloaded = WorkflowSpec::from_yaml(&spec.to_yaml().unwrap()).unwrap();
        assert_eq!(reloaded, spec);
    }
}

#[test]
fn invalid_entries_name_stage_and_exact_entry() {
    for (field, entry, reason) in [
        ("  required_tools: ['']\n", "", "empty"),
        ("  required_tools: ['   ']\n", "   ", "empty"),
        ("  required_tools: [fetch, fetch]\n", "fetch", "duplicate"),
        (
            "  required_tools: [mcp__sample]\n",
            "mcp__sample",
            "incomplete",
        ),
        (
            "  required_tools: [mcp__sample__]\n",
            "mcp__sample__",
            "incomplete",
        ),
    ] {
        assert_eq!(
            spec(field).unwrap_err().to_string(),
            format!(
                "invalid workflow spec: stage 'author' required_tools entry '{entry}' is {reason}"
            )
        );
    }
}

#[test]
fn approval_metadata_moves_tools_out_of_flattened_extras() {
    for tools in [json!("fetch"), json!(["fetch", "mcp__sample__inspect"])] {
        let call = serde_json::from_value(json!({"id":"author", "method":"agent", "options":{"extra":{"required_tools":tools,"custom":true}}})).unwrap();
        let stage = archon_workflow::v2::plan_metadata::approval_metadata_stage("work", &call);
        assert!(!stage.extra.contains_key("required_tools"));
        assert_eq!(
            json!(stage.required_tools),
            if tools.is_string() {
                json!([tools])
            } else {
                tools
            }
        );
        assert_eq!(stage.extra["custom"], true);
    }
}

#[test]
fn historical_non_string_shapes_load_and_validate_with_stage_and_entry() {
    for (field, entry) in [("7", "7"), ("[fetch, 8]", "8"), ("[null, fetch]", "null")] {
        let raw = format!(
            "schema: archon.workflow.v1\nname: old\ntask: work\nstages:\n- id: author\n  kind: agent\n  required_tools: {field}\n"
        );
        let loaded: WorkflowSpec = serde_yaml_ng::from_str(&raw).unwrap();
        let error = loaded.validate().unwrap_err();
        assert!(matches!(
            error,
            archon_workflow::WorkflowError::SpecInvalid(_)
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "invalid workflow spec: stage 'author' required_tools entry '{entry}' is not a string"
            )
        );
        assert_eq!(
            WorkflowSpec::from_yaml(&raw).unwrap_err().to_string(),
            error.to_string()
        );
    }
}

#[test]
fn metadata_preserves_unrepresentable_and_explicit_empty_values() {
    for value in [json!(null), json!(7), json!(["fetch", 8]), json!([])] {
        let call = serde_json::from_value(
            json!({"id":"author", "method":"agent", "options":{"extra":{"required_tools":value}}}),
        )
        .unwrap();
        let stage = archon_workflow::v2::plan_metadata::approval_metadata_stage("work", &call);
        assert!(stage.required_tools.is_empty());
        assert_eq!(stage.extra["required_tools"], value);
        let reloaded: archon_workflow::StageSpec =
            serde_json::from_value(serde_json::to_value(&stage).unwrap()).unwrap();
        assert_eq!(reloaded, stage);
    }
}

#[test]
fn historical_yaml_scalar_strings_keep_direct_deserializer_coercions() {
    for (field, expected) in [
        ("id: 1", "1"),
        ("depends_on: [1]", "1"),
        ("task: 2024", "2024"),
        ("verify_command: true", "true"),
    ] {
        let raw = format!("kind: agent\n{field}\n");
        let raw = if field.starts_with("id:") {
            raw
        } else {
            format!("id: author\n{raw}")
        };
        let stage: archon_workflow::StageSpec = serde_yaml_ng::from_str(&raw).unwrap();
        let dependency = if field.starts_with("depends_on:") {
            "- id: '1'\n  kind: agent\n"
        } else {
            ""
        };
        let stage_yaml = raw
            .lines()
            .map(|line| format!("  {line}\n"))
            .collect::<String>();
        let workflow = WorkflowSpec::from_yaml(&format!(
            "schema: archon.workflow.v1\nname: old\ntask: work\nstages:\n{dependency}-\n{stage_yaml}"
        )).unwrap();
        assert_eq!(workflow.stages.last().unwrap(), &stage);
        let actual = match field.split_once(':').unwrap().0 {
            "id" => stage.id.as_str(),
            "depends_on" => stage.depends_on[0].as_str(),
            "task" => stage.task.as_deref().unwrap(),
            "verify_command" => stage.verify_command.as_deref().unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(actual, expected, "{field}");
    }
}
