Identity-only compatibility fixture for issue 358, read from the live run's
launch records on 2026-10-06. Binary revision `8b7a3c13f`, state schema 1.

`fixed-decomposition-launch-v1.json` preserves the exact template, revision,
script/catalog digests, phase, attempt and disposition shape. Filesystem
identities and the log path are replaced with generic fixture paths.
`fixed-command-catalog-v1.json` is the launch's generic host-command catalog:
only capabilities and placeholder argv, without project inputs or artifacts.

The live run was only read. Tests use temporary synthetic projects and never
invoke Archon against the live run. The source digest was independently
verified against its recorded workflow.js and its identity against metadata.
