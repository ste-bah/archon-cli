use super::constants::MAX_OBSERVATION_LEN;
use super::helpers::epoch_secs;
use super::types::TrajectoryStep;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Step Capture Service
// ---------------------------------------------------------------------------

/// Per-trajectory step buffer for recording agent reasoning steps.
pub struct StepCaptureService {
    buffers: HashMap<String, Vec<TrajectoryStep>>,
}

impl StepCaptureService {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    /// Begin capturing steps for a trajectory.
    pub fn begin_capture(&mut self, trajectory_id: &str) {
        self.buffers.insert(trajectory_id.to_string(), Vec::new());
    }

    /// Record a step in the trajectory buffer.
    pub fn capture_step(
        &mut self,
        trajectory_id: &str,
        action: &str,
        observation: &str,
        reward: f64,
    ) {
        let buffer = self.buffers.entry(trajectory_id.to_string()).or_default();

        let step_index = buffer.len();

        // Truncate large observations
        let obs = if observation.len() > MAX_OBSERVATION_LEN {
            &observation[..MAX_OBSERVATION_LEN]
        } else {
            observation
        };

        buffer.push(TrajectoryStep {
            step_id: uuid::Uuid::new_v4().to_string(),
            trajectory_id: trajectory_id.to_string(),
            step_index,
            action: action.to_string(),
            observation: obs.to_string(),
            reward,
            timestamp: epoch_secs(),
        });
    }

    /// End capture and return all steps. Clears the buffer.
    pub fn end_capture(&mut self, trajectory_id: &str) -> Vec<TrajectoryStep> {
        self.buffers.remove(trajectory_id).unwrap_or_default()
    }
}

impl Default for StepCaptureService {
    fn default() -> Self {
        Self::new()
    }
}
