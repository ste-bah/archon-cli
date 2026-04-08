//! Algorithm prompt snippets and phase-default mapping.
//!
//! Each [`Algorithm`] variant carries a natural-language prompt snippet that
//! instructs the LLM how to reason during a pipeline agent invocation.

use std::fmt;
use std::str::FromStr;

use super::agents::{Algorithm, CodingAgent, Phase};

// ---------------------------------------------------------------------------
// Prompt snippets
// ---------------------------------------------------------------------------

impl Algorithm {
    /// Returns a natural-language prompt snippet (~200-500 tokens) that
    /// describes the reasoning protocol the LLM should follow when this
    /// algorithm is active.
    pub fn prompt_snippet(&self) -> &'static str {
        match self {
            Algorithm::LATS => LATS_SNIPPET,
            Algorithm::ReAct => REACT_SNIPPET,
            Algorithm::ToT => TOT_SNIPPET,
            Algorithm::SelfDebug => SELF_DEBUG_SNIPPET,
            Algorithm::Reflexion => REFLEXION_SNIPPET,
            Algorithm::PoT => POT_SNIPPET,
        }
    }
}

const LATS_SNIPPET: &str = "\
**Algorithm: LATS (Language Agent Tree Search)**
You are solving a complex algorithmic task using tree search over candidate solutions.

Follow this reasoning protocol:

1. **Decompose** the problem into sub-goals. Identify the key constraints and \
edge cases before writing any code.
2. **Branch** — generate at least two distinct solution approaches (e.g., \
brute-force vs. optimized, iterative vs. recursive). Briefly describe each.
3. **Evaluate** each branch: estimate time complexity, space complexity, and \
correctness risk. Prune any branch that clearly violates a constraint.
4. **Expand** the most promising branch into a full implementation. If you hit \
a dead end (logic error, impossible constraint), backtrack to the next branch.
5. **Verify** the final solution against all stated requirements and edge cases \
before presenting it.

Output format: present the chosen solution with inline comments explaining \
each major step. After the code, list the branches you considered and why \
they were accepted or pruned.

Self-check: re-read the original requirements and confirm every constraint \
is satisfied. If any constraint is unmet, backtrack and try another branch.";

const REACT_SNIPPET: &str = "\
**Algorithm: ReAct (Reasoning + Acting)**
You are completing a task that requires interleaving reasoning with tool use.

Follow this reasoning protocol:

1. **Thought** — describe what you need to find out or accomplish next and why.
2. **Action** — invoke exactly one tool (read a file, run a command, search, \
etc.) to gather information or make a change.
3. **Observation** — summarize the tool's output and what it tells you.
4. **Repeat** steps 1-3 until you have enough information to produce the final \
answer or artifact.
5. **Conclude** — synthesize your observations into a final, complete response.

Output format: label each step clearly as Thought / Action / Observation. \
End with a Conclusion section containing the deliverable.

Self-check: before concluding, verify that every sub-question raised in \
your Thought steps has been answered by a corresponding Observation. If \
any gap remains, add another Thought-Action-Observation cycle.";

const TOT_SNIPPET: &str = "\
**Algorithm: ToT (Tree of Thought)**
You are making a design decision that benefits from exploring multiple options.

Follow this reasoning protocol:

1. **Frame** the decision: state the goal, the constraints, and the evaluation \
criteria (e.g., simplicity, performance, extensibility).
2. **Generate** at least three distinct thought branches — each a different \
approach or design choice. Describe each in 2-3 sentences.
3. **Evaluate** each branch against the criteria. Score or rank them and \
explain your reasoning.
4. **Select** the best branch. If two branches tie, identify the tie-breaker \
criterion and justify your choice.
5. **Detail** the selected design: expand it into a concrete specification, \
API surface, or implementation plan.

Output format: present the branches in a numbered list, then the evaluation \
table, then the selected design in full detail.

Self-check: confirm the selected design satisfies every stated constraint. \
If it compromises on any criterion, state the trade-off explicitly.";

const SELF_DEBUG_SNIPPET: &str = "\
**Algorithm: Self-Debug**
You are writing code in a test-driven, iterative debugging loop.

Follow this reasoning protocol:

