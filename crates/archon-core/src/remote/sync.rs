use super::RemoteSession;

/// Synchronizes files between local working directory and a remote agent workspace.
pub struct FileSyncer {
    /// Absolute path on the remote host where files should be mirrored.
    pub remote_dir: String,
}

impl FileSyncer {
    /// Upload files changed since last sync. Returns list of uploaded paths.
    /// Phase 5 placeholder: full SFTP upload deferred to Phase 6.
    pub async fn upload_changed(
        &self,
        local_dir: &std::path::Path,
        session: &RemoteSession,
    ) -> anyhow::Result<Vec<String>> {
        tracing::info!(
            "file sync: local {} → remote {} (session={}): upload deferred to phase 6",
            local_dir.display(),
            self.remote_dir,
            session.session_id
        );
        Ok(vec![])
    }

    /// Download files changed on remote since last sync. Returns list of downloaded paths.
    /// Phase 5 placeholder: full SFTP download deferred to Phase 6.
    pub async fn download_changed(
        &self,
        local_dir: &std::path::Path,
        session: &RemoteSession,
    ) -> anyhow::Result<Vec<String>> {
        tracing::info!(
            "file sync: remote {} → local {} (session={}): download deferred to phase 6",
            self.remote_dir,
            local_dir.display(),
            session.session_id
        );
        Ok(vec![])
    }
}
