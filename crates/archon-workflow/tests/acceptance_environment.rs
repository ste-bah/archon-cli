#[path="support/native_fixture.rs"] mod support;
use archon_workflow::acceptance_scratch::{ScratchPolicy,observe_commands};
#[test]
fn configured_host_values_cross_only_at_execution_and_never_enter_evidence(){
    let mut child=std::process::Command::new(std::env::current_exe().unwrap());
    child.args(["--exact","environment_child","--ignored","--nocapture"])
        .env("FIXTURE_ALLOWED_TOKEN","secret-acceptance-canary-98e3")
        .env("FIXTURE_NOT_ALLOWED","ambient-must-not-cross");
    assert!(child.status().unwrap().success());
}
#[tokio::test]
#[ignore="private environment subprocess"]
async fn environment_child(){
    let(t,p,commit,c,refs)=support::fixture("test -n \"$FIXTURE_ALLOWED_TOKEN\" && test -z \"${FIXTURE_NOT_ALLOWED:-}\" && printf '%s' \"$FIXTURE_ALLOWED_TOKEN\"");
    let mut raw=serde_json::to_value(&p).unwrap();
    raw["environment_allowlist"]=serde_json::json!(["FIXTURE_ALLOWED_TOKEN","FIXTURE_ABSENT"]);
    let policy:ScratchPolicy=serde_json::from_value(raw).expect("allowlist must be supported");
    let evidence=t.path().join("evidence");
    let out=observe_commands(&policy,&commit,&c,"chain",&refs,&evidence).await.unwrap();
    assert!(out.passed(),"{:?}",out.operational_errors);
    let encoded=serde_json::to_string(&out).unwrap();
    assert!(!encoded.contains("secret-acceptance-canary-98e3"));
    assert!(!format!("{out:?}").contains("secret-acceptance-canary-98e3"));
    assert!(!String::from_utf8_lossy(&out.checks[0].stdout).contains("secret-acceptance-canary-98e3"));
    let value:serde_json::Value=serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["host_environment"]["FIXTURE_ALLOWED_TOKEN"],true);
    assert_eq!(value["host_environment"]["FIXTURE_ABSENT"],false);
}
