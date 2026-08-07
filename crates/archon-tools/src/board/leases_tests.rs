use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use archon_memory::board::{BoardAccess, BoardItemKind, BoardStatus, NewBoardItem};
use archon_memory::graph::MemoryGraph;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{HolderLiveness, TOP_LEVEL_AGENT, holder_liveness, release_dead_claims};
use crate::background_agents::{
    AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, new_result_slot,
};
use crate::task_manager::{TASK_MANAGER, TaskStatus};

fn board() -> Arc<dyn BoardAccess> {
    Arc::new(MemoryGraph::in_memory().expect("in-memory graph"))
}

fn raise(board: &dyn BoardAccess, run_id: &str, title: &str) -> String {
    board
        .create_board_item(&NewBoardItem {
            id: None,
            run_id: run_id.to_string(),
            kind: BoardItemKind::Issue,
            title: title.to_string(),
            evidence: "crates/archon-tools/src/board/leases.rs:1 -- observed".to_string(),
            acceptance: "the claim survives or does not, per its holder".to_string(),
            raised_by: "raiser".to_string(),
        })
        .expect("create")
        .id
}

/// Register a live `AgentTool`-style agent in the global registry and return
/// the id it is keyed by.
fn register_background_agent(status: AgentStatus) -> Uuid {
    let agent_id = Uuid::new_v4();
    register_subagent(&agent_id.to_string(), status);
    agent_id
}

/// Register an agent the way `agent_tool::run::run_subagent_with_auto_background`
/// does — by runtime subagent id, whatever shape that id happens to be.
///
/// `crates/archon-tools/tests/subagent_liveness_choke_point.rs` is what proves
/// the runners really do this; here it is spelled out so the lease can be tested
/// without standing up an executor.
fn register_subagent(subagent_id: &str, status: AgentStatus) {
    BACKGROUND_AGENTS
        .register(BackgroundAgentHandle {
            agent_id: Uuid::parse_str(subagent_id).unwrap_or_else(|_| Uuid::new_v4()),
            subagent_id: subagent_id.to_string(),
            join_handle: None,
            cancel_token: CancellationToken::new(),
            spawned_at: SystemTime::now(),
            status: Arc::new(Mutex::new(status)),
            result_slot: new_result_slot(),
        })
        .expect("a test id cannot collide");
}

/// Spawn an `archon-pipeline`-style agent: its subagent id is
/// `{session}-{ordinal}-{agent}`, not a UUID, and no task ever describes it.
fn spawn_pipeline_agent(name: &str, status: AgentStatus) -> String {
    let subagent_id = format!("run-{name}-3-implementer");
    register_subagent(&subagent_id, status);
    subagent_id
}

/// Dispatch a `TaskCreate`-style agent: a task in `TASK_MANAGER` carrying the
/// subagent id, and nothing at all in `BACKGROUND_AGENTS`. This is now only
/// half of what dispatching does — the runner registers too — so it is used to
/// show what `TASK_MANAGER` alone is *not* enough to establish.
fn dispatch_task_agent(status: TaskStatus) -> String {
    let task_id = TASK_MANAGER.create_task("board lease test");
    let agent_id = Uuid::new_v4().to_string();
    TASK_MANAGER.set_agent_id(&task_id, &agent_id);
    TASK_MANAGER.set_status(&task_id, status);
    agent_id
}

// ── liveness ───────────────────────────────────────────────────

#[test]
fn a_running_background_agent_is_live() {
    let agent = register_background_agent(AgentStatus::Running);
    assert_eq!(holder_liveness(&agent.to_string()), HolderLiveness::Live);
}

#[test]
fn a_terminal_background_agent_is_dead() {
    let agent = register_background_agent(AgentStatus::Finished);
    assert_eq!(holder_liveness(&agent.to_string()), HolderLiveness::Dead);
}

/// Reaping is eager and leaves no tombstone, so an unknown id is either an
/// agent that finished and was swept up or one that never existed. Both mean
/// nothing is executing under it.
#[test]
fn an_id_in_neither_registry_is_dead() {
    assert_eq!(
        holder_liveness(&Uuid::new_v4().to_string()),
        HolderLiveness::Dead
    );
    assert_eq!(holder_liveness("not-a-uuid-at-all"), HolderLiveness::Dead);
}

/// The top-level agent is in neither registry and is alive as long as the
/// process is. Reading its absence as death would have the sweep release its
/// own claims.
#[test]
fn the_top_level_agent_is_never_swept() {
    assert_eq!(holder_liveness(TOP_LEVEL_AGENT), HolderLiveness::Live);
}

