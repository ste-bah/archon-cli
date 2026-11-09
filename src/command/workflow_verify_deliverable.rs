//! Read-only evaluation before application bootstrap.
use crate::cli_args::WorkflowVerifyDeliverableArgs;
use archon_workflow::task_universe::WorkflowV2DeliverableContract;
use archon_workflow::v2::{
    declarative_floor_output::verdict_value, deliverable_contract::ContractRoots,
};
use archon_workflow::{collect_declarative_floor_facts, evaluate_floor_predicates};
use std::io::Read;

pub(crate) fn early_entry() -> Option<i32> {
    let mut arguments = std::env::args_os();
    let program = arguments.next()?;
    if arguments.next()?.to_str() != Some("workflow")
        || arguments.next()?.to_str() != Some("verify-deliverable")
    {
        return None;
    }
    #[derive(clap::Parser)]
    struct Input {
        #[command(flatten)]
        args: WorkflowVerifyDeliverableArgs,
    }
    use clap::Parser;
    let input = Input::parse_from(std::iter::once(program).chain(arguments));
    Some(run(&input.args))
}

pub(crate) fn run(args: &WorkflowVerifyDeliverableArgs) -> i32 {
    let mut text = args.contract_json.clone();
    if text == "-" {
        text.clear();
        if let Err(error) = std::io::stdin().read_to_string(&mut text) {
            eprintln!("could not read contract input: {error}");
            return 2;
        }
    }
    let raw: serde_json::Value = match serde_json::from_str(&text) {
        Ok(raw) => raw,
        Err(error) => {
            eprintln!("contract input is not valid JSON: {error}");
            return 2;
        }
    };
    let contract: WorkflowV2DeliverableContract = match serde_json::from_value(raw.clone()) {
        Ok(contract) => contract,
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"status":"failed","failure_count":1,"failures":[format!("declared deliverable contract does not parse: {error}")]})
            );
            return 1;
        }
    };
    let Some(roots) = ContractRoots::from_ordered(
        args.root
            .iter()
            .map(|path| path.to_string_lossy().into_owned()),
    ) else {
        eprintln!("contract input requires at least one resolution root");
        return 2;
    };
    let facts = match collect_declarative_floor_facts(&roots, &contract) {
        Ok(facts) => facts,
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"status":"failed","failure_count":1,"failures":[format!("host could not collect declared contract facts: {error}")]})
            );
            return 1;
        }
    };
    let verdict = match archon_workflow::v2::deliverable_contract::contract_defect(&raw) {
        Some(defect) => archon_workflow::FloorVerdict::Failed {
            findings: vec![defect],
        },
        None => evaluate_floor_predicates(&contract, &facts),
    };
    let passed = matches!(verdict, archon_workflow::FloorVerdict::Passed);
    println!("{}", verdict_value(&contract, &facts, verdict));
    i32::from(!passed)
}
