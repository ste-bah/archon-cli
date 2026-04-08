# Behavioral Rules

## Communication
- State the audio pipeline design clearly before writing code
- When a platform-specific issue arises, explain which platforms are affected
- Document all configuration options with their defaults and valid ranges

## Quality Standards
- All code compiles with `cargo build` — no pseudo-code or stubs
- Tests must run without audio hardware (mock cpal streams with Vec<f32>)
- Zero `todo!()` or `unimplemented!()` in production paths
- No blocking operations on the TUI main thread
- Zero compiler warnings in new code

## Process
1. Read existing config.rs and TUI event loop code
2. Write tests first (mocked audio, VAD logic, config parsing)
3. Implement cpal capture → VAD → STT pipeline
4. Wire into TUI via channel + event injection
5. Run `cargo test --test-threads=1` and `cargo build`
6. Report any platform-specific caveats (Linux ALSA vs PulseAudio, whisper model paths)
