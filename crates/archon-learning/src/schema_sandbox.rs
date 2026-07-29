use anyhow::Result;
use cozo::DbInstance;

use super::run_create;

pub(super) fn ensure_sandbox_runtime_events(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create sandbox_runtime_events {
            event_id: String =>
            backend_kind: String,
            backend_instance_id: String default "",
            agent_type: String default "",
            run_id: String default "",
            tool_name: String default "",
            decision: String,
            reason_code: String default "",
            sandbox_profile_id: String default "",
            workspace_mode: String default "",
            network_mode: String default "",
            workspace_mount_mode: String default "",
            redacted_context_json: String default "{}",
            created_at: String,
        }"#,
    )
}

pub(super) fn ensure_sandbox_profiles(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create sandbox_profiles {
            sandbox_profile_id: String =>
            backend_kind: String,
            display_name: String default "",
            default_network_mode: String default "",
            workspace_mount_mode: String default "",
            writable_paths_json: String default "[]",
            env_allowlist_json: String default "[]",
            resource_limits_json: String default "{}",
            created_at: String,
            updated_at: String,
        }"#,
    )
}

pub(super) fn ensure_sandbox_sessions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create sandbox_sessions {
            sandbox_session_id: String =>
            backend_kind: String,
            sandbox_profile_id: String,
            run_id: String default "",
            agent_type: String default "",
            backend_instance_id: String default "",
            workspace_mode: String default "",
            canonical_workspace: String default "",
            transport_kind: String default "",
            transport_endpoint_redacted: String default "",
            provider_injection_enabled: Bool default false,
            status: String,
            created_at: String,
            updated_at: String,
        }"#,
    )
}