1. **Write** the initial implementation based on the specification or failing \
test case.
2. **Run** the tests (or mentally trace the code against the expected behavior) \
and observe which tests pass and which fail.
3. **Diagnose** each failure: identify the root cause by examining the error \
message, stack trace, and relevant code path.
4. **Fix** one issue at a time, re-running tests after each fix to confirm \
progress and avoid regressions.
5. **Finalize** once all tests pass. Review the code for clarity and remove \
any debugging artifacts.

Output format: present the final implementation followed by a brief changelog \
listing each fix you applied and the test it resolved.

Self-check: confirm that every test case mentioned in the spec passes. If \
any test is still failing, loop back to step 3.";

const REFLEXION_SNIPPET: &str = "\
**Algorithm: Reflexion**
You are learning from previous attempts and applying corrections.

Follow this reasoning protocol:

1. **Review** the previous attempt (or the current state of the code/artifact). \
Summarize what was tried and what went wrong.
2. **Reflect** — identify the root pattern behind the failure. Was it a \
misunderstanding of the requirements, a wrong assumption, or a technical \
mistake? State the lesson learned.
3. **Plan** a corrective approach that directly addresses the identified \
pattern. Explain how this attempt differs from the previous one.
4. **Implement** the corrected solution, being explicit about where and \
why you deviated from the earlier approach.
5. **Validate** the new solution against the same criteria that revealed \
the earlier failure. Confirm the fix and check for side effects.

Output format: begin with a Reflection section summarizing the previous \
failure and the lesson learned, then present the corrected implementation.

Self-check: compare the new output to the original failure criteria. If \
the same failure pattern reappears, add another reflection cycle.";

const POT_SNIPPET: &str = "\
**Algorithm: PoT (Program of Thought)**
You are solving an analytical or mathematical task by structuring your \
reasoning as an executable program.

Follow this reasoning protocol:

1. **Define** the inputs, outputs, and constraints as explicit variables \
or data structures.
2. **Decompose** the computation into a sequence of clearly named steps, \
each computing an intermediate result.
3. **Implement** each step as a function or expression. Use descriptive \
variable names so the logic is self-documenting.
4. **Execute** the program mentally (or via a tool) to compute the final \
result. Show intermediate values at each step.
5. **Verify** the result by checking boundary conditions, dimensional \
analysis, or an independent calculation method.

Output format: present the structured program with comments annotating \
each step, followed by the computed result and the verification check.

Self-check: plug extreme or boundary inputs into the program and confirm \
the outputs are reasonable. If any step produces an unexpected value, \
trace backward to find the error.";

// ---------------------------------------------------------------------------
// Phase defaults
// ---------------------------------------------------------------------------

/// Returns the default algorithm for a given pipeline phase.
pub fn phase_default(phase: Phase) -> Algorithm {
    match phase {
        Phase::Understanding => Algorithm::ReAct,
        Phase::Design => Algorithm::ToT,
        Phase::WiringPlan => Algorithm::ToT,
        Phase::Implementation => Algorithm::SelfDebug,
        Phase::Testing => Algorithm::SelfDebug,
        Phase::Refinement => Algorithm::Reflexion,
    }
}

// ---------------------------------------------------------------------------
// Algorithm selection
// ---------------------------------------------------------------------------

/// Returns the algorithm to use for a given agent. Currently this is simply
/// the agent's primary `algorithm` field; future versions may incorporate
/// task-level overrides or fallback logic.
pub fn select_algorithm(agent: &CodingAgent) -> Algorithm {
    agent.algorithm
}

// ---------------------------------------------------------------------------
// FromStr
// ---------------------------------------------------------------------------

/// Error returned when parsing an invalid algorithm name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseAlgorithmError(String);

impl fmt::Display for ParseAlgorithmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown algorithm: {:?}", self.0)
    }
}

impl std::error::Error for ParseAlgorithmError {}

impl FromStr for Algorithm {
    type Err = ParseAlgorithmError;

