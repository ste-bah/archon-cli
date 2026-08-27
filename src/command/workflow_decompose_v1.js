async function workflow(w) {
  const authored = await w.agent("acceptance-author-1", {
    task: `Author the acceptance contract candidate for the PRD at ${args.prdPath}. Return only the complete candidate artifact.`,
    tier: "planner",
    resultMode: "rawOutcome"
  });
  return authored;
}
