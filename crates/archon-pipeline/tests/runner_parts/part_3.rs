
// ---------------------------------------------------------------------------
// Compile-check: CodingFacade and ResearchFacade stubs
// ---------------------------------------------------------------------------

/// This test exists purely to verify that the PipelineFacade trait is
/// implementable by domain-specific facades (CodingFacade, ResearchFacade).
/// It does not run the pipeline — it only confirms the types compile.
#[tokio::test]
async fn test_coding_and_research_facades_compile() {
    // CodingFacade stub
    struct CodingFacade;

    #[async_trait::async_trait]
    impl PipelineFacade for CodingFacade {
        async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
            Ok(PipelineSession {
                id: "coding-session".to_string(),
                pipeline_type: PipelineType::Coding,
                task: task.to_string(),
                started_at: Instant::now(),
                agent_results: vec![],
                leann_context: String::new(),
            })
        }

        async fn next_agent(&self, _session: &PipelineSession) -> anyhow::Result<NextAgent> {
            Ok(NextAgent::Done)
        }

        async fn build_prompt(
            &self,
            _session: &PipelineSession,
            _agent: &AgentInfo,
        ) -> anyhow::Result<(
            Vec<serde_json::Value>,
            Vec<serde_json::Value>,
            Vec<serde_json::Value>,
        )> {
            Ok((vec![], vec![], vec![]))
        }

        async fn score_quality(
            &self,
            _session: &PipelineSession,
            _agent: &AgentInfo,
            _result: &AgentResult,
        ) -> anyhow::Result<QualityScore> {
            Ok(QualityScore {
                overall: 0.9,
                dimensions: HashMap::new(),
            })
        }

        async fn process_completion(
            &self,
            _session: &mut PipelineSession,
            _agent: &AgentInfo,
            _result: &AgentResult,
            _quality: &QualityScore,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn finalize(&self, session: PipelineSession) -> anyhow::Result<PipelineResult> {
            Ok(PipelineResult {
                session_id: session.id,
                pipeline_type: PipelineType::Coding,
                agent_results: vec![],
                total_cost_usd: 0.0,
                duration: session.started_at.elapsed(),
                final_output: String::new(),
            })
        }
    }

    // ResearchFacade stub
    struct ResearchFacade;

    #[async_trait::async_trait]
    impl PipelineFacade for ResearchFacade {
        async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
            Ok(PipelineSession {
                id: "research-session".to_string(),
                pipeline_type: PipelineType::Research,
                task: task.to_string(),
                started_at: Instant::now(),
                agent_results: vec![],
                leann_context: String::new(),
            })
        }

        async fn next_agent(&self, _session: &PipelineSession) -> anyhow::Result<NextAgent> {
            Ok(NextAgent::Done)
        }

        async fn build_prompt(
            &self,
            _session: &PipelineSession,
            _agent: &AgentInfo,
        ) -> anyhow::Result<(
            Vec<serde_json::Value>,
            Vec<serde_json::Value>,
            Vec<serde_json::Value>,
        )> {
            Ok((vec![], vec![], vec![]))
        }

        async fn score_quality(
            &self,
            _session: &PipelineSession,
            _agent: &AgentInfo,
            _result: &AgentResult,
        ) -> anyhow::Result<QualityScore> {
            Ok(QualityScore {
                overall: 0.8,
                dimensions: HashMap::new(),
            })
        }

        async fn process_completion(
            &self,
            _session: &mut PipelineSession,
            _agent: &AgentInfo,
            _result: &AgentResult,
            _quality: &QualityScore,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn finalize(&self, session: PipelineSession) -> anyhow::Result<PipelineResult> {
            Ok(PipelineResult {
                session_id: session.id,
                pipeline_type: PipelineType::Research,
                agent_results: vec![],
                total_cost_usd: 0.0,
                duration: session.started_at.elapsed(),
                final_output: String::new(),
            })
        }
    }

    // Verify both facades can be used where PipelineFacade is expected.
    let _coding: Box<dyn PipelineFacade> = Box::new(CodingFacade);
    let _research: Box<dyn PipelineFacade> = Box::new(ResearchFacade);
}
