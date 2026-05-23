use archon_policy::{EffectivePolicy, VideoPolicy};

#[test]
fn default_video_policy_denies_ingest() {
    let policy = EffectivePolicy::default();

    assert!(!VideoPolicy::default().enabled);
    assert!(!policy.video_ingest_decision().allowed);
}

#[test]
fn local_asr_is_allowed_without_network_when_video_enabled() {
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;
    policy.video.asr.provider = "whisper-rs".into();

    let decision = policy.video_asr_decision();

    assert!(decision.allowed, "{decision:?}");
}

#[test]
fn youtube_sources_require_youtube_policy_gate() {
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;

    assert!(!policy.video_acquisition_decision("YouTube").allowed);

    policy.video.allow_youtube = true;
    assert!(policy.video_acquisition_decision("YouTube").allowed);
}

#[test]
fn external_downloaders_require_explicit_policy_gate() {
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;

    assert!(
        !policy
            .video_acquisition_decision("ExternalDownloader")
            .allowed
    );

    policy.video.allow_external_downloaders = true;
    assert!(
        policy
            .video_acquisition_decision("ExternalDownloader")
            .allowed
    );
}

#[test]
fn browser_automation_requires_explicit_policy_gate() {
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;

    assert!(
        !policy
            .video_acquisition_decision("BrowserAutomation")
            .allowed
    );

    policy.video.allow_browser_automation = true;
    assert!(
        policy
            .video_acquisition_decision("BrowserAutomation")
            .allowed
    );
}
