#[path = "../../tests/support/workflow_decomposition_offline_server.rs"]
#[allow(dead_code)]
mod offline_provider;

#[test]
fn offline_provider_content_variants_keep_their_fixture_contracts() {
    let user_prompt = |content: String| {
        serde_json::json!({
            "messages": [{"role": "user", "content": content}]
        })
        .to_string()
    };
    let acceptance_prompt = |id: &str, criterion: &str| {
        user_prompt(format!(
            "Author exactly one acceptance entry identified below. Author ONLY entry {id}: {criterion}"
        ))
    };
    let (role, reply) = offline_provider::variants::answer(
        &acceptance_prompt(
            "AC-X-001",
            "The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true.",
        ),
        offline_provider::Variant::SecretInAcceptanceReply,
    );
    assert_eq!(role, "acceptance");
    assert!(reply.contains(offline_provider::variants::SECRET_TOKEN));

    for (variant, id, criterion, script) in [
        (
            offline_provider::Variant::CrlfCheck,
            "SUP-REQ-X-001",
            "SUPPLEMENTARY check owed to PRD requirement REQ-X-001 (no other check covers it): The first task creates `src/alpha.txt` with the nonempty line `alpha ready`.",
            offline_provider::variants::CRLF_SCRIPT,
        ),
        (
            offline_provider::Variant::HeredocCheck,
            "SUP-REQ-X-002",
            "SUPPLEMENTARY check owed to PRD requirement REQ-X-002 (no other check covers it): The second task depends on the first and creates `src/beta.json` as an object whose `ready` field is true.",
            offline_provider::variants::HEREDOC_SCRIPT,
        ),
    ] {
        let (role, reply) =
            offline_provider::variants::answer(&acceptance_prompt(id, criterion), variant);
        assert_eq!(role, "acceptance");
        assert!(reply.contains(script));
    }

    let (role, _) = offline_provider::variants::answer(
        &acceptance_prompt(
            "AC-X-002",
            "The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready_002` set to true.",
        ),
        offline_provider::Variant::Large(2),
    );
    assert_eq!(role, "acceptance");
    let prd = offline_provider::variants::large::prd("# fixture\n", 2);
    assert!(prd.contains("AC-X-002"));
    let outcome: serde_json::Value =
        serde_json::from_str(&offline_provider::variants::large::outcome(2)).unwrap();
    assert_eq!(outcome["ready"], true);
    assert_eq!(outcome["ready_002"], true);
}

#[test]
fn refute_first_judge_provider_scripts_acceptance_reauthor_requests() {
    let prompt = serde_json::json!({
        "messages": [{"role": "user", "content": concat!(
            "## Pipeline Agent Run\n",
            "Agent: acceptance-reauthor (acceptance reauthor)\n",
            "Re-author exactly one acceptance entry of a frozen acceptance contract: AC-X-001.\n",
            "Return one JSON object with id, criterion, check, gap_permitted, judgment.\n",
            "The entry being replaced: {\"id\":\"AC-X-001\",\"criterion\":\"ready\",\"check\":{\"kind\":\"command\",\"command\":\"/old/root/.archon/proof/outcome.json\",\"cwd\":\"project_root\"},\"gap_permitted\":false}"
        )}]
    })
    .to_string();

    let (role, answer) =
        offline_provider::variants::answer(&prompt, offline_provider::Variant::RefuteFirstJudge);
    assert_eq!(role, "reauthor");
    let entry: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(entry["id"], "AC-X-001");
    assert_eq!(entry["check"]["cwd"], "project_root");
    assert!(entry["check"]["command"].as_str().is_some_and(|command| {
        command.contains(".archon/proof/outcome.json")
            && command.contains("json.load(open(sys.argv[1]))[\"ready\"] is True")
            && !command.starts_with('/')
    }));
    assert_eq!(
        offline_provider::variants::request_key(role, &prompt),
        "reauthor:AC-X-001"
    );
    let judge_prompt = serde_json::json!({
        "messages": [{"role": "user", "content": concat!(
            "Adversarially judge every acceptance check below. The toolchain is fixed: ",
            "Checks: [{\"id\":\"AC-X-001\"}]"
        )}]
    })
    .to_string();
    let judged = offline_provider::variants::refute_first_judge_answer(
        "judge",
        &judge_prompt,
        serde_json::json!({"decisions": [{"id": "AC-X-001", "verdict": "accepted"}]}).to_string(),
        offline_provider::Variant::RefuteFirstJudge,
        1,
    );
    let judged: serde_json::Value = serde_json::from_str(&judged).unwrap();
    assert_eq!(judged["decisions"][0]["verdict"], "refuted");

    let later_script_judge = offline_provider::variants::refute_first_judge_answer(
        "judge",
        &judge_prompt,
        serde_json::json!({"decisions": [{"id": "AC-X-001", "verdict": "accepted"}]}).to_string(),
        offline_provider::Variant::RefuteFirstJudge,
        4,
    );
    let later_script_judge: serde_json::Value = serde_json::from_str(&later_script_judge).unwrap();
    assert_eq!(later_script_judge["decisions"][0]["verdict"], "accepted");
}
