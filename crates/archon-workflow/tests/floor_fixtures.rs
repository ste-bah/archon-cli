//! Recorded verdicts from the real verifier, before its replacement.
use std::{fs, path::Path};

use archon_workflow::{
    FloorVerdict, collect_declarative_floor_facts, evaluate_floor_predicates,
    task_universe::WorkflowV2DeliverableContract, v2::deliverable_contract::ContractRoots,
};
use serde_json::{Value, json};

#[path = "support/floor_strict_expectations.rs"]
mod strict;

fn cases() -> Vec<String> {
    let mut cases: Vec<_> = fs::read_dir(corpus())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .collect();
    cases.sort();
    cases
}

fn corpus() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floors")
}

fn inputs(
    case: &Path,
) -> (
    Value,
    WorkflowV2DeliverableContract,
    ContractRoots,
    tempfile::TempDir,
) {
    let raw: Value =
        serde_json::from_slice(&fs::read(case.join("contract.json")).unwrap()).unwrap();
    let contract = serde_json::from_value(raw.clone()).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        if !from.is_dir() {
            return;
        }
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    copy(&case.join("roots"), temporary.path());
    let count: usize = fs::read_to_string(case.join("roots.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    for index in 0..count {
        fs::create_dir_all(temporary.path().join(index.to_string())).unwrap();
    }
    if case.file_name().unwrap() == "jsonl_cr" {
        let path = temporary.path().join("0/payload.json");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace('\r', "\n")
            .replace('\n', "\r");
        fs::write(path, text).unwrap();
    }
    if case.file_name().unwrap() == "single_present" {
        fs::write(temporary.path().join("0/artifact.json"), b"{\r\n}\r\n").unwrap();
    }
    let mut root = temporary.path().join("0");
    if let Ok(suffix) = fs::read_to_string(case.join("root_suffix.txt")) {
        root.push(suffix.trim());
    }
    let second = temporary.path().join("1");
    let roots = ContractRoots::new(
        root.to_str().unwrap(),
        second.is_dir().then(|| second.to_str().unwrap()),
    );
    let now = fs::read_to_string(case.join("now.txt")).unwrap();
    chrono::DateTime::parse_from_rfc3339(now.trim()).unwrap();
    (raw, contract, roots, temporary)
}

fn native(contract: &WorkflowV2DeliverableContract, roots: &ContractRoots, case: &Path) -> Value {
    let mut facts = collect_declarative_floor_facts(roots, contract).unwrap();
    facts.verified_at = Some(
        fs::read_to_string(case.join("now.txt"))
            .unwrap()
            .trim()
            .into(),
    );
    let (verdict, findings) = match evaluate_floor_predicates(contract, &facts) {
        FloorVerdict::Passed => ("passed", Vec::new()),
        FloorVerdict::Failed { findings } => ("failed", findings),
    };
    let findings: Vec<_> = findings
        .into_iter()
        .map(|finding| {
            let mut finding = finding.replace('\\', "/");
            for (index, root) in roots.ordered().enumerate() {
                finding = finding.replace(&root.replace('\\', "/"), &format!("<root-{index}>"));
            }
            finding
        })
        .collect();
    json!({"verdict": verdict, "findings": findings})
}

#[test]
fn recorded_native_findings_are_reproducible() {
    let mut mismatches = Vec::new();
    for name in cases() {
        let case = corpus().join(&name);
        let (_, contract, roots, _temporary) = inputs(&case);
        let expected: Value =
            serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
        let actual = native(&contract, &roots, &case);
        let pinned = strict::expected(&name, &expected);
        if actual != pinned {
            mismatches.push(format!("{name}: actual {actual}; expected {pinned}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

fn assert_parity(name: &str) {
    let case = corpus().join(&name);
    let (_, contract, roots, _temporary) = inputs(&case);
    let expected: Value =
        serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
    let recorded_pass = expected["python"]["exit"] == 0;
    let native_pass = native(&contract, &roots, &case)["verdict"] == "passed";
    assert_eq!(
        native_pass, recorded_pass,
        "{name}: recorded verdict parity"
    );
}

#[test]
fn identity_numeric_parity() {
    assert_parity("identity_numeric");
}

#[test]
fn identity_string_parity() {
    assert_parity("identity_string");
}

#[test]
fn identity_type_mismatch_parity() {
    assert_parity("identity_type_mismatch");
}

/// POSIX-only one-off recorder: the existing verifier is a shell script.
#[cfg(unix)]
#[test]
#[ignore = "one-off recording from the real shell verifier; requires python3"]
fn record_real_verifier_verdicts() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    let version = Command::new("python3").arg("--version").output().unwrap();
    assert!(version.status.success());
    let version = String::from_utf8(version.stdout).unwrap();
    for name in cases() {
        let case = corpus().join(&name);
        let (raw, contract, roots, _temporary) = inputs(&case);
        let script = archon_workflow::v2::deliverable_contract::verification_command(&roots, &raw);
        assert!(
            script.contains("ARCHON_PY"),
            "the original recorder must never label a replacement as the reference"
        );
        let mut child = Command::new(archon_shell::resolve_posix_shell())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let expected = json!({
            "recorded_with": version.trim(),
            "python": {"exit": output.status.code().unwrap(), "stdout": stdout},
            "native": native(&contract, &roots, &case),
        });
        fs::write(
            case.join("expected.json"),
            format!("{}\n", serde_json::to_string_pretty(&expected).unwrap()),
        )
        .unwrap();
        eprintln!("recorded {name}: {expected}");
    }
}

#[test]
fn corpus_verdict_parity() {
    let mut differences = Vec::new();
    for name in cases() {
        let case = corpus().join(&name);
        let (_, contract, roots, _temporary) = inputs(&case);
        let expected: Value =
            serde_json::from_slice(&fs::read(case.join("expected.json")).unwrap()).unwrap();
        if (expected["python"]["exit"] == 0)
            != (native(&contract, &roots, &case)["verdict"] == "passed")
            && expected["divergence"].is_null()
        {
            differences.push(name);
        }
    }
    assert!(
        differences.is_empty(),
        "verdict differences: {differences:?}"
    );
}

#[test]
#[ignore = "initial native finding recording; leaves the reference verdict untouched"]
fn record_native_findings() {
    for name in cases() {
        let case = corpus().join(&name);
        let (_, contract, roots, _temporary) = inputs(&case);
        let path = case.join("expected.json");
        let mut expected: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if expected.get("native_before").is_none() {
            expected["native_before"] = expected["native"].clone();
        }
        expected["native"] = native(&contract, &roots, &case);
        fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&expected).unwrap()),
        )
        .unwrap();
    }
}

#[test]
#[ignore = "read-only audit of an external frozen task set named by FLOOR_FROZEN_TASK_SET"]
fn external_frozen_contracts_parse() {
    let path = std::env::var_os("FLOOR_FROZEN_TASK_SET").expect("audit input");
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let mut count = 0;
    let mut failures = Vec::new();
    for task in value["tasks"].as_array().unwrap() {
        for contract in task["deliverable_contracts"]
            .as_array()
            .into_iter()
            .flatten()
        {
            count += 1;
            if let Err(error) =
                serde_json::from_value::<WorkflowV2DeliverableContract>(contract.clone())
            {
                failures.push(error.to_string());
            }
        }
    }
    eprintln!(
        "frozen contract audit: {count} contracts, {} parse failures",
        failures.len()
    );
    assert!(count > 0);
    assert!(failures.is_empty(), "{failures:?}");
}
