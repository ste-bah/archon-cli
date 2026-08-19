//! FinalStageOrchestrator (Phase 8) — assembles agent outputs into a final research paper.
//!
//! This is phase 8, "Final Assembly", of the 47-agent phdresearch pipeline
//! (`.archon/agents/phdresearch/pipeline.toml`) — the phase whose single
//! agent is `chapter-synthesizer`. It provides a state-machine-driven
//! pipeline that:
//!
//! 1. Scans agent output files from the research directory.
//! 2. Maps outputs to chapters using heuristic keyword matching.
//! 3. Assembles each chapter's body via [`writer::synthesize_chapter`] — the
//!    `chapter-synthesizer` seam, concatenation until `LlmClient` reaches
//!    this stage (REQ-RESEARCH-007).
//! 4. Combines chapters into a final paper with Table of Contents.
//! 5. Applies a style profile via [`style_applier::apply_style`], or reports
//!    in [`FinalStageResult::warnings`] that it could not.
//!
//! Steps 3 and 5 were described here while `run` called neither module: the
//! concatenation was inlined in step 5's loop, and `style_applier` was not
//! called at all, so [`FinalStageOptions::style_profile_id`] was read by
//! nothing and a requested profile was dropped in silence. This is a library
//! crate, so `pub` kept `dead_code` quiet about both seams.

pub mod combiner;
pub mod mapper;
pub mod scanner;
pub mod style_applier;
pub mod writer;

use std::path::Path;

// -------------------------------------------------------------------------
// State machine
// -------------------------------------------------------------------------

/// States through which the orchestrator progresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalStageState {
    Idle,
    Initializing,
    Scanning,
    Summarizing,
    Mapping,
    Writing,
    Combining,
    Validating,
    Completed,
    Failed,
}

// -------------------------------------------------------------------------
// Configuration / result types
// -------------------------------------------------------------------------

/// Options that control orchestrator behaviour.
#[derive(Debug, Clone)]
pub struct FinalStageOptions {
    /// When `true`, overwrite any existing output directory.
    pub force: bool,
    /// When `true`, run the mapping phase but skip writing to disk.
    pub dry_run: bool,
    /// Optional style profile identifier to apply to the final paper.
    pub style_profile_id: Option<String>,
    /// Optional token budget for LLM synthesis (currently unused).
    pub token_budget: Option<usize>,
}

/// The outcome of a successful orchestration run.
#[derive(Debug, Clone)]
pub struct FinalStageResult {
    /// Path to the generated final paper on disk (empty in dry-run mode).
    pub final_paper_path: String,
    /// Number of chapters in the paper.
    pub chapter_count: usize,
    /// Total word count of the final paper.
    pub word_count: usize,
    /// Non-fatal warnings collected during the run.
    pub warnings: Vec<String>,
}

// -------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------

/// Errors that can occur during final-stage orchestration.
#[derive(Debug)]
pub enum FinalStageError {
    /// The research directory does not exist.
    NoResearchDir,
    /// Output directory already exists and `force` is not set.
    OutputExists,
    /// Scanning agent outputs failed.
    ScanFailed(String),
    /// Mapping outputs to chapters failed.
    MappingFailed(String),
    /// No source material was found to assemble.
    NoSources,
    /// Style profile application failed.
    StyleError(String),
    /// Writing output to disk failed.
    WriteError(String),
    /// The assembled paper exceeds the token budget.
    TokenOverflow,
    /// Final validation of the paper failed.
    ValidationFailed(String),
    /// An illegal state transition was attempted.
    InvalidTransition {
        from: FinalStageState,
        to: FinalStageState,
    },
}

// -------------------------------------------------------------------------
// Orchestrator
// -------------------------------------------------------------------------

/// Drives the final-stage assembly pipeline through a linear state machine.
pub struct FinalStageOrchestrator {
    state: FinalStageState,
}

impl FinalStageOrchestrator {
    #[allow(clippy::new_without_default)]
    /// Create a new orchestrator in the `Idle` state.
    pub fn new() -> Self {
        Self {
            state: FinalStageState::Idle,
        }
    }

    /// Return the current state.
    pub fn state(&self) -> FinalStageState {
        self.state
    }

