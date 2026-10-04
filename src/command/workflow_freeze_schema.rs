//! Complete serde field schema. Exhaustive typed fixtures guard drift.
use super::{Field, ObjectShape, Shape};
use Shape::{Any, Bool, Choice, List, Map, Nullable, Object, String, Unsigned};

const fn required(name: &'static str, shape: Shape) -> Field {
    Field {
        name,
        required: true,
        shape,
    }
}
const fn defaulted(name: &'static str, shape: Shape) -> Field {
    Field {
        name,
        required: false,
        shape,
    }
}
const fn optional(name: &'static str, shape: &'static Shape) -> Field {
    defaulted(name, Nullable(shape))
}
const STRINGS: Shape = List(&String);
const USIZE: Shape = Unsigned(usize::MAX as u64);
const U64: Shape = Unsigned(u64::MAX);
const U32: Shape = Unsigned(u32::MAX as u64);
const U8S: Shape = List(&Unsigned(u8::MAX as u64));

pub(super) const DELIVERABLE: ObjectShape = ObjectShape {
    deny_unknown: true,
    sequence_override: None,
    fields: &[
        required("kind", String),
        required("artifact_path", String),
        optional("typed_verifier_command", &String),
        optional("registry_path", &String),
        optional("instance_source_path", &String),
        optional("instance_source_records_field", &String),
        optional("instance_artifact_field", &String),
        defaulted("min_instances", USIZE),
        defaulted("required_universe", Bool),
        optional("data_kind", &String),
        defaulted("universe_fields", STRINGS),
        optional("cells_field", &String),
        defaulted("cell_identity_fields", STRINGS),
        defaulted("required_true_fields", STRINGS),
        defaulted("required_nonempty_fields", STRINGS),
        defaulted("positive_count_fields", STRINGS),
        defaulted("minimum_count_fields", Map(&U64)),
        optional("gaps_field", &String),
        optional("registry_records_field", &String),
        defaulted("registry_key_fields", STRINGS),
        defaulted("registry_required_true_fields", STRINGS),
        optional("registry_status_field", &String),
        defaulted("registry_allowed_statuses", STRINGS),
        optional("registry_count_field", &String),
        defaulted("registry_minimum_count", U64),
        defaulted("registry_identity_fields", Map(&String)),
        optional("payload_path_field", &String),
        optional("payload_format", &String),
        defaulted("required_fields", STRINGS),
        defaulted("non_constant_fields", STRINGS),
        optional("artifact_format", &String),
        optional("observed_time_field", &String),
        optional("closed_weekdays", &U8S),
        defaulted("closed_dates", STRINGS),
        optional("step_variety_min_rows", &USIZE),
        optional("step_variety_min_percent", &U32),
        defaulted("series_value_fields", STRINGS),
        defaulted("series_overlap_min_rows", USIZE),
        optional("request_path_field", &String),
        optional("requested_count_field", &String),
        optional("response_path_field", &String),
        defaulted("response_identity_fields", Map(&String)),
        optional("validation_path_field", &String),
        optional("validation_status_field", &String),
        optional("validation_checks_field", &String),
        optional("validation_check_status_field", &String),
        defaulted("validation_failed_values", STRINGS),
        defaulted("validation_passed_values", STRINGS),
    ],
};
const CONSUMED: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("artifact_path", String),
        optional("instance_source_records_field", &String),
        optional("registry_records_field", &String),
        optional("kind", &String),
    ],
};
const DEPENDENCY: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("task_id", String),
        defaulted("consumes", List(&Object(&CONSUMED))),
        defaulted("ordering_only", Bool),
    ],
};
pub(super) const TASK: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("task_id", String),
        required("file_name", String),
        defaulted("depends_on", List(&Object(&DEPENDENCY))),
        defaulted("blocks", STRINGS),
        defaulted("implements", STRINGS),
        defaulted("deliverable_contracts", List(&Object(&DELIVERABLE))),
    ],
};
const COMMAND: ObjectShape = ObjectShape {
    deny_unknown: true,
    sequence_override: None,
    fields: &[
        required("command", String),
        required("cwd", Choice(&["project_root", "repo_root"], true)),
    ],
};
const FLOOR: ObjectShape = ObjectShape {
    deny_unknown: true,
    sequence_override: None,
    fields: &[required("contract", Object(&DELIVERABLE))],
};
const CHECK: Shape = Shape::Tagged("kind", &[("command", &COMMAND), ("floor", &FLOOR)]);
const JUDGMENT: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("verdict", Choice(&["accepted", "refuted"], false)),
        required("counterexample", String),
        required("reason", String),
        required("host_call_id", String),
        optional("sampling", &Any),
    ],
};
// Assembly stamps object entries only. Positional entries retain judgments.
pub(super) const ENTRY: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: Some(&LEGACY_ENTRY),
    fields: &[
        required("id", String),
        required("criterion", String),
        required("check", CHECK),
        defaulted("gap_permitted", Bool),
        defaulted("judgment", Any),
        defaulted("covers", STRINGS),
    ],
};
const LEGACY_ENTRY: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("id", String),
        required("criterion", String),
        required("check", CHECK),
        defaulted("gap_permitted", Bool),
        required("judgment", Object(&JUDGMENT)),
        defaulted("covers", STRINGS),
    ],
};
pub(super) const AUTHORED: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("entries", List(&Object(&ENTRY))),
        defaulted("supplementary", List(&Object(&ENTRY))),
    ],
};
const PRD: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[required("path", String), required("digest", String)],
};
const GAP_POLICY: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        defaulted("permitted_acceptance_ids", STRINGS),
        defaulted("forbidden_phrases", STRINGS),
        defaulted("required_fields", STRINGS),
    ],
};
pub(super) const LEGACY: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("schema_version", U32),
        required("prd", Object(&PRD)),
        required("gap_policy", Object(&GAP_POLICY)),
        required("acceptance", List(&Object(&LEGACY_ENTRY))),
        defaulted("supplementary", List(&Object(&LEGACY_ENTRY))),
    ],
};
pub(super) const SKELETON: ObjectShape = ObjectShape {
    deny_unknown: false,
    sequence_override: None,
    fields: &[
        required("schema_version", U32),
        required("acceptance_digest", String),
        required("tasks", List(&Object(&TASK))),
    ],
};