/// A `TaskCreate` agent is live because its runner registered it, exactly like
/// every other spawn path. The `TASK_MANAGER` task that dispatched it is still
/// there and still useful; it is simply not what the answer comes from.
#[test]
fn a_running_task_create_agent_is_live() {
    let agent = dispatch_task_agent(TaskStatus::Running);
    register_subagent(&agent, AgentStatus::Running);
    assert_eq!(holder_liveness(&agent), HolderLiveness::Live);
}

#[test]
fn a_completed_task_create_agent_is_dead() {
    let agent = dispatch_task_agent(TaskStatus::Running);
    register_subagent(&agent, AgentStatus::Running);
    BACKGROUND_AGENTS.mark_terminal(&agent, AgentStatus::Finished);
    assert_eq!(holder_liveness(&agent), HolderLiveness::Dead);
}

/// The fan-out is gone, and this is the assertion that keeps it gone: a task
/// that says `Running` and an agent that is in no registry disagree, and the
/// registry wins. `TASK_MANAGER` answers "what is this task doing", which is a
/// different question from "is this agent executing".
#[test]
fn holder_liveness_does_not_consult_the_task_manager() {
    let agent = dispatch_task_agent(TaskStatus::Running);
    assert_eq!(
        TASK_MANAGER.agent_is_running(&agent),
        Some(true),
        "the premise of this test is a TASK_MANAGER task that claims to be running"
    );
    assert_eq!(
        crate::background_agents::poll_subagent(&agent),
        crate::background_agents::PollOutcome::Unknown,
        "and an agent no runner ever registered"
    );

    assert_eq!(
        holder_liveness(&agent),
        HolderLiveness::Dead,
        "TASK_MANAGER must not be able to keep an unregistered agent alive"
    );
}

/// A pipeline agent is registered by the same choke point as everything else,
/// and its non-UUID id is no obstacle.
#[test]
fn a_running_pipeline_agent_is_live_and_a_finished_one_is_dead() {
    let agent = spawn_pipeline_agent("liveness", AgentStatus::Running);
    assert_eq!(holder_liveness(&agent), HolderLiveness::Live);

    BACKGROUND_AGENTS.mark_terminal(&agent, AgentStatus::Finished);
    assert_eq!(holder_liveness(&agent), HolderLiveness::Dead);
}

// ── the sweep ──────────────────────────────────────────────────

/// Both halves in one test on purpose: a sweep that released everything would
/// pass the first assertion on its own.
#[test]
fn the_sweep_releases_a_dead_holders_claim_and_leaves_a_live_ones() {
    let board = board();
    let run = "run-sweep-1";
    let dead_item = raise(board.as_ref(), run, "held by a dead agent");
    let live_item = raise(board.as_ref(), run, "held by a live agent");

    let dead_agent = register_background_agent(AgentStatus::Failed);
    let live_agent = register_background_agent(AgentStatus::Running);
    assert!(
        board
            .claim_board_item(&dead_item, &dead_agent.to_string())
            .expect("claim")
            .applied
    );
    assert!(
        board
            .claim_board_item(&live_item, &live_agent.to_string())
            .expect("claim")
            .applied
    );

    let released = release_dead_claims(board.as_ref(), run).expect("sweep");

    assert_eq!(released.len(), 1, "released: {released:?}");
    assert_eq!(released[0].item_id, dead_item);
    assert_eq!(released[0].holder, dead_agent.to_string());

    let dead_row = board.get_board_item(&dead_item).expect("get");
    assert_eq!(dead_row.claimed_by, None);
    assert_eq!(
        dead_row.status,
        BoardStatus::Open,
        "a released item must be available again"
    );

    let live_row = board.get_board_item(&live_item).expect("get");
    assert_eq!(
        live_row.claimed_by.as_deref(),
        Some(live_agent.to_string().as_str()),
        "the live agent's claim must survive the sweep"
    );
    assert_eq!(live_row.status, BoardStatus::Claimed);
}

