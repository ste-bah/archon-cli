impl WorkflowV2ResultStore {
    /// Invalidate the calls a stage restart revokes, and clear every branch
    /// outcome for them. The complete branch move plan is built before the
    /// first call record or checkpoint is changed.
    pub fn invalidate_calls_and_clear_branches(
        &self,
        executions: &[super::WorkflowV2CallExecution],
        call_id: &str,
    ) -> WorkflowResult<Vec<String>> {
        let mut invalidated = downstream_call_ids(executions, call_id);
        let records = self.load_call_record_history()?;
        invalidated.extend(dynamic_wave_invalidated_call_ids(&records, call_id));

        let mut plans = Vec::new();
        for id in &invalidated {
            plans.push((id.clone(), self.plan_all_branch_outcomes_for_call(id)?));
        }

        for id in &invalidated {
            self.invalidate_call_everywhere(id, call_id)?;
        }
        if let Some(mut checkpoint) = self.load_checkpoint()? {
            checkpoint.remove_completed(&invalidated);
            self.save_checkpoint(&checkpoint)?;
        }

        let mut output = invalidated.clone();
        for (id, plan) in plans {
            let count = plan.iter().map(|(_, files)| files.len()).sum::<usize>();
            self.execute_item_revocation(plan)?;
            if count > 0 {
                output.insert(format!("{id}:branches({count})"));
            }
        }
        Ok(output.into_iter().collect())
    }

    fn plan_all_branch_outcomes_for_call(
        &self,
        call_id: &str,
    ) -> WorkflowResult<Vec<(String, Vec<PathBuf>)>> {
        let hashed = self.branch_call_dir(call_id);
        let legacy = self.root.join("branches").join(sanitize_call_id(call_id));
        let mut items = std::collections::BTreeSet::new();
        for dir in [&hashed, &legacy] {
            if dir.exists() {
                items.extend(
                    stored_outcomes_in(dir)?
                        .into_iter()
                        .map(|(_, outcome)| outcome.item_id),
                );
            }
        }
        self.plan_item_revocation(call_id, &items.into_iter().collect::<Vec<_>>())
    }
}
