// Fixture: triggers the BOUNDED_CHAN rule.
// The literal expression below must match the regex
//     mpsc::channel::<[^>]*>\(\s*256\s*\)
// Do not "fix" this file - it is intentionally non-compliant.

use tokio::sync::mpsc;

struct AgentEvent;

fn make_channel() {
    let (tx, rx) = mpsc::channel::<AgentEvent>(256);
    let _ = (tx, rx);
}
