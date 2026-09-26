//! The declared focused tests of a write call, tracked by the guard so the
//! host can tell the agent to submit once every one of them has passed.
use super::normalise_command;

/// What a write agent must see pass before the host tells it to submit.
#[derive(Debug, Clone, Default)]
pub struct FocusedTestPlan {
    /// Declared commands, verbatim; empty means the completion signal is inert.
    pub commands: Vec<String>,
    /// Tool calls admitted after the completion message before inspection and
    /// build/test calls are refused.
    pub submit_grace_calls: u32,
}

impl FocusedTestPlan {
    pub fn new(commands: Vec<String>, submit_grace_calls: u32) -> Self {
        Self {
            commands,
            submit_grace_calls,
        }
    }
}

/// Which declared focused tests this session has seen exit 0.
#[derive(Debug)]
pub(super) struct FocusedTests {
    pub(super) declared: Vec<String>,
    pub(super) passed: Vec<bool>,
    pub(super) submit_grace_calls: u32,
    /// The tool call at which the last declared test passed; set once.
    pub(super) complete_at_call: Option<u64>,
    /// Whether the completion message has been handed to the runner.
    pub(super) announced: bool,
    /// Tool calls since completion, against the grace allowance.
    pub(super) calls_after_complete: u64,
}

impl FocusedTests {
    pub(super) fn new(plan: FocusedTestPlan) -> Option<Self> {
        let declared: Vec<String> = plan
            .commands
            .iter()
            .map(|command| normalise_command(command))
            .filter(|command| !command.is_empty())
            .collect();
        if declared.is_empty() {
            return None;
        }
        Some(Self {
            passed: vec![false; declared.len()],
            declared,
            submit_grace_calls: plan.submit_grace_calls,
            complete_at_call: None,
            announced: false,
            calls_after_complete: 0,
        })
    }

    pub(super) fn submit_instruction(&self) -> String {
        format!(
            "All declared focused tests have passed in this session ({n} of {n} at tool call {k}). Return the result envelope now. Further verification is the verifier's job; pre-existing failures outside your target_files are to be reported in residual_gaps, not fixed.",
            n = self.declared.len(),
            k = self.complete_at_call.unwrap_or_default(),
        )
    }

    /// Issue-115: the budget refusal once every declared test has passed.
    /// "Write a deliverable file now" is the wrong advice to a session whose
    /// work is done, and a session told it keeps reading until the thrash
    /// cut; this leads with the instruction that ends the session cleanly.
    pub(super) fn submit_over_budget(&self, reads: u32, writes: u32) -> String {
        format!(
            "{} The read budget is exhausted as well ({reads} reads since your last substantive write; {writes} substantive write{} in this session): further reading is refused, and continuing to call tools instead of returning ends the session.",
            self.submit_instruction(),
            if writes == 1 { "" } else { "s" },
        )
    }
}
