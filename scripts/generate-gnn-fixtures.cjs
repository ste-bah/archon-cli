#!/usr/bin/env node
/**
 * Generate GNN parity test fixtures.
 *
 * Implements the GNN forward pass identically to root archon TS:
 *   gnn-enhancer.ts + gnn-math.ts + gnn-cache.ts
 *
 * Usage: node scripts/generate-gnn-fixtures.js [--seed 12345] [--output path]
 *
 * Output: JSON file with 10 input fixtures, 1 graph fixture, and initialized
 * layer weights — all deterministically generated from the given seed.
 */

const fs = require('fs');

// ---------------------------------------------------------------------------
// Config (matching DEFAULT_GNN_CONFIG in gnn-enhancer.ts:110-120)
// ---------------------------------------------------------------------------
const CONFIG = {
    inputDim: 1536,
    outputDim: 1536,
    numLayers: 3,
    attentionHeads: 12,
    dropout: 0.1,
    maxNodes: 50,
    useResidual: true,
    useLayerNorm: true,
    activation: 'relu',
};

const INTERMEDIATE_DIM1 = Math.floor(1536 * 2 / 3);  // 1024
const INTERMEDIATE_DIM2 = Math.floor(1536 * 5 / 6);  // 1280

const LAYERS = [
    { id: 'input_projection', inDim: 1536, outDim: 1536 },
    { id: 'layer1', inDim: 1536, outDim: INTERMEDIATE_DIM1 },
    { id: 'layer2', inDim: INTERMEDIATE_DIM1, outDim: INTERMEDIATE_DIM2 },
    { id: 'layer3', inDim: INTERMEDIATE_DIM2, outDim: 1536 },
    { id: 'feature_projection', inDim: 1536, outDim: 1536 },
];

// ---------------------------------------------------------------------------
// Seeded PRNG (xoshiro128** — simple but sufficient for deterministic weights)
// ---------------------------------------------------------------------------
class SeededRng {
    constructor(seed) {
        // SplitMix64 to seed 4 state words
        this.s = new Uint32Array(4);
        for (let i = 0; i < 4; i++) {
            seed = (seed + 0x9e3779b9) | 0;
            let z = seed;
            z = Math.imul(z ^ (z >>> 16), 0x85ebca6b);
            z = Math.imul(z ^ (z >>> 13), 0xc2b2ae35);
            z = (z ^ (z >>> 16)) >>> 0;
            this.s[i] = z;
        }
    }

    /** Generate a float in [-1, 1] range */
    nextFloat() {
        // xoshiro128** next()
        const s = this.s;
        const result = Math.imul(s[1] * 5, 0x7FFFFFFF);
        const t = s[1] << 9;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = (s[3] << 11) | (s[3] >>> 21);
        return (result >>> 0) / 0xFFFFFFFF - 0.5;  // Scale to [-0.5, 0.5]
    }
}

// ---------------------------------------------------------------------------
// Math primitives (matching gnn-math.ts exactly)
// ---------------------------------------------------------------------------

function addVectors(a, b) {
    const maxLen = Math.max(a.length, b.length);
    const result = new Float32Array(maxLen);
    for (let i = 0; i < maxLen; i++) {
        const av = i < a.length ? a[i] : 0;
        const bv = i < b.length ? b[i] : 0;
        result[i] = av + bv;
    }
    return result;
}

function zeroPad(embedding, targetDim) {
    if (embedding.length >= targetDim) return embedding.slice(0, targetDim);
    const padded = new Float32Array(targetDim);
    padded.set(embedding);
    return padded;
}

function normalize(embedding) {
    let magnitude = 0;
    for (let i = 0; i < embedding.length; i++) magnitude += embedding[i] * embedding[i];
    magnitude = Math.sqrt(magnitude);
    if (magnitude === 0) return new Float32Array(embedding);
    const result = new Float32Array(embedding.length);
    for (let i = 0; i < embedding.length; i++) result[i] = embedding[i] / magnitude;
    return result;
}

function applyActivation(embedding, activation) {
    const result = new Float32Array(embedding.length);
    for (let i = 0; i < embedding.length; i++) {
        const x = embedding[i];
        switch (activation) {
            case 'relu': result[i] = Math.max(0, x); break;
            case 'tanh': result[i] = Math.tanh(x); break;
            case 'sigmoid': result[i] = 1 / (1 + Math.exp(-x)); break;
            case 'leaky_relu': result[i] = x > 0 ? x : 0.01 * x; break;
            default: result[i] = x;
        }
    }
    return result;
}

