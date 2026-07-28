use super::{ProviderEnvPolicy, ProviderEnvResolution, ProviderEnvSource};

impl ProviderEnvSource {
    pub fn policy(&self) -> Option<&ProviderEnvPolicy> {
        match self {
            Self::Policy(policy) | Self::ResolvedPolicy { policy, .. } => Some(policy),
            Self::Resolution(_) => None,
        }
    }

    pub fn resolution(&self) -> Option<&ProviderEnvResolution> {
        match self {
            Self::Resolution(resolution) | Self::ResolvedPolicy { resolution, .. } => {
                Some(resolution)
            }
            Self::Policy(_) => None,
        }
    }
}

impl std::fmt::Debug for ProviderEnvResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderEnvResolution")
            .field("proof", &self.proof)
            .field("resolved_key_count", &self.env.len())
            .finish()
    }
}

impl ProviderEnvResolution {
    pub fn covers(&self, policy: &ProviderEnvPolicy) -> bool {
        let checked = self
            .proof
            .redacted_env_keys_checked
            .iter()
            .map(|proof| proof.key.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        super::normalize_keys(policy.required_keys.clone())
            .iter()
            .all(|key| checked.contains(key.as_str()))
    }

    pub fn apply_to_env(&self, env: &mut Vec<(String, String)>) {
        for (key, value) in &self.env {
            if let Some(existing) = env.iter_mut().find(|(candidate, _)| candidate == key) {
                existing.1 = value.clone();
            } else {
                env.push((key.clone(), value.clone()));
            }
        }
    }

    pub fn redact_text(&self, text: &str) -> String {
        self.env
            .iter()
            .fold(text.to_string(), |current, (key, value)| {
                if value.is_empty() {
                    current
                } else {
                    current.replace(value, &format!("<redacted:{key}>"))
                }
            })
    }
}
