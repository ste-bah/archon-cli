# `archon requirements trace`

Traces the requirement IDs in a PRD to the code that implements them, and
reports what each link has actually *earned* rather than whether one exists.

```
archon requirements trace --prd <PATH> --tasks <DIR> [options]
```

Also available in the TUI as `/requirements trace` (alias `/reqs`).

The command is read-only by default and its exit status is success whichever way
the report comes out. An unproven requirement is a **declared residual gap**,
not a failure: a traceability report that failed CI would be muted within a
week, and calling an unproven link satisfied is the exact mistake the proof
ladder exists to prevent.

## The proof ladder

Four levels, ordered, defined in
`crates/archon-knowledge/src/traceability/ladder.rs:45`:

| Level | What earns it |
|---|---|
| `Unproven` | The fail-closed floor. No live anchor — none was found, or every anchor is stale. |
| `Candidate` | An anchor exists and its file still hashes to what was recorded. Cheap, and not proof. |
| `Exercised` | A verifier the task itself named ran and passed, **and** the ambient trace shows that run read the anchored file. |
| `Falsifiable` | Breaking the anchored code breaks the verifier. Planned always, executed only under `--falsify`. |

Only `Exercised` and `Falsifiable` satisfy a promotion gate. That decision lives
in exactly one place — `ProofLevel::satisfies_promotion_gate` (`ladder.rs:60`) —
and it is a match on an enum, not a threshold on a score. Relevance scores from
the code index are carried so a reader can see the ordering the index produced;
nothing reads them to decide anything.

### Why `Exercised` needs two independent facts

Promotion requires both a passing named verifier and a trace record showing that
run reading the anchored file. One command's trace cannot touch four unrelated
anchors, so the same evidence reused across four requirements promotes only the
ones whose anchored files that command actually read, and leaves the rest at
`Candidate` with the missing half named.

Every level below `Exercised` carries a `MissingForPromotion` value
(`ladder.rs:131`) naming the specific absent fact:

- the task declared no runnable verifier command at all;
- it declared commands, but none appears in the run's evidence;
- a declared command ran and did not pass;
- a declared command passed, but the trace never shows the anchored file being
  read by that run;
- there was no trace at all.

### The honest limit

`FileRead` evidence is file-granular. "This run read the file containing the
anchor" is weaker than "this run executed the anchored lines", so a broad test
that reads a file without exercising the anchored function still promotes to
`Exercised`. Line-granular proof needs coverage instrumentation, which is the
upgrade path rather than the first cut.

## Why an empty ladder is the correct result

**Running the command with no `--leann-db` reports every requirement as
`Unproven`, and that is right, not a bug.**

Anchoring requires an already-built code index. Without one there is no index to
consult, so no anchor can be found, so nothing can climb. The command says so in
its own header (`src/command/requirement_trace/render.rs:38`):

```
Code index: NOT CONSULTED (--leann-db not given). Every requirement is
Unproven for want of an anchor, not for want of code.
```

Internally the row is marked `AnchorGap::IndexNotConsulted` rather than "no
anchors found" (`src/command/requirement_trace.rs:258`) — "we did not look" is
reported as such, because understating the code without evidence is the same
error as overstating it.

Two other paths also produce a floor result, and both are correct:

- **A requirement no task claims** is `AnchorGap::Unclaimed`. There is nothing
  to anchor against.
- **A stale anchor** — its file changed since anchoring, so the recorded line
  range no longer names the anchored code — collapses to `Unproven`.
  Known-stale beats silently-wrong, and neither counts.

So a first run on a fresh checkout should be expected to report zeros. To get
anything above `Candidate` you need three things at once: a built index, a run's
verifier evidence, and that run's ambient trace.

## Options

| Flag | Meaning |
|---|---|
| `--prd <PATH>` | **Required.** PRD markdown to extract `REQ-<AREA>-<NNN>` IDs from. |
| `--tasks <DIR>` | **Required.** Directory of decomposed-PRD `TASK-*.md` files. |
| `--leann-db <PATH>` | An existing LEANN code index to anchor against. Never created here. |
| `--graph <ID>` | A recorded graph id under `.archon/topology`, supplying `FileRead` evidence. |
| `--evidence <PATH>` | A run's final report JSON, supplying `commands_run` evidence. |
| `--persist <PATH>` | Write requirement entities and anchored edges into this knowledge store. |
| `--falsify` | Execute the falsification plans. **Mutates files in your working tree.** |
| `--json` | Emit the report model as JSON. |
| `--limit-per-scope <N>` | Index hits requested per declared path scope. Default `3`. |
| `--max-scopes <N>` | Declared path scopes searched per task, capping the query budget. Default `8`. |

