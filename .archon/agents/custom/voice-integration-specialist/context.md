# Domain Context: Voice Integration in Rust

## Background
Integrating voice input into a Rust TUI requires solving three orthogonal problems:
1. **Audio capture**: platform-specific microphone access via cpal
2. **VAD**: distinguishing speech from silence/noise in real-time
3. **STT**: converting audio bytes to text, either locally (whisper-rs) or via API

The TUI (ratatui + crossterm) expects keyboard events. Voice input is injected by simulating
keypresses or writing directly to the input buffer state.

## Key Concepts
- **cpal**: Cross-platform audio I/O. `cpal::default_input_device()`, `device.build_input_stream()`
- **Sample format**: Whisper needs f32 mono 16kHz. cpal may give i16 stereo at 44100Hz — convert.
- **VAD**: Energy threshold: `rms = sqrt(samples.iter().map(|s| s*s).sum::<f32>() / n as f32)`
- **whisper-rs**: Rust bindings for whisper.cpp. `WhisperContext::new(model_path)`, `state.full(params, &audio)`
- **Ring buffer**: Fixed-size circular buffer; drop oldest samples when full during continuous mode
- **Push-to-talk**: Hold key → start recording; release key → stop → transcribe → inject
- **Continuous**: VAD detects speech start/end; auto-transcribes each utterance

## Common Patterns

```rust
// Non-blocking capture via channel
let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
let stream = device.build_input_stream(&config, move |data: &[f32], _| {
    let _ = tx.send(data.to_vec());
}, err_fn, None)?;

// Sample rate conversion (44100 → 16000) — linear resampling
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let idx = src as usize;
            let frac = src - idx as f64;
            let a = *samples.get(idx).unwrap_or(&0.0);
            let b = *samples.get(idx + 1).unwrap_or(&0.0);
            a + (b - a) * frac as f32
        })
        .collect()
}

// Stereo to mono
fn stereo_to_mono(samples: &[f32]) -> Vec<f32> {
    samples.chunks(2).map(|c| (c[0] + c.get(1).copied().unwrap_or(0.0)) * 0.5).collect()
}
```

## Cargo Dependencies (check workspace before adding)
- `cpal` — audio capture
- `whisper-rs` — local STT (optional feature flag)
- `hound` — WAV file I/O for testing
- `tokio::sync::mpsc` — async channel for TUI integration
