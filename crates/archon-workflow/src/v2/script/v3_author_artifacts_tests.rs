//! The `artifacts:` rule in the dialect reference (issue-12).
//!
//! The reference used to gloss `artifacts:` as "artifacts the work must
//! produce", and an author read a data store the task's code merely migrated
//! as one. The host then checked the store on return, found it empty — as a
//! migration of an empty store correctly leaves it — and the branch was
//! failed. The host side now raises emptiness for review instead; the rule
//! here is what stops the over-declaration at the source.

/// The `artifacts:` rule as one line of prose: the option's comment block
/// from the reference, with the `//` continuation stripped and the lines
/// joined, so a phrase can be asserted whole however the block is wrapped.
fn artifacts_rule() -> String {
    let reference = super::V3_PRIMITIVE_REFERENCE;
    let block = reference
        .split("artifacts: ['relative/artifact.path'],")
        .nth(1)
        .and_then(|rest| rest.split("tier: 'coder'").next())
        .expect("the artifacts rule sits between the artifacts and tier options");
    block
        .lines()
        .map(|line| line.trim().trim_start_matches("//").trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The reference says what `artifacts:` is FOR, in generic terms.
#[test]
fn the_artifacts_rule_names_what_belongs_in_it() {
    let rule = artifacts_rule();

    assert!(
        rule.contains("ONLY files this task must PRODUCE WITH CONTENT as its deliverables"),
        "the rule must say artifacts are produced-with-content deliverables: {rule}"
    );
    for example in [
        "a report",
        "a generated spec",
        "a data output the task itself populates",
    ] {
        assert!(
            rule.contains(example),
            "the positive examples must be generic deliverable kinds, missing: {example}"
        );
    }
}

/// The reference says what does NOT belong, using the phrases a task file
/// uses for such files, and sends those to `targetFiles` or nowhere.
#[test]
fn the_artifacts_rule_excludes_files_the_code_mutates_or_a_later_task_populates() {
    let rule = artifacts_rule();

    for phrase in [
        "\"mutated only by code paths\"",
        "\"written by the code under test\"",
        "\"never hand-edited\"",
        "a LATER task populates is NOT an artifact of this task",
        "put it in targetFiles if the task may edit it, otherwise nowhere",
    ] {
        assert!(
            rule.contains(phrase),
            "the exclusion must be explicit, missing: {phrase}\n{rule}"
        );
    }
}

/// A model copies the example over the instruction, so the rule carries a
/// negative one — and it names no project, task, or file.
#[test]
fn the_artifacts_rule_carries_a_generic_negative_example() {
    let rule = artifacts_rule();

    assert!(
        rule.contains(
            "WRONG: a schema-migration task listing the data store its new schema will hold"
        ),
        "the rule must show what not to declare: {rule}"
    );
    assert!(
        rule.contains("legitimately empty until the task that loads it runs"),
        "the negative example must state why the file is not this task's deliverable: {rule}"
    );
    assert!(
        rule.contains("RIGHT for that task: its migration report, if the task file declares one"),
        "and what to declare instead: {rule}"
    );
    for specific in ["TASK-", "PRD-", ".json", ".archon/", "registry", "trading"] {
        assert!(
            !rule.contains(specific),
            "the rule must stay generic, but names {specific}"
        );
    }
}

/// The rule states the host's consequence for each case, so an author knows
/// a declaration is a contract and not a hint.
#[test]
fn the_artifacts_rule_states_what_the_host_does_with_a_declared_path() {
    let rule = artifacts_rule();

    assert!(
        rule.contains("Absent or zero bytes FAILS the branch"),
        "the hard failure must be named: {rule}"
    );
    assert!(
        rule.contains("parses but holds no records is flagged for the verifier"),
        "the review outcome must be named: {rule}"
    );
}
