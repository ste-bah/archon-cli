//! Stamping implementation copied from origin/main (6beec2f59).
//! Kept independent so old receipts cannot agree with a changed stamper by construction.
use archon_workflow::{WorkflowError, WorkflowResult};
use serde::de::DeserializeOwned;

pub(crate) fn stamp_frozen_frontmatter(
    candidate: &str,
    frozen: &archon_workflow::task_skeleton::FrozenTask,
) -> WorkflowResult<(String, Vec<String>)> {
    let opener = candidate.find("```yaml").ok_or_else(|| {
        WorkflowError::SpecInvalid("bound task body has no YAML frontmatter block".into())
    })?;
    let body_start = opener + "```yaml".len();
    let close_rel = candidate[body_start..].find("```").ok_or_else(|| {
        WorkflowError::SpecInvalid("bound task body YAML frontmatter block is not closed".into())
    })?;
    let close = body_start + close_rel;
    let yaml = &candidate[body_start..close];
    let mut metadata: serde_json::Value = serde_yaml_ng::from_str(yaml).map_err(|error| {
        WorkflowError::SpecInvalid(format!(
            "bound task body YAML frontmatter is invalid: {error}"
        ))
    })?;
    let map = metadata.as_object_mut().ok_or_else(|| {
        WorkflowError::SpecInvalid("bound task body YAML frontmatter is not a mapping".into())
    })?;
    const KNOWN_FIELDS: &[&str] = &[
        "task_id",
        "title",
        "complexity",
        "status",
        "depends_on",
        "blocks",
        "implements",
        "required_env_keys",
        "required_tools",
        "deliverable_contracts",
        "prd",
        "domain",
        "workstream",
        "source_sections",
        "shared_append_target_files",
        "artifact_requirements",
    ];
    if let Some(field) = map
        .keys()
        .find(|field| !KNOWN_FIELDS.contains(&field.as_str()))
    {
        return Err(WorkflowError::SpecInvalid(format!(
            "task frontmatter contains unknown field '{field}'"
        )));
    }
    let mut notes = Vec::new();
    stamp_owned_fields(map, frozen, &mut notes)?;
    // Serializing the host's own mapping is a host fault, never `SpecInvalid`.
    let stamped_yaml = serde_yaml_ng::to_string(&metadata)?;
    let stamped_yaml = stamped_yaml.strip_prefix("---\n").unwrap_or(&stamped_yaml);
    let mut result = String::with_capacity(candidate.len() + stamped_yaml.len());
    result.push_str(&candidate[..body_start]);
    result.push('\n');
    result.push_str(stamped_yaml.trim_end_matches('\n'));
    result.push('\n');
    result.push_str(&candidate[close..]);
    Ok((result, notes))
}

// This declaration drives both stamping and remediation ownership.
macro_rules! frozen_fields {
    ($($field:ident: $ty:ty),+ $(,)?) => {
        pub(crate) const HOST_STAMPED_FIELDS: &[&str] = &[$(stringify!($field)),+];
        fn stamp_owned_fields(
            map: &mut serde_json::Map<String, serde_json::Value>,
            frozen: &archon_workflow::task_skeleton::FrozenTask,
            notes: &mut Vec<String>,
        ) -> WorkflowResult<()> {
            $(stamp_typed_field::<$ty>(map, stringify!($field), &frozen.$field, notes)?;)+
            Ok(())
        }
    };
}
frozen_fields! {
    depends_on: Vec<archon_workflow::task_skeleton::FrozenDependency>,
    blocks: Vec<String>,
    implements: Vec<String>,
    source_sections: Vec<String>,
    deliverable_contracts: Vec<archon_workflow::task_universe::WorkflowV2DeliverableContract>,
}

fn stamp_typed_field<T>(
    map: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    expected: &T,
    notes: &mut Vec<String>,
) -> WorkflowResult<()>
where
    T: DeserializeOwned + serde::Serialize + Default + PartialEq,
{
    let matches = match map.get(field) {
        Some(value) => serde_json::from_value::<T>(value.clone())
            .map(|authored| authored == *expected)
            .unwrap_or(false),
        None => T::default() == *expected,
    };
    if !matches {
        notes.push(field.to_string());
    }
    map.insert(field.to_string(), serde_json::to_value(expected)?);
    Ok(())
}
