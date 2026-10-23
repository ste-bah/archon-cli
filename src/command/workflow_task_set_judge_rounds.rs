use super::incomplete::JudgeIncompleteKind;
use super::*;

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
    task: &str,
    model: &str,
    validate: impl Fn(&AcceptanceContract) -> Result<()>,
    partial: Option<&PartialReply<'_>>,
    roots: &JudgeRoots,
) -> Result<AcceptanceContract> {
    let mut messages = vec![serde_json::json!({"role":"user","content":task})];
    let mut judged_contract = contract;
    let mut pending_contract = judged_contract.clone();
    let mut current_task = task.to_string();
    let mut seen = BTreeSet::new();
    let mut no_progress = 0usize;
    let mut attempts = 0usize;
    let mut last: String;
    // Whose the last unusable reply was, typed where it was produced.
    let mut last_kind: JudgeIncompleteKind;
    // The ids this call decided, so an incomplete end hands them back.
    let mut decided = BTreeSet::new();
    loop {
        attempts += 1;
        let content = match complete_reply(client, &messages, model, partial).await? {
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
                messages = vec![serde_json::json!({"role":"user","content":current_task})];
                continue;
            }
        };
        // A batch that parses but leaves a verdict field empty is the same kind
        // of slip as one that will not parse, so it is re-asked rather than
        // ending the freeze: the judged shape is what makes a batch usable.
        let mut attempt = pending_contract.clone();
        let judged = apply_judgments(&mut attempt, &content).and_then(|missing| {
            let missing_set = missing;
            let answered = subset_without_ids(&attempt, &missing_set);
            // The final round is validated as the whole judged contract, so a
            // structure check that needs every id sees the merged decisions,
            // not only the ids this round answered.
            let validation = if missing_set.is_empty() {
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
            Ok((missing, answered)) => {
                merge_judged_entries(&mut judged_contract, &answered);
                decided.extend(
                    (answered.acceptance.iter())
                        .chain(&answered.supplementary)
                        .map(|entry| entry.id.clone()),
                );
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
                    }
                    return Ok(judged_contract);
                }
                pending_contract = subset_with_ids(&judged_contract, &missing);
                current_task = batched_judge_prompt_with_roots(&pending_contract, roots)?;
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
    incomplete.decided = (judged_contract.acceptance.into_iter())
        .chain(judged_contract.supplementary)
        .filter(|entry| decided.contains(&entry.id))
        .collect();
    Err(incomplete.into())
}
