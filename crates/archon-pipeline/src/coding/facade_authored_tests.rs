use super::*;
use crate::coding::agents::{Phase, ToolAccess};

#[tokio::test]
async fn authored_roster_runs_custom_phase_sequence_and_keys() {
    let mut first = AGENTS[0].clone();
    first.key = "authored-first";
    first.phase = Phase::Custom(42);
    first.depends_on = &[];
    first.tool_access = ToolAccess::Full;
    first.parallelizable = false;
    let mut second = first.clone();
    second.key = "authored-second";
    second.phase = Phase::Custom(1);
    second.depends_on = &["authored-first"];
    second.tool_access = ToolAccess::ReadOnly;
    let facade = CodingFacade::new().with_agents(vec![first, second]);
    let mut session = facade.init_session("declared task").await.unwrap();
    let NextAgent::Continue(first) = facade.next_agent(&session).await.unwrap() else {
        panic!("first")
    };
    assert_eq!((first.key.as_str(), first.phase), ("authored-first", 42));
    session
        .agent_results
        .push((first, super::tests::make_result("done")));
    let NextAgent::Continue(second) = facade.next_agent(&session).await.unwrap() else {
        panic!("second")
    };
    assert_eq!((second.key.as_str(), second.phase), ("authored-second", 1));
    assert!(matches!(
        second.tool_access_level,
        crate::runner::ToolAccessLevel::ReadOnly
    ));
}
