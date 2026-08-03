// Dry-run plan extraction: QuickJS is the single grammar for workflow
// scripts. Validation IS execution against a recording host — a syntax error
// or policy violation surfaces as a hard error with the engine diagnostic,
// and the recorded typed calls are the approval-time plan preview.
//
// Included into workflow_live_v2_script.rs so it shares the script bridge
// (`script_source`) and the live host's typed payload parsing
// (`ScriptHostRequest`, `parse_script_options`) — one deserialization path
// for live and dry-run, no second interpretation of script source text.

use super::*;

include!("workflow_live_v2_script_dry_run_a.rs");
include!("workflow_live_v2_script_dry_run_b.rs");
