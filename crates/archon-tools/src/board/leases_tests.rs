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
    BACKGROUND_AGENTS
        .register(BackgroundAgentHandle {
            agent_id,
            join_handle: None,
            cancel_token: CancellationToken::new(),
            spawned_at: SystemTime::now(),
            status: Arc::new(Mutex::new(status)),
            result_slot: new_result_slot(),
        })
        .expect("fresh uuid cannot collide");
    agent_id
}

/// Dispatch a `TaskCreate`-style agent: a task in `TASK_MANAGER` carrying the
/// subagent id, and nothing at all in `BACKGROUND_AGENTS`.
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

#[test]
fn a_running_task_create_agent_is_live_despite_being_absent_from_background_agents() {
    let agent = dispatch_task_agent(TaskStatus::Running);
    assert_eq!(
        crate::background_agents::poll_background_agent(
            &Uuid::parse_str(&agent).expect("uuid-shaped")
        ),
        crate::background_agents::PollOutcome::Unknown,
        "the premise of this test is that TaskCreate agents are not in BACKGROUND_AGENTS"
    );
    assert_eq!(holder_liveness(&agent), HolderLiveness::Live);
}

#[test]
fn a_completed_task_create_agent_is_dead() {
    let agent = dispatch_task_agent(TaskStatus::Running);
    let task = TASK_MANAGER
        .list_tasks()
        .into_iter()
        .find(|task| task.agent_id.as_deref() == Some(agent.as_str()))
        .expect("dispatched task");
    TASK_MANAGER.set_status(&task.id, TaskStatus::Completed);
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

/// The regression the two-registry check exists to prevent: a sweep that only
/// consulted `BACKGROUND_AGENTS` would see `Unknown` here and release a claim
/// held by an agent that is still working.
#[test]
fn the_sweep_leaves_a_claim_held_by_a_running_task_create_agent() {
    let board = board();
    let run = "run-sweep-2";
    let item = raise(board.as_ref(), run, "held by a TaskCreate agent");
    let agent = dispatch_task_agent(TaskStatus::Running);

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
