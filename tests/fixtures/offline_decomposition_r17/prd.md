Document: Artifact Preparation
Revision: 17
Status: ready

# 1 Objective
Create two small artifacts in dependency order.

# 2 Requirements
- REQ-X-001: The first task creates `src/alpha.txt` with the nonempty line `alpha ready`.
- REQ-X-002: The second task depends on the first and creates `src/beta.json` as an object whose `ready` field is true.
- REQ-X-003: The second artifact remains valid JSON.

# 3 Artifact layout
## 3.1 First artifact
The first artifact is plain text.
## 3.2 Second artifact
The second artifact is a JSON object.

# 4 Constraints
- C-X-001: All work preserves the declared artifact formats.
- C-X-002: The second task preserves the first artifact unchanged.

| ID | Constraint |
|---|---|
| C-X-003 | All artifacts use repository-relative paths. |

# 5 Acceptance Criteria
| ID | Criterion |
|---|---|
| AC-X-001 | The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true. |

# 6 Completion
Both artifacts satisfy their focused checks.

# 7 Non-Goals
Remote publication is outside the scope.

# 8 Decomposition Guidance
Prefer one task for each artifact; this guidance adds no work.

# 9 Closing statement
This document concludes the artifact preparation contract and adds no work.
