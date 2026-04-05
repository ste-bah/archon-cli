use archon_core::config::VoiceConfig;
use archon_tui::voice::capture::{AudioCapture, VoiceActivityDetector};
use archon_tui::voice::hotkey::HotkeyState;
use archon_tui::voice::{VoiceManager, VoiceState};

#[test]
fn vad_silence_below_threshold() {
    assert!(!VoiceActivityDetector::new(0.1).is_speech(&[0.0; 100]));
}

#[test]
fn vad_speech_above_threshold() {
    assert!(VoiceActivityDetector::new(0.01).is_speech(&[0.5; 100]));
}

#[test]
fn vad_threshold_boundary() {
    let vad = VoiceActivityDetector::new(0.5);
    // RMS of [0.5; 100] = 0.5, NOT > 0.5
    assert!(!vad.is_speech(&[0.5; 100]));
}

#[test]
fn wav_encode_produces_valid_header() {
    let cap = AudioCapture::new();
    let bytes = cap.encode_to_wav(&[0.0_f32; 16000]);
    assert!(bytes.starts_with(b"RIFF"));
}

#[test]
fn wav_encode_correct_length() {
    let cap = AudioCapture::new();
    let bytes = cap.encode_to_wav(&[0.0_f32; 16000]);
    assert!(bytes.len() > 44);
}

#[test]
fn wav_encode_empty_input() {
    let cap = AudioCapture::new();
    let bytes = cap.encode_to_wav(&[]);
    assert!(bytes.starts_with(b"RIFF"));
    assert!(bytes.len() >= 44);
}

#[test]
fn voice_state_transitions() {
    let mut mgr = VoiceManager::new(VoiceConfig {
        enabled: true,
        ..Default::default()
    });
    mgr.set_state(VoiceState::Listening);
    assert!(matches!(mgr.state(), VoiceState::Listening));
    mgr.set_state(VoiceState::Transcribing);
    assert!(matches!(mgr.state(), VoiceState::Transcribing));
    mgr.set_state(VoiceState::Idle);
    assert!(matches!(mgr.state(), VoiceState::Idle));
}

#[test]
fn voice_manager_disabled_by_default() {
    assert!(!VoiceManager::new(VoiceConfig::default()).is_enabled());
}

#[test]
fn voice_manager_inject_text() {
    let mut mgr = VoiceManager::new(VoiceConfig::default());
    assert!(mgr.inject_text("hello world").is_ok());
}

#[test]
fn hotkey_push_to_talk() {
    let mut h = HotkeyState::new();
    assert!(h.press());
    assert!(!h.release());
}

#[test]
fn hotkey_toggle() {
    let mut h = HotkeyState::new();
    assert!(h.toggle());
    assert!(!h.toggle());
}

#[tokio::test]
async fn mock_stt_returns_response() {
    use archon_tui::voice::stt::{MockStt, SttProvider};
    let stt = MockStt {
        response: "hello world".into(),
    };
    let result = stt.transcribe(&[]).await.unwrap();
    assert_eq!(result, "hello world");
}
