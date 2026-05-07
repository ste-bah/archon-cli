/// Weight initialization strategy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Initialization {
    /// Kaiming He initialization for ReLU variants: scale = sqrt(2.0 / fan_in)
    He,
    /// Xavier/Glorot initialization: scale = sqrt(2.0 / (fan_in + fan_out))
    Xavier,
}
