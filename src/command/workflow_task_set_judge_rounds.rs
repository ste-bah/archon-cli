use super::incomplete::JudgeIncompleteKind;
use super::*;

/// Added to a fresh ask after a reply cut off without a usable document
/// (#552). The judge samples at temperature 0, so the same prompt again
/// would repeat the same reply; the whole task is kept.
const JSON_ONLY_REASK: &str = "\n\nYour previous reply to this request was cut off before it gave a usable JSON document. Reply with the JSON document only, with no reasoning or other text before or after it.";

/// The checks of `contract` this judge call left without a verdict (#552).
fn undecided(contract: &AcceptanceContract, decided: &BTreeSet<String>) -> Vec<String> {
    (contract.acceptance.iter().chain(&contract.supplementary))
        .filter(|entry| !decided.contains(&entry.id))
        .map(|entry| entry.id.clone())
        .collect()
}

/// `error`, when it is an incomplete judge that names no check, naming the
/// checks of `contract` not in `decided`.
fn naming_undecided(
    mut error: anyhow::Error,
    contract: &AcceptanceContract,
    decided: &BTreeSet<String>,
) -> anyhow::Error {
    if let Some(incomplete) = error.downcast_mut::<JudgeIncomplete>()
        && incomplete.undecided.is_empty()
    {
        incomplete.undecided = undecided(contract, decided);
    }
    error
}

/// Copy every entry `answered` judged into `contract`, matched by id.
pub(super) fn merge_judged_entries(
    contract: &mut AcceptanceContract,
    answered: &AcceptanceContract,
) {
    for entry in answered.acceptance.iter().chain(&answered.supplementary) {
        if let Some(destination) = contract
            .acceptance
            .iter_mut()
            .chain(&mut contract.supplementary)
            .find(|destination| destination.id == entry.id)
        {
            *destination = entry.clone();
        }
    }
}

pub(super) async fn judge_prompted_with_roots(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    prompt: impl Fn(&AcceptanceContract) -> Result<String>,
    model: &str,
    validate: impl Fn(&AcceptanceContract) -> Result<()>,
    partial: Option<&PartialReply<'_>>,
    roots: &super::JudgeRoots,
) -> Result<AcceptanceContract> {
    judge_rounds(
        client,
        contract,
        prompt,
        model,
        validate,
        partial,
        roots,
        |_| false,
        true,
    )
    .await
}

/// A known refutation is ready for its author before the next provider call.
/// An incomplete result carries exactly the decided entries; all others remain pending.
pub(crate) async fn judge_refutations(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    prompt: impl Fn(&AcceptanceContract) -> Result<String>,
    model: &str,
    refutes: impl Fn(&AcceptanceContract) -> bool,
) -> Result<AcceptanceContract> {
    judge_rounds(
        client,
        contract,
        prompt,
        model,
        require_judged_prose,
        None,
        &super::JudgeRoots::default(),
        refutes,
        false,
    )
    .await
}

