//! Exercise the same callback as main, replacing only microphone construction.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_core::config::ArchonConfig;
use archon_tui::app::TuiEvent;
use archon_tui::keybindings::{Action, KeyMap, parse_hotkey};
use archon_tui::voice::pipeline::{
    AudioSource, PUSH_TO_TALK_WINDOW, VoiceTrigger, fire_trigger, fire_trigger_for_hotkey,
};
use tokio::sync::mpsc;

#[path = "main_voice_test_support.rs"]
mod support;

#[derive(Default)]
struct CaptureCounts {
    opened: AtomicUsize,
    started: AtomicUsize,
    stopped: AtomicUsize,
    cancelled: AtomicUsize,
}

struct ControlledAudio {
    counts: Arc<CaptureCounts>,
    levels: mpsc::Sender<f32>,
}

#[async_trait::async_trait]
impl AudioSource for ControlledAudio {
    async fn start(&self) -> anyhow::Result<()> {
        self.counts.started.fetch_add(1, Ordering::SeqCst);
        self.levels.try_send(0.75).unwrap();
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<Vec<f32>> {
        self.counts.stopped.fetch_add(1, Ordering::SeqCst);
        Ok(vec![0.25; 1600])
    }

    async fn cancel(&self) -> anyhow::Result<()> {
        self.counts.cancelled.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn production_voice_toggle_wiring() {
    if !support::isolated("production_voice_toggle_wiring") {
        scenario(true, "ctrl+k", false).await;
    }
}

#[tokio::test(start_paused = true)]
async fn production_voice_push_to_talk_wiring() {
    if !support::isolated("production_voice_push_to_talk_wiring") {
        scenario(false, "ctrl+shift+v", false).await;
    }
}

#[tokio::test(start_paused = true)]
async fn production_voice_unavailable_wiring() {
    if !support::isolated("production_voice_unavailable_wiring") {
        scenario(false, "ctrl+u", true).await;
    }
}

async fn scenario(toggle_mode: bool, hotkey: &str, unavailable: bool) {
    let logs = support::Logs::install();
    let counts = Arc::new(CaptureCounts::default());
    let factory_counts = counts.clone();
    crate::command::tui_helpers::voice_test_factory::install(Box::new(move |device, levels| {
        assert_eq!(device, "controlled-capture");
        factory_counts.opened.fetch_add(1, Ordering::SeqCst);
        let levels = levels.expect("production setup must connect the level meter");
        assert_eq!(levels.capacity(), 8);
        if unavailable {
            anyhow::bail!("controlled capture unavailable");
        }
        Ok(Arc::new(ControlledAudio {
            counts: factory_counts.clone(),
            levels,
        }))
    }));

    // Disabled setup must neither open audio nor install config bindings.
    let mut config = ArchonConfig::default();
    config.voice.enabled = false;
    config.voice.toggle_mode = !toggle_mode;
    config.voice.hotkey = "ctrl+y".into();
    let disabled_key = parse_hotkey(&config.voice.hotkey).unwrap();
    assert_ne!(
        KeyMap::default().resolve(disabled_key),
        Some(&Action::VoiceHotkey)
    );
    assert!(
        crate::command::tui_helpers::setup_voice_pipeline(&config)
            .await
            .is_none()
    );
    super::run_interactive_after_dispatch(
        &config,
        async { Ok(Some("disabled session")) },
        |session, receiver| async move {
            assert_eq!(session, "disabled session");
            assert!(receiver.is_none());
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!(counts.opened.load(Ordering::SeqCst), 0);
    assert_ne!(
        KeyMap::default().resolve(disabled_key),
        Some(&Action::VoiceHotkey)
    );
    let disabled_logs = logs.read();
    assert!(disabled_logs.contains("voice: disabled"));
    assert!(!disabled_logs.contains("voice: pipeline"));
    assert!(!disabled_logs.contains("voice: toggle_mode="));
    assert!(!disabled_logs.contains("voice: unavailable"));

    config.voice.enabled = true;
    config.voice.toggle_mode = toggle_mode;
    config.voice.hotkey = hotkey.into();
    config.voice.device = "controlled-capture".into();
    config.voice.stt_provider = "mock".into();
    config.voice.vad_threshold = 0.2;
    let configured_key = parse_hotkey(hotkey).unwrap();
    assert_ne!(
        KeyMap::default().resolve(configured_key),
        Some(&Action::VoiceHotkey)
    );
    super::run_interactive_after_dispatch(
        &config,
        async {
            assert_eq!(
                counts.opened.load(Ordering::SeqCst),
                0,
                "audio before dispatch"
            );
            Ok(Some("interactive session"))
        },
        |session, receiver| {
            let counts = &counts;
            async move {
                assert_eq!(session, "interactive session");
                if unavailable {
                    assert!(
                        receiver.is_none(),
                        "unavailable capture must not wire a mock fallback"
                    );
                } else {
                    let mut receiver =
                        receiver.expect("production callback lost the voice receiver");
                    exercise_receiver(&mut receiver, counts, toggle_mode).await;
                }
                assert_eq!(counts.opened.load(Ordering::SeqCst), 1);
                assert_eq!(
                    KeyMap::default().resolve(configured_key),
                    Some(&Action::VoiceHotkey)
                );
                Ok(())
            }
        },
    )
    .await
    .unwrap();
    let enabled_logs = logs.read().strip_prefix(&disabled_logs).unwrap().to_owned();
    assert!(enabled_logs.contains(&format!("voice: toggle_mode={toggle_mode}")));
    assert!(!enabled_logs.contains(&format!("voice: toggle_mode={}", !toggle_mode)));
    assert!(!enabled_logs.contains("voice: disabled"));
    if unavailable {
        assert!(enabled_logs.contains("voice: unavailable: controlled capture unavailable"));
        assert!(!enabled_logs.contains("voice: pipeline wired"));
        assert!(!enabled_logs.contains("voice: pipeline started"));
        assert_eq!(counts.started.load(Ordering::SeqCst), 0);
    } else {
        assert!(enabled_logs.contains("voice: pipeline wired"));
        assert!(enabled_logs.contains("voice: pipeline started"));
        assert!(!enabled_logs.contains("voice: unavailable"));
    }
}

async fn settle() {
    // All controlled operations are ready immediately; allow the pipeline and
    // level forwarding tasks to consume their queued work without wall timers.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
}

fn drain(receiver: &mut mpsc::Receiver<TuiEvent>) -> Vec<TuiEvent> {
    let mut events = Vec::new();
    for _ in 0..16 {
        match receiver.try_recv() {
            Ok(event) => events.push(event),
            Err(mpsc::error::TryRecvError::Empty) => return events,
            Err(error) => panic!("production voice receiver disconnected: {error}"),
        }
    }
    panic!("unexpected voice event volume");
}

async fn exercise_receiver(
    receiver: &mut mpsc::Receiver<TuiEvent>,
    counts: &CaptureCounts,
    toggle_mode: bool,
) {
    fire_trigger_for_hotkey();
    settle().await;
    let started = drain(receiver);
    assert_eq!(started.len(), 2);
    assert!(
        started
            .iter()
            .any(|e| matches!(e, TuiEvent::VoiceRecording(true)))
    );
    assert!(
        started
            .iter()
            .any(|e| matches!(e, TuiEvent::VoiceLevel(v) if *v == 0.75))
    );
    assert_eq!(counts.started.load(Ordering::SeqCst), 1);

    tokio::time::advance(PUSH_TO_TALK_WINDOW).await;
    settle().await;
    if toggle_mode {
        assert_eq!(
            counts.stopped.load(Ordering::SeqCst),
            0,
            "toggle auto-stopped"
        );
        assert!(drain(receiver).is_empty());
        fire_trigger_for_hotkey();
        settle().await;
    }
    assert_eq!(
        counts.stopped.load(Ordering::SeqCst),
        1,
        "configured mode did not stop capture"
    );
    let stopped = drain(receiver);
    assert_eq!(stopped.len(), 2);
    assert!(matches!(stopped[0], TuiEvent::VoiceRecording(false)));
    assert!(
        matches!(&stopped[1], TuiEvent::VoiceText(text) if text == "[voice: no STT configured]")
    );

    fire_trigger(VoiceTrigger::Toggle).unwrap();
    settle().await;
    assert_eq!(drain(receiver).len(), 2);
    fire_trigger(VoiceTrigger::Cancel).unwrap();
    settle().await;
    let cancelled = drain(receiver);
    assert_eq!(counts.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(cancelled.len(), 1);
    assert!(matches!(cancelled[0], TuiEvent::VoiceRecording(false)));
    assert_eq!(
        counts.stopped.load(Ordering::SeqCst),
        1,
        "cancel transcribed audio"
    );
}
