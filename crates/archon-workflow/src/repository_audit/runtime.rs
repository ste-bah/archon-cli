use crate::*;
use super::{budget::AuditPolicy,AuditRecord};
use std::path::PathBuf;
pub struct Snapshot{pub identity:String,pub root:PathBuf,pub paths:Vec<String>}
pub struct AuditRuntime;
impl AuditRuntime{
 pub fn initialize(_:WorkflowStore,_:String,_:AuditPolicy)->WorkflowResult<Self>{Ok(Self)}
 pub async fn assess(&self,_:&Snapshot,_:&[String],_:&str,_:&dyn WorkflowAgentDispatch)->WorkflowResult<()>{Ok(())}
 pub fn records_for(&self,_:&[String])->WorkflowResult<Vec<AuditRecord>>{Ok(vec![])}
 pub fn require_closed(&self,_:&str)->WorkflowResult<()>{Ok(())}
}
