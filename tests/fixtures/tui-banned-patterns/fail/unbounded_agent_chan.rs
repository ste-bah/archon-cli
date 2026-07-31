// Fixture: triggers the UNBOUNDED_AGENT_CHAN rule.
// The construction below must match the regex
//     unbounded_channel::<\s*(TimestampedEvent|AgentEvent)\s*>|UnboundedSender<\s*(TimestampedEvent|AgentEvent)\s*>
// Do not "fix" this file - it is intentionally non-compliant.
//
// Replaces the former bounded.rs, which exercised BOUNDED_CHAN. That rule
// banned `mpsc::channel::<_>(256)`; TASK-AGS-102 made the bounded channel the
// required shape, so the banned construction is now the unbounded one.

use tokio::sync::mpsc;

struct AgentEvent;

fn make_channel() {
    let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();
    let _ = (tx, rx);
}