/// The regression registering at the choke point exists to prevent: a
/// `TaskCreate` agent used to be findable only through `TASK_MANAGER`, so a
/// sweep that consulted `BACKGROUND_AGENTS` alone released a claim held by an
/// agent that was still working.
#[test]
fn the_sweep_leaves_a_claim_held_by_a_running_task_create_agent() {
    let board = board();
    let run = "run-sweep-2";
    let item = raise(board.as_ref(), run, "held by a TaskCreate agent");
    let agent = dispatch_task_agent(TaskStatus::Running);
    register_subagent(&agent, AgentStatus::Running);

    assert!(
        board
            .claim_board_item(&item, &agent)
            .expect("claim")
            .applied
    );

    let released = release_dead_claims(board.as_ref(), run).expect("sweep");

    assert!(
        released.is_empty(),
        "a running TaskCreate agent's claim was released: {released:?}"
    );
    let row = board.get_board_item(&item).expect("get");
    assert_eq!(row.claimed_by.as_deref(), Some(agent.as_str()));
    assert_eq!(row.status, BoardStatus::Claimed);
}

/// Issue #129: the sweep used to hand a pipeline agent's work to someone else
/// while it was still doing it.
///
/// The two guards come first on purpose. Both arms of the old fan-out have to
/// be genuinely blind to this agent, or the test would keep passing for a
/// reason that has nothing to do with the regression.
#[test]
fn the_sweep_leaves_a_claim_held_by_a_running_pipeline_agent() {
    let board = board();
    let run = "run-sweep-6";
    let item = raise(board.as_ref(), run, "held by a pipeline agent");
    let agent = spawn_pipeline_agent("sweep", AgentStatus::Running);

    assert_eq!(
        TASK_MANAGER.agent_is_running(&agent),
        None,
        "the premise of this test is that pipeline agents dispatch no task"
    );
    assert!(
        Uuid::parse_str(&agent).is_err(),
        "and that their id is not a UUID, so the old UUID-keyed lookup could not \
         have found them either"
    );

    assert!(
        board
            .claim_board_item(&item, &agent)
            .expect("claim")
            .applied
    );

    let released = release_dead_claims(board.as_ref(), run).expect("sweep");

    assert!(
        released.is_empty(),
        "a running pipeline agent's claim was released: {released:?}"
    );
    let row = board.get_board_item(&item).expect("get");
    assert_eq!(row.claimed_by.as_deref(), Some(agent.as_str()));
    assert_eq!(row.status, BoardStatus::Claimed);
}

/// And the other half: once the pipeline agent stops, its claim does come back.
#[test]
fn the_sweep_releases_a_finished_pipeline_agents_claim() {
    let board = board();
    let run = "run-sweep-7";
    let item = raise(
        board.as_ref(),
        run,
        "held by a pipeline agent that finished",
    );
    let agent = spawn_pipeline_agent("sweep-done", AgentStatus::Running);
    board.claim_board_item(&item, &agent).expect("claim");

    BACKGROUND_AGENTS.mark_terminal(&agent, AgentStatus::Finished);
    let released = release_dead_claims(board.as_ref(), run).expect("sweep");

    assert_eq!(released.len(), 1, "released: {released:?}");
    assert_eq!(released[0].holder, agent);
    assert_eq!(board.get_board_item(&item).expect("get").claimed_by, None);
}

/// The sweep is partitioned like everything else on the board: another run's
/// dead claims are not this run's business.
#[test]
fn the_sweep_touches_only_its_own_run() {
    let board = board();
    let mine = raise(board.as_ref(), "run-sweep-3", "mine");
    let theirs = raise(board.as_ref(), "run-sweep-4", "theirs");
    let dead = register_background_agent(AgentStatus::Cancelled).to_string();
    board.claim_board_item(&mine, &dead).expect("claim");
    board.claim_board_item(&theirs, &dead).expect("claim");

    let released = release_dead_claims(board.as_ref(), "run-sweep-3").expect("sweep");

    assert_eq!(released.len(), 1);
    assert_eq!(
        board.get_board_item(&theirs).expect("get").claimed_by,
        Some(dead),
        "another run's claim must be left alone"
    );
}

/// An item further along its lifecycle keeps that status when its holder dies:
/// losing the agent is not the same as retracting the work it recorded.
#[test]
fn a_released_item_in_review_keeps_its_status() {
    let board = board();
    let run = "run-sweep-5";
    let item = raise(board.as_ref(), run, "under review when the agent died");
    let dead = register_background_agent(AgentStatus::Finished).to_string();
    board.claim_board_item(&item, &dead).expect("claim");
    board
        .set_board_item_status(&item, BoardStatus::Claimed, BoardStatus::InReview)
        .expect("transition");

    release_dead_claims(board.as_ref(), run).expect("sweep");

    let row = board.get_board_item(&item).expect("get");
    assert_eq!(row.claimed_by, None);
    assert_eq!(row.status, BoardStatus::InReview);
}
