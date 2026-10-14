use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::super::store_file::read_store_file;
use super::WorkflowV2ResultStore;
use crate::{WorkflowError, WorkflowResult};

impl WorkflowV2ResultStore {
    /// Read the typed, judge-only observation for a receipt-verified landing.
    /// These observations live outside call history: they neither answer a
    /// script call nor represent a publication.
    pub fn load_landing_rejudge_observation<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> WorkflowResult<Option<T>> {
        let path = self.landing_rejudge_observation_path(key)?;
        match read_store_file(&path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(WorkflowError::io(&path, error)),
        }
    }

    /// Persist one typed judge-only observation under its content-derived key.
    pub fn save_landing_rejudge_observation<T: Serialize>(
        &self,
        key: &str,
        observation: &T,
    ) -> WorkflowResult<()> {
        let path = self.landing_rejudge_observation_path(key)?;
        self.write_record(&path, observation)
    }

    fn landing_rejudge_observation_path(&self, key: &str) -> WorkflowResult<PathBuf> {
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(WorkflowError::StateCorrupt(
                "landing rejudge observation key is not a content digest".into(),
            ));
        }
        Ok(self
            .root
            .join("landing-rejudgements")
            .join(format!("{key}.json")))
    }
}
