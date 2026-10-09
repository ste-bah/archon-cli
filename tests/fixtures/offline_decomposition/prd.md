# Artifact Preparation

## Objective
Create two small artifacts in dependency order.

## Requirements
| ID | Requirement |
|---|---|
| REQ-001 | The first task creates `src/alpha.txt` with the nonempty line `alpha ready`. |
| REQ-002 | The second task depends on the first and creates `src/beta.json` as an object whose `ready` field is true. |

## Acceptance Criteria
| ID | Criterion |
|---|---|
| AC-001 | The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true. |