    /// Parses an algorithm name from a string.
    ///
    /// Accepted inputs (case-sensitive):
    /// - `"LATS"` -> [`Algorithm::LATS`]
    /// - `"ReAct"` -> [`Algorithm::ReAct`]
    /// - `"ToT"` -> [`Algorithm::ToT`]
    /// - `"Self-Debug"` or `"SelfDebug"` -> [`Algorithm::SelfDebug`]
    /// - `"Reflexion"` -> [`Algorithm::Reflexion`]
    /// - `"PoT"` -> [`Algorithm::PoT`]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "LATS" => Ok(Algorithm::LATS),
            "ReAct" => Ok(Algorithm::ReAct),
            "ToT" => Ok(Algorithm::ToT),
            "Self-Debug" | "SelfDebug" => Ok(Algorithm::SelfDebug),
            "Reflexion" => Ok(Algorithm::Reflexion),
            "PoT" => Ok(Algorithm::PoT),
            other => Err(ParseAlgorithmError(other.to_owned())),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_snippets_non_empty() {
        let variants = [
            Algorithm::LATS,
            Algorithm::ReAct,
            Algorithm::ToT,
            Algorithm::SelfDebug,
            Algorithm::Reflexion,
            Algorithm::PoT,
        ];
        for alg in &variants {
            let snippet = alg.prompt_snippet();
            assert!(!snippet.is_empty(), "{:?} returned an empty snippet", alg);
        }
    }

    #[test]
    fn snippets_contain_numbered_steps() {
        let variants = [
            Algorithm::LATS,
            Algorithm::ReAct,
            Algorithm::ToT,
            Algorithm::SelfDebug,
            Algorithm::Reflexion,
            Algorithm::PoT,
        ];
        for alg in &variants {
            let snippet = alg.prompt_snippet();
            // Every snippet should contain at least steps 1-3.
            for n in 1..=3 {
                let marker = format!("{}.", n);
                assert!(
                    snippet.contains(&marker),
                    "{:?} snippet missing step {}",
                    alg,
                    n
                );
            }
        }
    }

    #[test]
    fn snippets_under_2000_chars() {
        let variants = [
            Algorithm::LATS,
            Algorithm::ReAct,
            Algorithm::ToT,
            Algorithm::SelfDebug,
            Algorithm::Reflexion,
            Algorithm::PoT,
        ];
        for alg in &variants {
            let snippet = alg.prompt_snippet();
            assert!(
                snippet.len() <= 2000,
                "{:?} snippet is {} chars (max 2000)",
                alg,
                snippet.len()
            );
        }
    }

    #[test]
    fn phase_default_mapping() {
        assert_eq!(phase_default(Phase::Understanding), Algorithm::ReAct);
        assert_eq!(phase_default(Phase::Design), Algorithm::ToT);
        assert_eq!(phase_default(Phase::WiringPlan), Algorithm::ToT);
        assert_eq!(phase_default(Phase::Implementation), Algorithm::SelfDebug);
        assert_eq!(phase_default(Phase::Testing), Algorithm::SelfDebug);
        assert_eq!(phase_default(Phase::Refinement), Algorithm::Reflexion);
    }

    #[test]
    fn select_algorithm_uses_primary() {
        use super::super::agents::AGENTS;

        // Pick any agent and verify select_algorithm returns its algorithm.
        let agent = &AGENTS[0];
        assert_eq!(select_algorithm(agent), agent.algorithm);
    }

    #[test]
    fn from_str_valid() {
        assert_eq!("LATS".parse::<Algorithm>(), Ok(Algorithm::LATS));
        assert_eq!("ReAct".parse::<Algorithm>(), Ok(Algorithm::ReAct));
        assert_eq!("ToT".parse::<Algorithm>(), Ok(Algorithm::ToT));
        assert_eq!("Self-Debug".parse::<Algorithm>(), Ok(Algorithm::SelfDebug));
        assert_eq!("SelfDebug".parse::<Algorithm>(), Ok(Algorithm::SelfDebug));
        assert_eq!("Reflexion".parse::<Algorithm>(), Ok(Algorithm::Reflexion));
        assert_eq!("PoT".parse::<Algorithm>(), Ok(Algorithm::PoT));
    }

    #[test]
    fn from_str_invalid() {
        assert!("lats".parse::<Algorithm>().is_err());
        assert!("".parse::<Algorithm>().is_err());
        assert!("unknown".parse::<Algorithm>().is_err());
        assert!("self-debug".parse::<Algorithm>().is_err());
    }

    #[test]
    fn algorithm_is_clone_copy_debug_eq() {
        let a = Algorithm::LATS;
        let b = a; // Copy
        let _c = a.clone(); // Clone
        assert_eq!(a, b); // PartialEq + Eq
        let _ = format!("{:?}", a); // Debug
    }
}
