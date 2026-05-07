use std::sync::atomic::Ordering;

use super::GnnEnhancer;
use super::math::ActivationType;
use super::weights::Initialization;

impl GnnEnhancer {
    // -----------------------------------------------------------------------
    // Private: weight initialization
    // -----------------------------------------------------------------------

    pub(super) fn initialize_layer_weights(&self) {
        let init = match self.config.activation {
            ActivationType::Relu | ActivationType::LeakyRelu => Initialization::He,
            ActivationType::Tanh | ActivationType::Sigmoid => Initialization::Xavier,
        };

        // Intermediate dimensions
        let intermediate_dim1 = 1536 * 2 / 3; // 1024
        let intermediate_dim2 = 1536 * 5 / 6; // 1280

        let layers: &[(&str, usize, usize)] = &[
            (
                "input_projection",
                self.config.input_dim,
                self.config.input_dim,
            ),
            ("layer1", self.config.input_dim, intermediate_dim1),
            ("layer2", intermediate_dim1, intermediate_dim2),
            ("layer3", intermediate_dim2, self.config.output_dim),
            (
                "feature_projection",
                self.config.input_dim,
                self.config.input_dim,
            ),
        ];

        for (i, &(id, in_dim, out_dim)) in layers.iter().enumerate() {
            self.weights
                .initialize(id, in_dim, out_dim, init, self.weight_seed + i as u64);
        }

        self.weights_loaded.store(true, Ordering::Relaxed);
    }
}
