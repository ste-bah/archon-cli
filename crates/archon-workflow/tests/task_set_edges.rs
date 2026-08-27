use archon_workflow::task_set_edges::{analyze_task_set_edges, validate_task_set_edges};
use archon_workflow::task_skeleton::{
    ConsumedArtifact, FrozenDependency, FrozenTask, TaskSkeleton,
};
use archon_workflow::task_universe::WorkflowV2DeliverableContract;

fn contract(path: &str) -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "report".into(),
        artifact_path: path.into(),
        typed_verifier_command: Some("grep -q required {artifact_path}".into()),
        ..Default::default()
    }
}

fn task(id: &str) -> FrozenTask {
    FrozenTask {
        task_id: id.into(),
        file_name: format!("{id}-body.md"),
        depends_on: Vec::new(),
        blocks: Vec::new(),
        implements: Vec::new(),
        deliverable_contracts: Vec::new(),
    }
}

fn skeleton(tasks: Vec<FrozenTask>) -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "acceptance".into(),
        tasks,
    }
}

fn dependency(task_id: &str) -> FrozenDependency {
    FrozenDependency {
        task_id: task_id.into(),
        ..Default::default()
    }
}

fn consumed(path: &str) -> ConsumedArtifact {
    ConsumedArtifact {
        artifact_path: path.into(),
        ..Default::default()
    }
}

#[test]
fn dependency_must_choose_data_or_ordering_but_not_neither_or_both() {
    let mut producer = task("TASK-X-001");
    let mut output = contract("out.json");
    output.min_instances = 1;
    producer.deliverable_contracts.push(output);
    let mut consumer = task("TASK-X-010");
    consumer.depends_on.push(dependency("TASK-X-001"));

    let findings = validate_task_set_edges(&skeleton(vec![producer.clone(), consumer.clone()]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].message.contains("TASK-X-010"));
    assert!(findings[0].message.contains("TASK-X-001"));
    assert!(
        findings[0]
            .message
            .contains("add a non-empty consumes list or set ordering_only: true")
    );

    consumer.depends_on[0].ordering_only = true;
    assert!(
        validate_task_set_edges(&skeleton(vec![producer.clone(), consumer.clone()])).is_empty()
    );

    consumer.depends_on[0].consumes.push(consumed("out.json"));
    let findings = validate_task_set_edges(&skeleton(vec![producer, consumer]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].message.contains("choose exactly one"));
}

#[test]
fn consumed_path_must_be_produced_by_the_named_dependency() {
    let mut producer = task("TASK-X-001");
    producer.deliverable_contracts.push(contract("actual.json"));
    let mut consumer = task("TASK-X-010");
    let mut edge = dependency("TASK-X-001");
    edge.consumes.push(consumed("missing.json"));
    consumer.depends_on.push(edge);

    let findings = validate_task_set_edges(&skeleton(vec![producer.clone(), consumer.clone()]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    let message = &findings[0].message;
    assert!(message.contains("TASK-X-010"), "{message}");
    assert!(message.contains("TASK-X-001"), "{message}");
    assert!(message.contains("missing.json"), "{message}");
    assert!(
        message
            .contains("add that exact artifact_path to the producer or correct the consumes path"),
        "{message}"
    );

    consumer.depends_on[0].consumes[0].artifact_path = "./actual.json".into();
    assert!(validate_task_set_edges(&skeleton(vec![producer, consumer])).is_empty());
}

#[test]
fn block_only_edge_is_refused_until_the_consumer_declares_it() {
    let mut producer = task("TASK-X-001");
    let mut grounding = contract("state.json");
    grounding.min_instances = 1;
    producer.deliverable_contracts.push(grounding);
    producer.blocks.push("TASK-X-010".into());
    let mut consumer = task("TASK-X-010");

    let findings = validate_task_set_edges(&skeleton(vec![producer.clone(), consumer.clone()]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].message.contains("block-only edge"));
    assert!(
        findings[0]
            .message
            .contains("add TASK-X-001 to TASK-X-010 depends_on")
    );

    let mut edge = dependency("TASK-X-001");
    edge.ordering_only = true;
    consumer.depends_on.push(edge);
    assert!(validate_task_set_edges(&skeleton(vec![producer, consumer])).is_empty());
}

#[test]
fn shared_collection_requires_a_matching_record_binding() {
    let mut first = task("TASK-X-001");
    let mut first_contract = contract("registry.json");
    first_contract.registry_records_field = Some("records".into());
    first.deliverable_contracts.push(first_contract);
    let mut second = task("TASK-X-002");
    let mut second_contract = contract("registry.json");
    second_contract.registry_records_field = Some("records".into());
    second.deliverable_contracts.push(second_contract);
    let mut consumer = task("TASK-X-010");
    let mut edge = dependency("TASK-X-001");
    edge.consumes.push(consumed("registry.json"));
    consumer.depends_on.push(edge);

    let findings = validate_task_set_edges(&skeleton(vec![
        first.clone(),
        second.clone(),
        consumer.clone(),
    ]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].message.contains("multi-writer"));
    assert!(
        findings[0]
            .message
            .contains("registry_records_field: records")
    );

    consumer.depends_on[0].consumes[0].registry_records_field = Some("entries".into());
    let findings = validate_task_set_edges(&skeleton(vec![
        first.clone(),
        second.clone(),
        consumer.clone(),
    ]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0]
            .message
            .contains("does not match producer field 'records'")
    );

    consumer.depends_on[0].consumes[0].registry_records_field = Some("records".into());
    assert!(validate_task_set_edges(&skeleton(vec![first, second, consumer])).is_empty());
}

#[test]
fn kind_mismatch_is_informational_not_blocking() {
    let mut producer = task("TASK-X-001");
    let mut output = contract("out.json");
    output.kind = "producer_label".into();
    producer.deliverable_contracts.push(output);
    let mut consumer = task("TASK-X-010");
    let mut edge = dependency("TASK-X-001");
    let mut input = consumed("out.json");
    input.kind = Some("consumer_label".into());
    edge.consumes.push(input);
    consumer.depends_on.push(edge);

    let analysis = analyze_task_set_edges(&skeleton(vec![producer, consumer]));
    assert!(analysis.blockers.is_empty(), "{:?}", analysis.blockers);
    assert_eq!(analysis.information.len(), 1);
    assert!(
        analysis.information[0]
            .message
            .contains("informational kind mismatch")
    );
    assert!(analysis.information[0].message.contains("does not block"));
}

#[test]
fn all_ordering_graph_requires_a_positive_data_obligation() {
    let producer = task("TASK-X-001");
    let mut consumer = task("TASK-X-010");
    let mut edge = dependency("TASK-X-001");
    edge.ordering_only = true;
    consumer.depends_on.push(edge);

    let findings = validate_task_set_edges(&skeleton(vec![producer.clone(), consumer.clone()]));
    assert!(findings.iter().any(|finding| {
        finding.field == "graph"
            && finding.message.contains("every dependency is ordering_only")
            && finding.message.contains("declare at least one consumes edge or a positive/source-bound instance contract")
    }), "{findings:?}");

    let mut grounded = producer;
    let mut output = contract("out.json");
    output.min_instances = 1;
    grounded.deliverable_contracts.push(output);
    assert!(validate_task_set_edges(&skeleton(vec![grounded, consumer])).is_empty());
}