function project(embedding, weights, outputDim) {
    const result = new Float32Array(outputDim);
    for (let o = 0; o < outputDim; o++) {
        let sum = 0;
        const w = weights[o] || new Float32Array(embedding.length);
        for (let i = 0; i < embedding.length && i < w.length; i++) {
            sum += embedding[i] * w[i];
        }
        result[o] = sum;
    }
    return result;
}

function softmax(scores) {
    if (scores.length === 0) return new Float32Array(0);
    let maxScore = scores[0];
    for (let i = 1; i < scores.length; i++) {
        if (scores[i] > maxScore) maxScore = scores[i];
    }
    const expScores = new Float32Array(scores.length);
    let sumExp = 0;
    for (let i = 0; i < scores.length; i++) {
        expScores[i] = Math.exp(scores[i] - maxScore);
        sumExp += expScores[i];
    }
    const result = new Float32Array(scores.length);
    if (sumExp === 0) {
        const uniform = 1.0 / scores.length;
        for (let i = 0; i < scores.length; i++) result[i] = uniform;
    } else {
        for (let i = 0; i < scores.length; i++) result[i] = expScores[i] / sumExp;
    }
    return result;
}

function attentionScore(query, key, scale) {
    const minLen = Math.min(query.length, key.length);
    if (minLen === 0) return 0;
    let dotProduct = 0;
    for (let i = 0; i < minLen; i++) dotProduct += query[i] * key[i];
    const scaleFactor = scale !== undefined ? scale : 1.0 / Math.sqrt(minLen);
    return dotProduct * scaleFactor;
}

function weightedAggregate(features, attentionWeights) {
    if (features.length === 0 || attentionWeights.length === 0) return new Float32Array(0);
    const dim = features[0].length;
    const result = new Float32Array(dim);
    const numFeatures = Math.min(features.length, attentionWeights.length);
    for (let f = 0; f < numFeatures; f++) {
        const weight = attentionWeights[f];
        const feature = features[f];
        const featureLen = Math.min(feature.length, dim);
        for (let i = 0; i < featureLen; i++) result[i] += weight * feature[i];
    }
    return result;
}

// ---------------------------------------------------------------------------
// Weight initialization (matching gnn-enhancer.ts initializeLayerWeights)
// ---------------------------------------------------------------------------

function heInit(inDim, outDim, rng) {
    const scale = Math.sqrt(2.0 / inDim);
    const weights = [];
    for (let o = 0; o < outDim; o++) {
        const row = new Float32Array(inDim);
        for (let i = 0; i < inDim; i++) {
            row[i] = rng.nextFloat() * 2.0 * scale;  // [-scale, scale]
        }
        weights.push(row);
    }
    return weights;
}

function xavierInit(inDim, outDim, rng) {
    const scale = Math.sqrt(2.0 / (inDim + outDim));
    const weights = [];
    for (let o = 0; o < outDim; o++) {
        const row = new Float32Array(inDim);
        for (let i = 0; i < inDim; i++) {
            row[i] = rng.nextFloat() * 2.0 * scale;  // [-scale, scale]
        }
        weights.push(row);
    }
    return weights;
}

function initAllWeights(seed) {
    const activation = CONFIG.activation;
    const isHe = activation === 'relu' || activation === 'leaky_relu';

    const layerWeights = {};
    for (let i = 0; i < LAYERS.length; i++) {
        const layer = LAYERS[i];
        const rng = new SeededRng(seed + i);
        const init = isHe ? heInit : xavierInit;
        layerWeights[layer.id] = {
            inDim: layer.inDim,
            outDim: layer.outDim,
            weights: init(layer.inDim, layer.outDim, rng),
            seed: seed + i,
        };
    }
    return layerWeights;
}

// ---------------------------------------------------------------------------
// Forward pass (matching gnn-enhancer.ts enhance + applyLayer)
// ---------------------------------------------------------------------------

function prepareInput(embedding, weights) {
    let prepared = embedding;
    if (embedding.length !== CONFIG.inputDim) {
        const w = weights['input_projection'].weights;
        prepared = project(embedding, w, CONFIG.inputDim);
    }
    return normalize(prepared);
}

function applyLayer(input, outputDim, layerNum, weights, config) {
    const layerId = 'layer' + layerNum;
    const w = weights[layerId].weights;
    let output = project(input, w, outputDim);
    output = applyActivation(output, config.activation);
    if (config.useResidual && input.length === output.length) {
        output = addVectors(output, input);
        output = normalize(output);
    }
    if (config.useLayerNorm) {
        output = normalize(output);
    }
    return output;
}

