//! Turning findings into text somebody can act on.
//!
//! Every line names the specific nodes involved and states what to change.
//! A lint that says "verifier diversity is low" and stops has told the reader
//! nothing they can do, so each finding carries its own `remedy()` from the
//! analysis crate and this module only arranges them.

use anyhow::Result;
use archon_topology::ir::{TaskGraph, WriteTarget};

/// Render the full report for `graph`.
///
/// Sections appear in a fixed order regardless of what was found, and a section
/// with nothing to say says so. A report that silently omits a lint is
/// indistinguishable from one where the lint did not run, and the difference
/// matters: two of these three lints stay deliberately silent on graphs that
/// declare no dataflow, and the reader has to be able to tell that apart from a
/// clean bill of health.
pub(super) fn report(graph: &TaskGraph, subject: &str) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!(
        "topology lint — {subject}\n{} nodes, advisory only: nothing here blocks a run\n",
        graph.len()
    ));

    out.push_str(&diamond_section(graph)?);
    out.push_str(&fake_edge_section(graph)?);
    out.push_str(&fusion_section(graph)?);
    Ok(out)
}

fn diamond_section(graph: &TaskGraph) -> Result<String> {
    let report = graph.diamond_conformance()?;
    let mut out = String::from("\n## diamond conformance\n");
    if report.diversity.is_empty() && report.findings.is_empty() {
        out.push_str("  no reduce stage has any verification feeding it — nothing to check.\n");
        return Ok(out);
    }
    for score in &report.diversity {
        out.push_str(&format!(
            "  {}: {} verifier(s) [{}], {} distinct agent(s)\n",
            score.reducer,
            score.verifiers.len(),
            score.verifiers.join(", "),
            score.distinct_agents
        ));
    }
    if report.findings.is_empty() {
        out.push_str("  no findings.\n");
        return Ok(out);
    }
    for finding in &report.findings {
        out.push_str(&format!("  [{}] {}\n", finding.subject(), finding.remedy()));
    }
    Ok(out)
}

fn fake_edge_section(graph: &TaskGraph) -> Result<String> {
    let edges = graph.fake_edges()?;
    let mut out = String::from("\n## fake edges\n");
    let declared = graph
        .nodes
        .iter()
        .filter(|node| node.consumption_is_known())
        .count();
    if declared == 0 {
        out.push_str("  no node declares what it consumes, so no edge can be shown unjustified.\n");
        return Ok(out);
    }
    if edges.is_empty() {
        out.push_str(&format!(
            "  no findings across {declared} node(s) with declared consumption.\n"
        ));
        return Ok(out);
    }
    for edge in &edges {
        out.push_str(&format!(
            "  {} -> {}: {}\n",
            edge.dependent, edge.dependency, "no declared dataflow"
        ));
        out.push_str(&format!(
            "      produced: {}\n      consumed: {}\n      {}\n",
            render_targets(&edge.produced),
            render_targets(&edge.consumed),
            edge.remedy()
        ));
    }
    Ok(out)
}

fn fusion_section(graph: &TaskGraph) -> Result<String> {
    let report = graph.stop_rule_fusion()?;
    let mut out = String::from("\n## stop-rule fusion\n");
    if report.is_clean() {
        out.push_str("  no findings.\n");
        return Ok(out);
    }
    for pair in &report.coupled {
        out.push_str(&format!(
            "  coupled: {} reads {} written by {}\n      {}\n",
            pair.reader,
            render_targets(&pair.targets),
            pair.writer,
            pair.remedy()
        ));
    }
    for chain in &report.fusible {
        out.push_str(&format!(
            "  {:?}: {} -> {}\n      {}\n",
            chain.kind,
            chain.upstream,
            chain.downstream,
            chain.remedy()
        ));
    }
    Ok(out)
}

/// Targets, truncated. A node in the real corpus declares dozens, and a report
/// nobody reads to the end is a report that found nothing.
fn render_targets(targets: &[WriteTarget]) -> String {
    const SHOWN: usize = 4;
    let rendered: Vec<String> = targets.iter().take(SHOWN).map(render_target).collect();
    if targets.len() > SHOWN {
        format!("{} (+{} more)", rendered.join(", "), targets.len() - SHOWN)
    } else if rendered.is_empty() {
        "none declared".to_string()
    } else {
        rendered.join(", ")
    }
}

fn render_target(target: &WriteTarget) -> String {
    match target {
        WriteTarget::Path(path) => path.clone(),
        WriteTarget::Artifact(key) => format!("artifact:{key}"),
    }
}
