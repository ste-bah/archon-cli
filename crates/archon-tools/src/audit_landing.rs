//! A host-installed capability, scoped to one audit attempt. No path arguments or shell access.
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
use serde_json::{Value,json};
use crate::tool::{Tool,ToolContext,ToolResult,ToolCapability,PermissionLevel,WorkingTreeEffect};
pub trait LandingHost: Send + Sync {
    fn land(&self, record: Value) -> Result<String,String>;
    fn hint(&self) -> Result<String,String>;
    fn complete(&self, value: &Value) -> Result<(),String>;
}
struct Progress { last_landed:Instant, last_nudge:Option<Instant>, nudges:u8 }
pub struct AuditLanding {
    host: Arc<dyn LandingHost>,
    path_timeout: Option<Duration>,
    progress: Mutex<Progress>,
    landed: Mutex<std::collections::BTreeSet<String>>,
}
impl AuditLanding {
    pub fn new(host: Arc<dyn LandingHost>, seconds: Option<u64>) -> Self {
        Self {host,path_timeout:seconds.map(Duration::from_secs),progress:Mutex::new(Progress {last_landed:Instant::now(),last_nudge:None,nudges:0}),landed:Mutex::new(Default::default())}
    }
    pub fn hint(&self) -> Result<String,String> { self.host.hint() }
    /// Consult only at a completed turn boundary, never as an inference timeout.
    pub fn progress_message(&self) -> Result<Option<String>,String> {
        let Some(interval)=self.path_timeout else {return Ok(None)};
        let mut progress=self.progress.lock().unwrap();
        let elapsed=progress.last_landed.elapsed();
        let since=progress.last_nudge.unwrap_or(progress.last_landed).elapsed();
        if since < interval {return Ok(None);}
        if progress.nudges >= 2 {
            return Err(format!("audit progress deadline ({} s since last landed record): two consecutive nudges produced no new landings",elapsed.as_secs()));
        }
        progress.nudges+=1;progress.last_nudge=Some(Instant::now());
        Ok(Some(format!("No record landed for {} s. Land every path you have established now, then continue. Progress nudge {}/2.",elapsed.as_secs(),progress.nudges)))
    }
    pub fn complete(&self, value: &Value) -> Result<(),String> { self.host.complete(value) }
}
tokio::task_local! { static CURRENT: Arc<AuditLanding>; }
pub async fn scope<T>(landing:Arc<AuditLanding>, work:impl std::future::Future<Output=T>) -> T { CURRENT.scope(landing,work).await }
pub fn current() -> Option<Arc<AuditLanding>> { CURRENT.try_with(Clone::clone).ok() }
pub struct LandAuditRecordTool;
#[async_trait::async_trait]
impl Tool for LandAuditRecordTool {
    fn name(&self)->&str { "land-audit-record" }
    fn description(&self)->&str { "Persist one established AuditRecord to the host's snapshot-bound audit store. No repository mutation. Returns the remaining paths. Land each record as soon as established." }
    fn input_schema(&self)->Value { json!({"type":"object","required":["declared_path","verdict","equivalents","required_action","reason"],"additionalProperties":false,"properties":{
        "declared_path":{"type":"string"},"verdict":{"type":"string","enum":["exists_as_declared","absent","exists_elsewhere","unreachable"]},
        "equivalents":{"type":"array","items":{"type":"string"}},"required_action":{"type":"string","enum":["none","deliver","wire_or_migrate"]},"reason":{"type":"string"}}}) }
    async fn execute(&self,input:Value,ctx:&ToolContext)->ToolResult {
        let Some(landing)=&ctx.audit_landing else { return ToolResult::error("land-audit-record requires host audit authority"); };
        let path=input.get("declared_path").and_then(Value::as_str).unwrap_or("").to_string();
        match landing.host.land(input) {
            Ok(hint)=>{
                if landing.landed.lock().unwrap().insert(path) {
                    *landing.progress.lock().unwrap()=Progress {last_landed:Instant::now(),last_nudge:None,nudges:0};
                }
                ToolResult::success(hint)
            },
            Err(error)=>ToolResult::error(error),
        }
    }
    fn capability(&self)->ToolCapability { ToolCapability::HostLocal }
    fn permission_level(&self,_:&Value)->PermissionLevel { PermissionLevel::Safe }
    fn working_tree_effect(&self)->WorkingTreeEffect { WorkingTreeEffect::ExternalOnly }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    struct Host;
    impl LandingHost for Host {
        fn land(&self,_:Value)->Result<String,String>{Ok("landed".into())}
        fn hint(&self)->Result<String,String>{Ok("remaining".into())}
        fn complete(&self,_:&Value)->Result<(),String>{Ok(())}
    }
    #[tokio::test(start_paused = true)]
    async fn progress_nudges_twice_before_failure_and_new_landing_resets() {
        let landing=Arc::new(AuditLanding::new(Arc::new(Host),Some(900)));
        tokio::time::advance(Duration::from_secs(720)).await;
        assert!(landing.progress_message().unwrap().is_none());
        tokio::time::advance(Duration::from_secs(180)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("1/2"));
        assert!(landing.progress_message().unwrap().is_none());
        tokio::time::advance(Duration::from_secs(900)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("2/2"));
        let ctx=ToolContext {audit_landing:Some(landing.clone()),..Default::default()};
        assert!(!LandAuditRecordTool.execute(json!({"declared_path":"new"}),&ctx).await.is_error);
        tokio::time::advance(Duration::from_secs(900)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("1/2"));
        tokio::time::advance(Duration::from_secs(900)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("2/2"));
        tokio::time::advance(Duration::from_secs(900)).await;
        let error=landing.progress_message().unwrap_err();
        assert!(error.contains("audit progress deadline (2700 s"));
        assert!(!error.contains("wall-clock"));
    }
}
