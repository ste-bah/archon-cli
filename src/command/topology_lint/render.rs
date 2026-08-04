//! Turning findings into text somebody can act on.
//!
//! Every line names the specific nodes involved and states what to change.
//! A lint that says "verifier diversity is low" and stops has told the reader
//! nothing they can do, so each finding carries its own `remedy()` from the
//! analysis crate and this module only arranges them.

use anyhow::Result;
use archon_topology::EdgeSupport;
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
    out.push_str(&edge_support_section(graph)?);
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

/// Declared edges, split by what supports them.
///
/// Three classes, printed differently on purpose. `Dataflow` edges are silent —
/// they are the expected case and listing them buries the rest. `OrderingOnly`
/// edges are listed under an explicit heading that says they are not findings,
/// because the reader has to be able to tell "the lint looked and concluded
/// this edge is fine" apart from "the lint did not look". Only `Unsupported`
/// edges are findings, and each carries the remedy the analysis wrote, which
/// names both candidate causes rather than assuming the dependent is at fault.
fn edge_support_section(graph: &TaskGraph) -> Result<String> {
    let edges = graph.classify_edges()?;
    let mut out = String::from("\n## dependency edges\n");
    let declared = graph
        .nodes
        .iter()
        .filter(|node| node.consumption_is_known())
        .count();
    if declared == 0 {
        out.push_str("  no node declares what it consumes, so no edge can be classified.\n");
        return Ok(out);
    }

    let dataflow = edges
        .iter()
        .filter(|edge| edge.support == EdgeSupport::Dataflow)
        .count();
    let ordering: Vec<_> = edges
        .iter()
        .filter(|edge| edge.support == EdgeSupport::OrderingOnly)
        .collect();
    let unsupported: Vec<_> = edges.iter().filter(|edge| edge.is_defect()).collect();

    out.push_str(&format!(
        "  {} edge(s) classified across {declared} node(s) with declared consumption: \
         {dataflow} carrying dataflow, {} ordering-only, {} unsupported.\n",
        edges.len(),
        ordering.len(),
        unsupported.len()
    ));

    if !ordering.is_empty() {
        out.push_str("  ordering-only (not findings — code must exist before it runs):\n");
        for edge in ordering {
            out.push_str(&format!(
                "    {} -> {}: {}\n        {}\n",
                edge.dependent,
                edge.dependency,
                edge.headline(),
                edge.remedy()
            ));
        }
    }

    if unsupported.is_empty() {
        out.push_str("  no unsupported edges.\n");
        return Ok(out);
    }
    for edge in unsupported {
        out.push_str(&format!(
            "  {} -> {}: {}\n",
            edge.dependent,
            edge.dependency,
            edge.headline()
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