function forwardPass(embedding, weights) {
    try {
        let current = prepareInput(embedding, weights);
        current = applyLayer(current, INTERMEDIATE_DIM1, 1, weights, CONFIG);
        current = applyLayer(current, INTERMEDIATE_DIM2, 2, weights, CONFIG);
        current = applyLayer(current, CONFIG.outputDim, 3, weights, CONFIG);

        // NaN check
        for (let i = 0; i < current.length; i++) {
            if (isNaN(current[i]) || !isFinite(current[i])) {
                throw new Error('NaN/Inf in output');
            }
        }

        return normalize(current);
    } catch (e) {
        return zeroPad(embedding, CONFIG.outputDim);
    }
}

// ---------------------------------------------------------------------------
// Fixture generation
// ---------------------------------------------------------------------------

function generateInputFixtures(seed) {
    const rng = new SeededRng(seed);
    const fixtures = [];

    // 5 random Float32Arrays
    for (let i = 1; i <= 5; i++) {
        const input = new Float32Array(CONFIG.inputDim);
        for (let j = 0; j < CONFIG.inputDim; j++) {
            input[j] = rng.nextFloat() * 2.0;
        }
        fixtures.push({ name: 'random_' + i, input: Array.from(input) });
    }

    // all-zeros
    fixtures.push({
        name: 'all_zeros',
        input: Array.from(new Float32Array(CONFIG.inputDim)),
    });

    // all-ones
    fixtures.push({
        name: 'all_ones',
        input: Array.from(new Float32Array(CONFIG.inputDim).fill(1.0)),
    });

    // single non-zero at position 42
    const singleNonZero = new Float32Array(CONFIG.inputDim);
    singleNonZero[42] = 1.0;
    fixtures.push({ name: 'single_nonzero', input: Array.from(singleNonZero) });

    // alternating +/- 1
    const alt = new Float32Array(CONFIG.inputDim);
    for (let i = 0; i < CONFIG.inputDim; i++) alt[i] = i % 2 === 0 ? 1.0 : -1.0;
    fixtures.push({ name: 'alternating', input: Array.from(alt) });

    // NaN input (should produce zero-padded fallback)
    const nanInput = new Float32Array(CONFIG.inputDim).fill(NaN);
    fixtures.push({ name: 'nan_input', input: Array.from(nanInput) });

    return fixtures;
}

function generateGraphFixture(seed) {
    // 3-node graph with center embedding
    const center = new Float32Array(CONFIG.inputDim);
    for (let i = 0; i < CONFIG.inputDim; i++) center[i] = 0.1 + (i % 10) * 0.01;

    const graph = {
        nodes: [
            { id: 'A', embedding: Array.from(center) },
            { id: 'B', embedding: Array.from(new Float32Array(CONFIG.inputDim).fill(0.2)) },
            { id: 'C', embedding: Array.from(new Float32Array(CONFIG.inputDim).fill(0.05)) },
        ],
        edges: [
            { source: 'A', target: 'B', weight: 0.9 },
            { source: 'B', target: 'C', weight: 0.1 },
        ],
    };

    return {
        name: 'graph_3node',
        center: Array.from(center),
        graph,
    };
}

// ---------------------------------------------------------------------------
// Graph enhancement path (matching gnn-enhancer.ts enhanceWithGraph)
// ---------------------------------------------------------------------------

function pruneGraph(graph) {
    if (graph.nodes.length <= CONFIG.maxNodes) return graph;

    const nodeScores = new Map();
    for (const node of graph.nodes) nodeScores.set(node.id, 0);
    if (graph.edges) {
        for (const edge of graph.edges) {
            nodeScores.set(edge.source, (nodeScores.get(edge.source) || 0) + edge.weight);
            nodeScores.set(edge.target, (nodeScores.get(edge.target) || 0) + edge.weight);
        }
    }

    const sorted = [...graph.nodes].sort((a, b) =>
        (nodeScores.get(b.id) || 0) - (nodeScores.get(a.id) || 0)
    );
    const prunedNodes = sorted.slice(0, CONFIG.maxNodes);
    const nodeSet = new Set(prunedNodes.map(n => n.id));
    const prunedEdges = (graph.edges || []).filter(e => nodeSet.has(e.source) && nodeSet.has(e.target));

    return { nodes: prunedNodes, edges: prunedEdges };
}

function buildFeatureMatrix(graph, weights) {
    return graph.nodes.map(node => {
        const emb = new Float32Array(node.embedding);
        if (emb.length !== CONFIG.inputDim) {
            const w = weights['feature_projection'].weights;
            return project(emb, w, CONFIG.inputDim);
        }
        return emb;
    });
}