    /// Validate that a state transition is allowed.
    ///
    /// Valid transitions follow the linear chain from `Idle` through to
    /// `Completed`. Any state may also transition to `Failed`.
    pub fn validate_transition(
        from: FinalStageState,
        to: FinalStageState,
    ) -> Result<(), FinalStageError> {
        let valid = matches!(
            (from, to),
            (FinalStageState::Idle, FinalStageState::Initializing)
                | (FinalStageState::Initializing, FinalStageState::Scanning)
                | (FinalStageState::Scanning, FinalStageState::Summarizing)
                | (FinalStageState::Summarizing, FinalStageState::Mapping)
                | (FinalStageState::Mapping, FinalStageState::Writing)
                | (FinalStageState::Writing, FinalStageState::Combining)
                | (FinalStageState::Combining, FinalStageState::Validating)
                | (FinalStageState::Validating, FinalStageState::Completed)
        );
        if valid || to == FinalStageState::Failed {
            Ok(())
        } else {
            Err(FinalStageError::InvalidTransition { from, to })
        }
    }

    /// Generate a placeholder chapter when writing fails (EC-PIPE-007).
    pub fn generate_chapter_placeholder(title: &str, error: &str) -> String {
        format!(
            "## {}\n\n*This chapter could not be generated. Error: {}*\n",
            title, error
        )
    }

    /// Return max retries for a given error type (recovery strategy).
    pub fn max_retries_for(error: &FinalStageError) -> u32 {
        match error {
            FinalStageError::ScanFailed(_) => 2,
            FinalStageError::WriteError(_) => 1,
            FinalStageError::NoSources => 1,
            FinalStageError::StyleError(_) => 0,
            FinalStageError::ValidationFailed(_) => 0,
            FinalStageError::TokenOverflow => 0,
            _ => 0,
        }
    }

    /// Run the orchestration pipeline.
    ///
    /// Scans `source_dir` for agent output files, maps them to the given
    /// `chapters`, and assembles a final paper under `source_dir/final/`.
    pub async fn run(
        &self,
        source_dir: &Path,
        chapters: &[(u32, String)],
        options: &FinalStageOptions,
    ) -> Result<FinalStageResult, FinalStageError> {
        // 1. Scan outputs
        let outputs = scanner::scan_outputs(source_dir)
            .map_err(|e| FinalStageError::ScanFailed(e.to_string()))?;

        // 2. Map to chapters
        let mapping = mapper::map_to_chapters(outputs, chapters);
        let chapter_count = chapters.len();

        // 3. Dry run — return early with mapping info
        if options.dry_run {
            return Ok(FinalStageResult {
                final_paper_path: String::new(),
                chapter_count,
                word_count: 0,
                warnings: Vec::new(),
            });
        }

        // 4. Check for existing output (unless force)
        let final_dir = source_dir.join("final");
        if final_dir.exists() && !options.force {
            return Err(FinalStageError::OutputExists);
        }

        // 5. Build chapter content from mapping, through the `writer` seam —
        // the phase-8 `chapter-synthesizer` agent's entry point. It performs
        // the concatenation that used to be inlined right here, which is what
        // left the seam with no caller at all.
        let mut chapter_contents: Vec<combiner::ChapterContent> = Vec::new();
        for (num, title) in chapters {
            let sources: Vec<&str> = mapping
                .mappings
                .get(num)
                .map(|mapped| mapped.iter().map(|m| m.content.as_str()).collect())
                .unwrap_or_default();

            let content = if sources.is_empty() {
                Self::generate_chapter_placeholder(title, "No source material mapped")
            } else {
                writer::synthesize_chapter(title, &sources)
            };

            chapter_contents.push(combiner::ChapterContent {
                number: *num,
                title: title.clone(),
                content,
            });
        }

        // 6. Combine
        let paper = combiner::combine_chapters(&chapter_contents);

        // 7. Style. No profiles are implemented yet, so naming one returns the
        // paper unchanged plus a warning saying so — which is the point of the
        // call: `style_profile_id` was previously read by nothing, so asking
        // for a profile was ignored in silence.
        let (paper, style_warning) =
            style_applier::apply_style(&paper, options.style_profile_id.as_deref());
        let warnings: Vec<String> = style_warning.into_iter().collect();
        let word_count = paper.split_whitespace().count();

        // 8. Write output
        std::fs::create_dir_all(&final_dir)
            .map_err(|e| FinalStageError::WriteError(e.to_string()))?;
        let paper_path = final_dir.join("final-paper.md");
        std::fs::write(&paper_path, &paper)
            .map_err(|e| FinalStageError::WriteError(e.to_string()))?;

        Ok(FinalStageResult {
            final_paper_path: paper_path.to_string_lossy().to_string(),
            chapter_count,
            word_count,
            warnings,
        })
    }
}
