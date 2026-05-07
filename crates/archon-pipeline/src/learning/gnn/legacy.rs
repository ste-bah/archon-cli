use super::GnnEnhancer;
use super::types::{ForwardResult, LayerWeights};

impl GnnEnhancer {
    /// Legacy: get weights as LayerWeights tuple (used by trainer.rs).
    pub fn get_weights(&self) -> (LayerWeights, LayerWeights, LayerWeights) {
        let w1 = self.weights.get_weights("layer1");
        let w2 = self.weights.get_weights("layer2");
        let w3 = self.weights.get_weights("layer3");
        let b1 = self.weights.get_bias("layer1");
        let b2 = self.weights.get_bias("layer2");
        let b3 = self.weights.get_bias("layer3");
        (
            LayerWeights {
                w: (*w1).clone(),
                bias: (*b1).clone(),
            },
            LayerWeights {
                w: (*w2).clone(),
                bias: (*b2).clone(),
            },
            LayerWeights {
                w: (*w3).clone(),
                bias: (*b3).clone(),
            },
        )
    }

    /// Legacy: set weights from LayerWeights tuple (used by trainer.rs).
    pub fn set_weights(&self, l1: LayerWeights, l2: LayerWeights, l3: LayerWeights) {
        self.weights.set_weights("layer1", l1.w, l1.bias);
        self.weights.set_weights("layer2", l2.w, l2.bias);
        self.weights.set_weights("layer3", l3.w, l3.bias);
    }

    /// Legacy: 1-arg enhance for backward compat with trainer.rs validate().
    /// Redirects to the new 4-arg signature.
    pub fn enhance_legacy(&self, embedding: &[f32]) -> ForwardResult {
        self.enhance(embedding, None, None, false)
    }
}