function buildAdjacencyMatrix(graph) {
    const n = graph.nodes.length;
    const nodeIndex = new Map();
    graph.nodes.forEach((node, idx) => nodeIndex.set(node.id, idx));

    if (!graph.edges || graph.edges.length === 0) {
        const matrix = [];
        for (let i = 0; i < n; i++) {
            const row = new Float32Array(n);
            for (let j = 0; j < n; j++) {
                if (i !== j) row[j] = 1.0 / (n - 1);
            }
            matrix.push(row);
        }
        return matrix;
    }

    const matrix = [];
    for (let i = 0; i < n; i++) matrix.push(new Float32Array(n));

    for (const edge of graph.edges) {
        const si = nodeIndex.get(edge.source);
        const ti = nodeIndex.get(edge.target);
        if (si !== undefined && ti !== undefined) {
            matrix[si][ti] = edge.weight;
            matrix[ti][si] = edge.weight;
        }
    }
    return matrix;
}

function aggregateNeighborhood(center, features, adjacency) {
    if (features.length === 0) return new Float32Array(center);

    const n = features.length;

    // Node importance
    const importance = new Float32Array(n);
    for (let i = 0; i < n; i++) {
        let total = 0;
        for (let j = 0; j < adjacency.length && j < n; j++) {
            if (adjacency[i] && adjacency[i][j]) total += adjacency[i][j];
            if (adjacency[j] && adjacency[j][i]) total += adjacency[j][i];
        }
        importance[i] = total;
    }

    // Raw scores
    const rawScores = [];
    for (let j = 0; j < n; j++) {
        const base = attentionScore(center, features[j]);
        const bonus = Math.log(importance[j] + 1);
        rawScores.push(base + bonus);
    }

    const attnWeights = softmax(new Float32Array(rawScores));
    const aggregated = weightedAggregate(features, attnWeights);
    const result = addVectors(center, aggregated);
    return normalize(result);
}

function enhanceWithGraph(centerEmbedding, graph, weights) {
    const pruned = pruneGraph(graph);
    const features = buildFeatureMatrix(pruned, weights);
    const adjacency = buildAdjacencyMatrix(pruned);
    const center = new Float32Array(centerEmbedding);
    const aggregated = aggregateNeighborhood(center, features, adjacency);
    return forwardPass(aggregated, weights);
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main() {
    const args = process.argv.slice(2);
    let seed = 12345;
    let outputPath = null;

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--seed' && i + 1 < args.length) {
            seed = parseInt(args[++i], 10);
        } else if (args[i] === '--output' && i + 1 < args.length) {
            outputPath = args[++i];
        }
    }

    // Initialize weights
    const layerWeights = initAllWeights(seed);

    // Generate input fixtures
    const inputFixtures = generateInputFixtures(seed);

    // Run forward pass on each input fixture
    const results = [];
    for (const fixture of inputFixtures) {
        const input = new Float32Array(fixture.input);
        const expected = forwardPass(input, layerWeights);
        results.push({
            name: fixture.name,
            seed,
            input: fixture.input,
            expected: Array.from(expected),
        });
    }

    // Graph fixture
    const graphFix = generateGraphFixture(seed);
    const graphExpected = enhanceWithGraph(graphFix.center, graphFix.graph, layerWeights);
    results.push({
        name: graphFix.name,
        seed,
        center: graphFix.center,
        graph: graphFix.graph,
        expected: Array.from(graphExpected),
    });

    // Serialize layer weights (just a few values per layer for pre-check)
    const serializedWeights = {};
    for (const layer of LAYERS) {
        const w = layerWeights[layer.id];
        // Store first 10 values of first row + metadata for each layer
        const firstRowSample = Array.from(w.weights[0].slice(0, 10));
        serializedWeights[layer.id] = {
            inDim: w.inDim,
            outDim: w.outDim,
            seed: w.seed,
            first_row_sample: firstRowSample,
            // Full weights for comprehensive parity check
            weights: w.weights.map(row => Array.from(row)),
        };
    }

    const output = {
        generated_at: new Date().toISOString(),
        seed,
        config: CONFIG,
        layer_ids: LAYERS.map(l => l.id),
        layer_weights: serializedWeights,
        fixtures: results,
    };

    const json = JSON.stringify(output, null, 2);

    if (outputPath) {
        fs.writeFileSync(outputPath, json);
        console.log(`Wrote ${results.length} fixtures to ${outputPath}`);
    } else {
        process.stdout.write(json);
    }

    console.error(`Generated ${results.length} fixtures with seed=${seed}`);
}

main();
