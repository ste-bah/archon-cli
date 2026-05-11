use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeDType {
    F32,
    F16,
    Int8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOrder {
    RowMajor,
    ColumnMajor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeTensorMetadata {
    pub name: String,
    pub dtype: BridgeDType,
    pub shape: Vec<usize>,
    pub memory_order: MemoryOrder,
    pub contains_nan_or_inf: bool,
}

impl BridgeTensorMetadata {
    pub fn new(name: impl Into<String>, dtype: BridgeDType, shape: Vec<usize>) -> Self {
        Self {
            name: name.into(),
            dtype,
            shape,
            memory_order: MemoryOrder::RowMajor,
            contains_nan_or_inf: false,
        }
    }

    pub fn with_memory_order(mut self, memory_order: MemoryOrder) -> Self {
        self.memory_order = memory_order;
        self
    }

    pub fn from_f32_values(
        name: impl Into<String>,
        dtype: BridgeDType,
        shape: Vec<usize>,
        values: &[f32],
    ) -> Self {
        let contains_nan_or_inf = values.iter().any(|value| !value.is_finite());
        Self {
            name: name.into(),
            dtype,
            shape,
            memory_order: MemoryOrder::RowMajor,
            contains_nan_or_inf,
        }
    }
}

pub fn output_cosine_parity(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("parity outputs must have matching dimensions");
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        Ok(0.0)
    } else {
        Ok(dot / (left_norm * right_norm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_metadata_preserves_dtype_shape_and_order() {
        let meta = BridgeTensorMetadata::new("transition.weight", BridgeDType::F32, vec![384, 768]);

        assert_eq!(meta.dtype, BridgeDType::F32);
        assert_eq!(meta.shape, vec![384, 768]);
        assert_eq!(meta.memory_order, MemoryOrder::RowMajor);
        assert!(!meta.contains_nan_or_inf);
    }

    #[test]
    fn bridge_metadata_flags_nan_and_inf_without_coercion() {
        let meta = BridgeTensorMetadata::from_f32_values(
            "projection.bias",
            BridgeDType::F16,
            vec![3],
            &[0.0, f32::NAN, f32::INFINITY],
        )
        .with_memory_order(MemoryOrder::ColumnMajor);

        assert_eq!(meta.dtype, BridgeDType::F16);
        assert_eq!(meta.memory_order, MemoryOrder::ColumnMajor);
        assert!(meta.contains_nan_or_inf);
    }

    #[test]
    fn bridge_metadata_covers_int8_quantized_tensors() {
        let meta = BridgeTensorMetadata::new("aux.failure.int8", BridgeDType::Int8, vec![16, 16]);

        assert_eq!(meta.dtype, BridgeDType::Int8);
        assert_eq!(meta.shape, vec![16, 16]);
    }

    #[test]
    fn fp32_forward_parity_can_pass_threshold() {
        let cosine = output_cosine_parity(&[1.0, 0.5, 0.25], &[1.0, 0.5, 0.249]).unwrap();

        assert!(cosine >= 0.95);
    }
}
