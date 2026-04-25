// Fixture: triggers the INLINE_AGENT_AWAIT rule.
// The call site below must match the regex
//     process_message\([^)]*\)\s*\.await
// Do not "fix" this file - it is intentionally non-compliant.

struct Agent;

impl Agent {
    async fn process_message(&self, _prompt: &str) -> Result<(), ()> {
        Ok(())
    }
}

async fn driver() {
    let agent = Agent;
    let prompt = String::from("hello");
    let _ = agent.process_message(&prompt).await;
}