The slash surface accepts every flag above except `--limit-per-scope` and
`--max-scopes` (`src/command/requirement_trace/slash.rs:86`). An unrecognised
token is an error naming the accepted flags rather than being ignored.

### `--leann-db` never indexes

The adapter opens an existing index and constructs the search directly, and
deliberately does not go through the path that would call `ensure_schema()` —
that is a write. If the index has never been built, the query fails and the
failure is reported as a named gap telling you to index out of band.

This is enforced at the point of construction, not by convention
(`src/command/requirement_trace/leann_source.rs:1`). The reason is contention:
LEANN's file replacement holds the Cozo write lock across an entire
`multi_transaction`, the longest critical section in the repository. A report
that indexed would serialise every other writer in the process for its duration.

Missing index file, verbatim:

```
no code index at <path>; build it out of band before tracing —
indexing holds the Cozo write lock across a whole multi_transaction
and must never run inside a report
```

## `--falsify`

`Exercised` proves the anchored file was in the causal path of a passing
verifier. It does not prove the verifier *depends* on the anchored lines — a
test that reads a module and asserts nothing about the anchored function still
promotes. `--falsify` closes that gap by experiment: replace the anchored lines
with an abort, run the verifier the task declared, and restore. The edge
promotes to `Falsifiable` only if the verifier failed while mutated. If it still
passed, the edge was decoration and the report says so.

**Without the flag, nothing runs.** No file is read for mutation, no `git`
executes, no process is spawned, and both the text and the JSON are
byte-identical to what they were before the flag existed
(`src/command/requirement_trace/falsify.rs:1`). An opt-in that changed the
read-only output would not be an opt-in.

Scope is limited to error-severity requirements whose edge already reached
`Exercised`. An unexecuted plan promotes nothing — a requirement with a plan and
no result stays at `Exercised`, which is the fail-closed direction.

### What it refuses to do, before writing anything

Every check that can refuse runs before a byte is written, in this order:
language, command shape, stranded backup, file hash, renderability, working-tree
cleanliness, and finally the baseline run
(`crates/archon-knowledge/src/traceability/falsification/outcome.rs:148`):

| Refusal | Why |
|---|---|
| `DirtyFile` | The file has uncommitted changes. A mutation mistaken for an edit is data loss. |
| `CleanlinessUnknown` | No repository, no `git`, or an error from it. Unknown is treated as dirty. |
| `WorkspaceWideCommand` | NFR-004 forbids workspace-wide runs; one has exhausted a disk twice. |
| `NotDirectlyExecutable` | The command needs a shell (pipe, redirect, `&&`). It is run as an argv, never through a shell. |
| `NoMutationForLanguage` | No known abort form for the file's language. |
| `FileChangedSincePlan` | The file no longer hashes to what the plan was written against. |
| `UnreconciledBackup` | A previous run left a backup and did not reconcile it. |
| `BaselineDidNotPass` | The verifier did not pass on the *original* bytes, so a failure while mutated proves nothing. |
| `UnusableRange` / `FileUnreadable` | The anchored range could not be replaced, or the file could not be read. |

The pass criterion has two halves: the verifier must fail while mutated **and**
pass again once restored. The second half is settled by reading the file back
and requiring byte equality with the original, so the experiment costs two runs
rather than three.

## Reading the report

The header states the PRD and task directory, whether the index was consulted, a
count per level, how many anchors are stale, and the gate verdict. The verdict
is deliberately blunt (`crates/archon-knowledge/src/traceability/report.rs:196`):

```
N/M requirements satisfied on evidence (Exercised or above). K are declared
residual gaps: each names what is missing, none counts as satisfied, and
none satisfies a promotion gate.
```

Nothing below `Exercised` is printed as satisfied — not as a tick, not as a
percentage that rounds up, not as a "mapped" column.

Coverage is reported separately from the ladder, from the tasks' explicit
`implements:` lists: how many requirements the PRD defines, how many distinct
IDs the tasks cite, and any phantom citations (an ID cited by a task but absent
from the PRD).

The report also groups anchors by citation to surface evidence reused across
several requirements, which is the shape a padded traceability matrix takes.

## Related

- [`archon workflow lint`](workflow-lint.md) — advisory topology analyses,
  including requirement coverage, over the same task set.
- [Topology](../architecture/topology.md) — the ambient trace that supplies the
  `FileRead` records this command consumes.
- [PRD-driven development](../cookbook/prd-driven-development.md) — where the
  PRD and task set come from.
