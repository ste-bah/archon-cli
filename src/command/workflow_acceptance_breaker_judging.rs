//! Rotate pairs between pending judgments.
use super::*;
pub(super) async fn run(
    ctx: &mut Context<'_>,
    pairs: &mut [PairVerdict],
    states: &mut [Vec<ProofState>],
    all: &[MutantRun],
    retried: &BTreeSet<(String, String)>,
    store: Option<&Path>,
    distinct: bool,
    order: &[usize],
) {
    for _ in 0..=all.len() {
        let marks: usize = pairs
            .iter()
            .map(|p| p.relevance.len() + p.equivalence.len())
            .sum();
        for &index in order {
            let pair = &mut pairs[index];
            if !retried.contains(&(pair.task_id.clone(), pair.id.clone())) {
                continue;
            }
            ctx.circuit.pair = index;
            let runs: Vec<_> = all
                .iter()
                .filter(|r| r.task_id == pair.task_id && r.requirement_id == pair.id)
                .cloned()
                .collect();
            if runs.is_empty() {
                continue;
            }
            match view(ctx, index).await {
                Ok(view) => {
                    if let Some(text) = rule::requirement_text(ctx.req.prd_text, &pair.id) {
                        judges::relevance(ctx, pair, &view, &runs, store, &text).await;
                        judges::equivalence(ctx, pair, &view, &runs, store, &text).await;
                    }
                    if let Err(e) = view.remove() {
                        ctx.verdicts
                            .refusals
                            .push(format!("view_remove_failed: {e}"));
                    }
                }
                Err(e) => {
                    ctx.verdicts.refusals.push(format!(
                        "judge_view_unavailable: {} {}: {e}",
                        pair.task_id, pair.id
                    ));
                    states[index].push(ProofState::HostUnrun);
                }
            }
            for r in runs.iter().filter(|r| r.counted) {
                let ids = pair.killed_by.entry(r.mutant_id.clone()).or_default();
                if !ids.contains(&r.check_id) {
                    ids.push(r.check_id.clone());
                }
            }
            pair.verdict = rule::decide(rule::Decision {
                refused: false,
                has_text: pair.requirement_text_digest.is_some(),
                states: &states[index],
                pair,
                runs: &runs,
                distinct,
            });
        }
        if pairs
            .iter()
            .map(|p| p.relevance.len() + p.equivalence.len())
            .sum::<usize>()
            == marks
        {
            break;
        }
    }
}
