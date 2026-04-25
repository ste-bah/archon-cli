use archon_tui::screens::voice_capture::VoiceCaptureOverlay;

#[test]
fn test_new_not_recording() {
    let v = VoiceCaptureOverlay::new();
    assert!(!v.is_recording());
    assert!(v.transcription().is_empty());
}

#[test]
fn test_start_stop() {
    let mut v = VoiceCaptureOverlay::new();
    v.start();
    assert!(v.is_recording());
    v.stop();
    assert!(!v.is_recording());
}

#[test]
fn test_push_sample_truncates_at_200() {
    let mut v = VoiceCaptureOverlay::new();
    for i in 0..250 {
        v.push_sample((i as f32) * 0.01);
    }
    assert!(v.waveform_slice().len() <= 200);
}

#[test]
fn test_clear_resets() {
    let mut v = VoiceCaptureOverlay::new();
    v.start();
    v.set_transcription("hello world");
    v.clear();
    assert!(!v.is_recording());
    assert!(v.transcription().is_empty());
    assert!(v.waveform_slice().is_empty());
}

#[test]
fn test_set_transcription() {
    let mut v = VoiceCaptureOverlay::new();
    v.set_transcription("test transcription");
    assert_eq!(v.transcription(), "test transcription");
}
