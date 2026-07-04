  function sourceArtifactRequirements(item) {
    return array(item && item.artifact_requirements);
  }

  function noopOutcomeAccepted(outcome) {
    return outcome && (outcome.status === "accepted" || outcome.status === "noop");
  }

  function outcomeCurrentEvidenceValues(outcome) {
    return []
      .concat(array(outcome && outcome.artifacts))
      .concat(array(outcome && outcome.artifact_paths))
      .concat(array(outcome && outcome.artifacts_checked))
      .concat(array(outcome && outcome.current_artifacts_checked))
      .concat(array(outcome && outcome.commands_run))
      .concat(array(outcome && outcome.current_commands_run))
      .concat(array(outcome && outcome.completion_evidence));
  }

  function outcomeHasNoopSourceEvidence(sourceItem, outcome) {
    if (!noopOutcomeAccepted(outcome) || !hasConcreteEvidence(outcome)) {
      return false;
    }
    const requirements = sourceArtifactRequirements(sourceItem);
    if (requirements.length === 0) {
      return true;
    }
    return outcomeCurrentEvidenceValues(outcome).length > 0;
  }

  function matchingAcceptedNoopIds(sourceItems, outcomes) {
    const accepted = new Set();
    for (const item of array(sourceItems)) {
      for (const outcome of array(outcomes)) {
        if (!outcomeHasNoopSourceEvidence(item, outcome)) {
          continue;
        }
        const sourceIds = new Set(canonicalIdsFor(item));
        for (const id of canonicalIdsFor(outcome)) {
          if (sourceIds.has(id)) {
            accepted.add(id);
          }
        }
      }
    }
    return Array.from(accepted);
  }

  function matchingAcceptedCompletionIds(sourceItems, outcomes) {
    const accepted = [];
    for (const item of array(sourceItems)) {
      const ids = workTypeFor(item) === "verified_noop"
        ? matchingAcceptedNoopIds([item], outcomes)
        : matchingAcceptedIds([item], outcomes);
      for (const id of ids) {
        if (!accepted.includes(id)) {
          accepted.push(id);
        }
      }
    }
    return accepted;
  }
