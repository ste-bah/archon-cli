# tests/regression_guards/

Regression guard tests enforcing PRESERVE-D5 and PRESERVE-D8 policies.

PRESERVE-D5: behavioral invariants that must never regress across refactors.
PRESERVE-D8: API surface and wire-format invariants.
Any failure here indicates a preserved behavior was broken and must be
reverted or explicitly re-approved.
