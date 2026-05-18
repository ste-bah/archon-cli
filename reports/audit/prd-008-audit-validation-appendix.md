# PRD-008 Audit Validation Appendix

Generated for PRD-ARCHON-FINALISATION-008 remediation.

## Rerunnable Surface Check

```bash
tools/audit/validate-report-surface --expect-crates 28 --expect-docs 154
```

The validation script reports machine-readable JSON containing:

- git branch, commit, and dirty-worktree state
- crate count
- documentation count
- test annotation counts per crate
- caller/reference counts for cited symbols
- feature-vs-implementation spot checks

## Count Definitions

Crate count is the number of `crates/*/Cargo.toml` package manifests.

Documentation count is the number of Markdown files under `docs/`, `scripts/`,
and `extensions/`, plus top-level Markdown files. This is the count definition
that yields the corrected 154-document value for commit `a593fd6`.

Test annotation counts include Rust `#[test]`, `#[tokio::test]`, and
`#[async_std::test]` annotations split by crate `src/` and crate `tests/`
directories. Root integration tests are reported separately for the bash
surface spot check.

## Corrected Surface Facts

As of commit `a593fd6`:

- crates: 28
- docs: 154
- `archon-leann` tests: 70 total by current annotation scan
- `archon-plugin` tests: 69 total by current annotation scan
- direct root integration files invoking `Command::new("bash")`: 4 files, 12 test annotations in those files

These numbers supersede the stale May 17 audit counts of 27 crates and 143 docs.

## Status Vocabulary

Audit findings must use one of:

- Confirmed
- False
- Stale
- Partially confirmed
- Unproven
- Needs runtime/data validation

Reports must avoid subsystem-dead language when the evidence only proves that
one pathway is unwired. For example, the GNN auto-trainer has a live
meaning-triplet provider path; the verified gap is that production SONA
trajectory writes were not feeding that training source.
