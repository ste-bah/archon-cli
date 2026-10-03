//! The resume a run carries, scoped to the run's own task (#241).
//!
//! A resume hands its run the history and the occupancy it resumes. Kept in
//! a slot shared by id, it could be taken by another run of the same id, or
//! dropped by a caller that stopped waiting while the run still queued for
//! capacity. Scoped to the run's task, it goes wherever the run goes and
//! nothing else can see it. The payload is the host's own type; this crate
//! only carries it.
use std::any::Any;
use std::sync::Arc;

#[derive(Clone)]
pub struct ResumeScope {
    pub agent_id: String,
    pub payload: Arc<dyn Any + Send + Sync>,
}

tokio::task_local! { static RESUME: ResumeScope; }

/// The resume of `agent_id` this task runs, if it runs one.
pub fn current_for(agent_id: &str) -> Option<ResumeScope> {
    RESUME
        .try_with(Clone::clone)
        .ok()
        .filter(|scope| scope.agent_id == agent_id)
}

pub async fn scope<T>(resume: ResumeScope, work: impl std::future::Future<Output = T>) -> T {
    RESUME.scope(resume, work).await
}

pub async fn inherit<T>(
    resume: Option<ResumeScope>,
    work: impl std::future::Future<Output = T>,
) -> T {
    match resume {
        Some(resume) => scope(resume, work).await,
        None => work.await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_resume_is_seen_only_by_its_own_agent_id() {
        let resume = ResumeScope {
            agent_id: "a".into(),
            payload: Arc::new(7u32),
        };
        scope(resume, async {
            assert!(current_for("a").is_some());
            assert!(current_for("b").is_none());
        })
        .await;
        assert!(current_for("a").is_none());
    }
}