async fn judge_rounds(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    prompt: impl Fn(&AcceptanceContract) -> Result<String>,
    model: &str,
    validate: impl Fn(&AcceptanceContract) -> Result<()>,
    partial: Option<&PartialReply<'_>>,
    roots: &super::JudgeRoots,
    refutes: impl Fn(&AcceptanceContract) -> bool,
    screen_refutations: bool,
) -> Result<AcceptanceContract> {
    let task = prompt(&contract)?;
    let mut messages = vec![serde_json::json!({"role":"user","content":task})];
    let mut judged_contract = contract;
    let mut pending_contract = judged_contract.clone();
    let mut current_task = task;
    let mut seen = BTreeSet::new();
    let mut no_progress = 0usize;
    let mut attempts = 0usize;
    let mut last: String;
    // Whose the last unusable reply was, typed where it was produced.
    let mut last_kind: JudgeIncompleteKind;
    // The ids this call decided, so an incomplete end hands them back.
    let mut decided = BTreeSet::new();
    let mut screen = screen::Screen::default();
    loop {
        attempts += 1;
        let reply = complete_reply(client, &messages, model, partial)
            .await
            .map_err(|error| naming_undecided(error, &judged_contract, &decided))?;
        let content = match reply {
            Ok(content) => content,
            Err((unusable, kind)) => {
                last = unusable.clone();
                last_kind = kind;
                let class = judge_error_class(&unusable);
                if seen.insert(class) {
                    no_progress = 0;
                } else {
                    no_progress += 1;
                }
                if no_progress >= JUDGE_NO_PROGRESS_WINDOW {
                    break;
                }
                // `complete_reply` returns this path only when there is no
                // complete reply to quote (for example, a truncation before
                // JSON starts). Ask afresh; parsed rejections below preserve
                // the actual assistant reply and exact validation error.
                // A reply cut off before a usable document is asked with a
                // JSON-only instruction, so the ask differs (#552).
                let ask = if kind == JudgeIncompleteKind::AuthoredReply {
                    format!("{current_task}{JSON_ONLY_REASK}")
                } else {
                    current_task.clone()
                };
                messages = vec![serde_json::json!({"role":"user","content":ask})];
                continue;
            }
        };
        // A batch that parses but leaves a verdict field empty is the same kind
        // of slip as one that will not parse, so it is re-asked rather than
        // ending the freeze: the judged shape is what makes a batch usable.
        let mut attempt = pending_contract.clone();
        let judged = (if screen_refutations {
            screen.apply(&mut attempt, &content, roots)
        } else {
            screen::apply_unscreened(&mut attempt, &content)
        })
        .and_then(|missing| {
            let missing_set = missing;
            let answered = subset_without_ids(&attempt, &missing_set);
            // The final round is validated as the whole judged contract, so a
            // structure check that needs every id sees the merged decisions,
            // not only the ids this round answered.
            let completes_contract = missing_set.is_empty()
                && undecided(&judged_contract, &decided).iter().all(|id| {
                    answered
                        .acceptance
                        .iter()
                        .chain(&answered.supplementary)
                        .any(|entry| &entry.id == id)
                });
            let validation = if completes_contract {
                let mut merged = judged_contract.clone();
                merge_judged_entries(&mut merged, &answered);
                validate(&merged)
            } else {
                require_judged_prose(&answered)
            };
            validation.map(|()| (missing_set, answered))
        });
        // An unusable reply is spent with its credit. A usable reply keeps
        // its continuation until JudgeStore durably saves the verdicts.
        if judged.is_err()
            && let Some(partial) = partial
        {
            partial.spent();
        }
        match judged {
            Ok((_missing, answered)) => {
                merge_judged_entries(&mut judged_contract, &answered);
                decided.extend(
                    (answered.acceptance.iter())
                        .chain(&answered.supplementary)
                        .map(|entry| entry.id.clone()),
                );
                let missing: BTreeSet<_> =
                    undecided(&judged_contract, &decided).into_iter().collect();
                last_kind = JudgeIncompleteKind::AuthoredReply;
                if missing.is_empty() {
                    for entry in judged_contract
                        .acceptance
                        .iter_mut()
                        .chain(&mut judged_contract.supplementary)
                    {
                        entry.judgment.sampling = Some(serde_json::json!({
                            "temperature": 0.0, "model": client.resolve_model_alias(model),
                            "provider": client.provider_id(),
                        }));
                        crate::command::workflow_task_set::reauthor::cache::stamp_judgment_with_content_digest(
                            entry,
                            &client.resolve_model_alias(model),
                            client.provider_id().as_deref(),
                            roots,
                        );
                    }
                    return Ok(judged_contract);
                }
                if refutes(&answered) {
                    let mut incomplete = JudgeIncomplete::new(
                        JudgeIncompleteKind::AuthoredReply,
                        "known refutations require repair; remaining checks are undecided",
                    );
                    incomplete.undecided = undecided(&judged_contract, &decided);
                    incomplete.decided = judged_contract
                        .acceptance
                        .into_iter()
                        .chain(judged_contract.supplementary)
                        .filter(|entry| decided.contains(&entry.id))
                        .collect();
                    return Err(incomplete.into());
                }
                pending_contract = subset_with_ids(&judged_contract, &screen.next_ids(&missing));
                let correction = screen.instruction(&pending_contract);
                if !correction.is_empty()
                    && let Some(partial) = partial
                {
                    partial.spent();
                }
                current_task = format!("{}{}", prompt(&pending_contract)?, correction);
                messages = vec![serde_json::json!({"role":"user","content":current_task})];
                last = format!(
                    "acceptance judge left decisions missing={:?}; retry only those checks",
                    missing.iter().collect::<Vec<_>>()
                );
                let made_progress =
                    !answered.acceptance.is_empty() || !answered.supplementary.is_empty();
                let class = judge_error_class(&last);
                if made_progress || seen.insert(class) {
                    no_progress = 0;
                } else {
                    no_progress += 1;
                }
                if no_progress >= JUDGE_NO_PROGRESS_WINDOW {
                    break;
                }
            }
            Err(error) => {
                last = format!("{error:#}");
                last_kind = JudgeIncompleteKind::AuthoredReply;
                let class = judge_error_class(&last);
                if seen.insert(class) {
                    no_progress = 0;
                } else {
                    no_progress += 1;
                }
                if no_progress >= JUDGE_NO_PROGRESS_WINDOW {
                    break;
                }
                let said = if content.trim().is_empty() {
                    "(an empty reply)"
                } else {
                    &content
                };
                messages = vec![
                    serde_json::json!({"role":"user","content":current_task}),
                    serde_json::json!({"role":"assistant","content":said}),
                    serde_json::json!({"role":"user","content":format!("Your reply was rejected: {last}\nReply again with the complete JSON document required by the first message.")}),
                ];
            }
        }
    }
    let mut incomplete = JudgeIncomplete::new(
        last_kind,
        format!(
            "{attempts} replies; the last {JUDGE_NO_PROGRESS_WINDOW} gave no new first-error class or usable verdict; the last: {last}"
        ),
    );
    incomplete.undecided = undecided(&judged_contract, &decided);
    incomplete.decided = (judged_contract.acceptance.into_iter())
        .chain(judged_contract.supplementary)
        .filter(|entry| decided.contains(&entry.id))
        .collect();
    Err(incomplete.into())
}
