//! A host-installed capability, scoped to one audit attempt. No path arguments or shell access.
use std::sync::{Arc, Mutex};
use std::time::{Duration,Instant};
use serde_json::{Value,json};
use crate::tool::{Tool,ToolContext,ToolResult,ToolCapability,PermissionLevel,WorkingTreeEffect};
pub trait LandingHost: Send + Sync {
    fn land(&self, record: Value) -> Result<String,String>;
    fn hint(&self) -> Result<String,String>;
    fn complete(&self, value: &Value) -> Result<(),String>;
}
pub struct AuditLanding {
    host: Arc<dyn LandingHost>,
    path_timeout: Option<Duration>,
    last_landed: Mutex<Instant>,
}
impl AuditLanding {
    pub fn new(host: Arc<dyn LandingHost>, seconds: Option<u64>) -> Self {
        Self {host,path_timeout:seconds.map(Duration::from_secs),last_landed:Mutex::new(Instant::now())}
    }
    pub fn hint(&self) -> Result<String,String> { self.host.hint() }
    pub fn remaining(&self) -> Option<Duration> {
        self.path_timeout.map(|limit| limit.saturating_sub(self.last_landed.lock().unwrap().elapsed()))
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
        match landing.host.land(input) {
            Ok(hint)=>{ *landing.last_landed.lock().unwrap()=Instant::now(); ToolResult::success(hint) },
            Err(error)=>ToolResult::error(error),
        }
    }
    fn capability(&self)->ToolCapability { ToolCapability::HostLocal }
    fn permission_level(&self,_:&Value)->PermissionLevel { PermissionLevel::Safe }
    fn working_tree_effect(&self)->WorkingTreeEffect { WorkingTreeEffect::ExternalOnly }
}
