use super::super::backprop;
use super::super::math::ActivationType;
use super::super::{GnnEnhancer, LayerActivationCache, LayerWeights};

pub(super) fn accumulate_embedding_grads(
    accumulated: &mut Option<Vec<(Vec<Vec<f32>>, Vec<f32>)>>,
    caches: &[LayerActivationCache],
    weights: [&LayerWeights; 3],
    grad: &[f32],
) {
    if caches.len() != 3 {
        return;
    }
    let grads = backprop::full_backward(
        caches,
        weights,
        grad,
        [
            ActivationType::LeakyRelu,
            ActivationType::LeakyRelu,
            ActivationType::Tanh,
        ],
    );
    let layer_grads: Vec<(Vec<Vec<f32>>, Vec<f32>)> =
        grads.into_iter().map(|grad| (grad.dw, grad.db)).collect();
    match accumulated {
        Some(acc) => add_grads_in_place(acc, &layer_grads),
        None => *accumulated = Some(layer_grads),
    }
}

fn add_grads_in_place(
    acc: &mut [(Vec<Vec<f32>>, Vec<f32>)],
    layer_grads: &[(Vec<Vec<f32>>, Vec<f32>)],
) {
    for (i, (dw, db)) in layer_grads.iter().enumerate() {
        for (row_a, row_g) in acc[i].0.iter_mut().zip(dw.iter()) {
            for (a, g) in row_a.iter_mut().zip(row_g.iter()) {
                *a += *g;
            }
        }
        for (a, g) in acc[i].1.iter_mut().zip(db.iter()) {
            *a += *g;
        }
    }
}

pub(super) fn average_grads(
    grads: Vec<(Vec<Vec<f32>>, Vec<f32>)>,
    divisor: f32,
) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
    if divisor <= 0.0 {
        return grads;
    }
    grads
        .into_iter()
        .map(|(dw, db)| {
            let dw = dw
                .into_iter()
                .map(|row| row.into_iter().map(|value| value / divisor).collect())
                .collect();
            let db = db.into_iter().map(|value| value / divisor).collect();
            (dw, db)
        })
        .collect()
}

pub(super) fn scale_grads(
    grads: &[(Vec<Vec<f32>>, Vec<f32>)],
    scale: f32,
) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
    grads
        .iter()
        .map(|(dw, db)| {
            let dw = dw
                .iter()
                .map(|row| row.iter().map(|value| value * scale).collect())
                .collect();
            let db = db.iter().map(|value| value * scale).collect();
            (dw, db)
        })
        .collect()
}

pub(super) fn pad_gradient(mut grad: Vec<f32>, dim: usize) -> Vec<f32> {
    grad.resize(dim, 0.0);
    grad
}

pub(super) fn zero_grads(enhancer: &GnnEnhancer) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
    let (l1, l2, l3) = enhancer.get_weights();
    vec![
        (
            vec![vec![0.0; l1.w[0].len()]; l1.w.len()],
            vec![0.0; l1.bias.len()],
        ),
        (
            vec![vec![0.0; l2.w[0].len()]; l2.w.len()],
            vec![0.0; l2.bias.len()],
        ),
        (
            vec![vec![0.0; l3.w[0].len()]; l3.w.len()],
            vec![0.0; l3.bias.len()],
        ),
    ]
}
