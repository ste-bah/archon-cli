use super::{AuditContract,AuditReport};
use crate::WorkflowResult;
use serde::{Serialize,Deserialize};
#[derive(Clone,Debug,Default,Serialize,Deserialize)]
pub struct AuditLedger { pub history:Vec<AuditReport> }
impl AuditLedger {
 pub fn accept(&mut self,_:AuditContract,r:AuditReport)->WorkflowResult<()> { self.history.push(r);Ok(()) }
 pub fn unresolved(&self,_:&str)->WorkflowResult<Vec<String>> { Ok(vec![]) }
 pub fn propose(&mut self,_:&str,_:String) {}
 pub fn record_applied(&mut self,_:&str,_:String) {}
}
