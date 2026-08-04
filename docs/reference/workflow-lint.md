# `archon workflow lint`

Runs four advisory analyses over a task set and prints what they found.

```
archon workflow lint --tasks <DIR>
archon workflow lint --spec-file <PATH>
archon workflow lint --graph <ID>
```

Also available in the TUI as `/workflow lint`, with the same three flags.

## Advisory means advisory

**Nothing here can fail a run.** The command reads a graph, runs pure analyses
over it, and prints the result. It never writes, never mutates a spec, and never
removes an edge it thinks is spurious. The exit status is success whether or not
findings were reported, because a finding is a question for the author, not a
verdict.

That is a construction, not a policy that could be tightened later: enforcement
is the [guardrail admission layer](../architecture/topology.md#guardrail-admission),
and it stays there. The lint and the enforcer do not share a code path.

Every finding carries its own remedy naming the specific nodes involved and what
to change. A lint that said "verifier diversity is low" and stopped would have
told the reader nothing they can act on.

## Choosing a source

Exactly one of the three flags must be given. Passing none is an error naming
all three rather than a guess at which was meant — a lint report is only useful
when you know what it is a report *of*.

| Flag | What it is | What it can conclude |
|---|---|---|
| `--tasks <DIR>` | A decomposed-PRD `TASK-*.md` directory | The only surface that declares dataflow on both sides — contracted artifacts out, named artifacts in — so it is the only one edge classification can conclude anything on. Also the only source with requirement coverage. |
| `--spec-file <PATH>` | A `WorkflowSpec` YAML file | Diamond conformance, which needs the roles and fan-out a spec carries. It declares no reads, so the dataflow lints stay silent. |
| `--graph <ID>` | A recorded graph under `.archon/topology/` | Coupling between concurrent nodes, visible here and nowhere else, because reads come from the `FileRead` records the tool tap emitted. |

For `--graph`, a declared `graph.json` and its trace are merged when both exist:
the declared shape wins because it carries authored roles and fan-out, and the
trace supplies the reads it is missing. That combination is the only one that
lets all the lints run at once. With no `graph.json`, the graph is reconstructed
from the trace alone (`src/command/topology_lint.rs:139`).

## Silence is not a clean bill of health

Sections appear in a fixed order regardless of what was found, and a section
with nothing to say says so. This matters because two of the analyses stay
deliberately silent on graphs that declare no dataflow, and a report that
omitted them would be indistinguishable from one where they had run and found
nothing.

So you will see lines like:

```
no node declares what it consumes, so no edge can be classified.
no reduce stage has any verification feeding it — nothing to check.
```

Those mean *the lint looked and had nothing to work with*, which is a different
fact from *no findings*.

## The four analyses

### Diamond conformance

Asks whether a fan-out reaches its reducer through independent verification, and
whether the verifiers are actually different from one another
(`crates/archon-topology/src/analysis/diamond.rs:1`). Three findings:

- **`UnverifiedFanout`** — a fan-out is folded by a reducer with no verification
  stage on any path between them. The branches are merged on their own say-so.
- **`SoleVerifier`** — a reducer whose entire verification is one stage. One
  reviewer is not a panel; there is no second opinion to disagree with the first.
- **`HomogeneousVerifiers`** — a reducer with several verifiers that all name the
  same agent. Repeating one reviewer is not adversarial review, and the
  correlated failure — the thing that agent cannot see — survives every one of
  them.

The reducer considered is the *nearest* one walking forward, not any reachable
one. On a real graph almost every reducer is transitively reachable from almost
every fan-out, so the broader question reports the same fan-out once per
downstream stage and says nothing useful.

Alongside the findings the section prints a diversity line per reducer: how many
verifiers feed it and how many distinct agents they name.

Silent when no reduce stage has any verification feeding it.

### Edge classification

Splits every declared `depends_on` edge into three classes
(`crates/archon-topology/src/analysis/edge_support.rs:95`):

- **`Dataflow`** — the dependent consumes something the dependency produces.
  Printed only as a count. These are the expected case and listing them would
  bury the rest.
- **`OrderingOnly`** — the dependency produces no artifact, only source, and the
  dependent consumes artifacts. Nothing flows and nothing should; code must
  exist before it runs. Listed under an explicit heading saying they are **not
  findings**, so the reader can tell "the lint looked and concluded this edge is
  fine" from "the lint did not look". Reporting these as defects is what made an
  earlier version of this lint untrustworthy on a real corpus.
- **`Unsupported`** — the dependency produces artifacts the dependent consumes
  none of. The only findings. Each prints what the dependency produced, what the
  dependent consumed, and a remedy. Where many tasks wait on one producer that
  is contracted to produce artifacts nobody declares consuming, the cause is
  named as `UnderDeclaredProducer`, since under-declaration on the producing
  side explains all those edges at once where dropping them explains none.
  Otherwise the cause is `Undetermined`: both possibilities are named and
  neither is ranked.

Nothing is classified unless *both* ends declared something — the dependency
must declare what it writes and the dependent what it reads or consumes. Empty
means unknown, so a graph lowered from `Vec<Subtask>` yields no classified edges
at all rather than an opinion about every edge in it.

### Stop-rule fusion

Where the graph's parallel/sequential split disagrees with its dataflow. Two
symmetrical mistakes, and it reports both
(`crates/archon-topology/src/analysis/fusion.rs:1`):

- **Coupling** — two nodes the graph permits to run at the same time, where one
  reads a target the other writes. Nothing orders them, so the reader sees the
  file before or after the write depending on scheduling. That work is not
  parallel; it only looks parallel.
- **Slack** — two nodes the graph orders, where the downstream one consumes
  nothing the upstream one produces. The barrier between them buys nothing. When
  the two also share a role and an agent they are one stage split in two
  (`Fuse`); otherwise they are two stages that could run at once
  (`Parallelise`).

This overlaps deliberately with edge classification and is kept separate because
the remedies differ: edge classification questions an edge or the declaration
behind it, while fusion merges or re-levels two stages. Fusion fires only on a
degenerate chain — sole predecessor, sole successor — where the remedy is
mechanical. Both can fire on one pair.

Targets are truncated after four with a `(+N more)` suffix. A node in a real
corpus declares dozens, and a report nobody reads to the end is a report that
found nothing.

### Requirement coverage

Not a graph analysis. It compares the task files' `implements:` claims against
the requirement IDs of the PRD they name, which is why it takes the task
directory rather than the lowered graph, and why it has anything to say only for
`--tasks` (`src/command/topology_lint/coverage.rs:1`).

Two pure set operations, implementing the Decomposition Completeness Gate:

- every requirement is claimed by at least one task's `implements:` list;
- no task cites an ID the PRD does not define.

A requirement no task claims is a decomposition gap. An ID no PRD defines is a
typo or a stale reference. Both are reported, never raised.

A normative requirement is a line whose first non-space content is `- ` or `* `
followed immediately by a `REQ-<AREA>-<NNN>` id. An ID buried mid-paragraph is
invisible to the check — deliberately, since otherwise it would pass coverage by
never being counted.

**Finding the PRD.** The check needs the PRD but is given a task directory, so
two conventions are tried: the PRD beside the task directory named for the same
PRD id, and the repository-level `prds/<PRD-id>/` root that `/workflow-prd`
writes to. Each task also names its PRD in `prd:`, which supplies the id when
the directory name does not. When none of those paths exists, the section says
which paths it tried and stops. It does **not** fall back to scanning for any
markdown file containing requirement IDs: a coverage report computed against the
wrong document is worse than none, because it looks like one.

For `--spec-file` and `--graph` the section prints why it was not computed rather
than staying silent.

## Report shape

```
topology lint — <subject>
<N> nodes, advisory only: nothing here blocks a run

## diamond conformance
## dependency edges
## stop-rule fusion
## requirement coverage
```

## Related

- [Topology](../architecture/topology.md) — the `TaskGraph` IR these analyses run
  over, and the admission layer that does the enforcing.
- [`archon requirements trace`](requirements-trace.md) — requirement-to-code
  traceability with a proof ladder.
- [Dynamic workflows](../architecture/dynamic-workflows.md) — where a
  `WorkflowSpec` comes from.
