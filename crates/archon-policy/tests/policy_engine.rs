use archon_policy::{EffectivePolicy, PolicySource, load_policy_from_sources};

fn write_policy(dir: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn parses_user_facing_policy_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_policy(
        &dir,
        "policy.toml",
        r#"
[policy.gametheory]
max_agents_per_council = 8
enable_tier11 = true

[policy.learning]
auto_apply_low_risk = true

[policy.world_model]
allow_third_party_embeddings = true
allow_llm_labeler = true
allow_behavior_changes = true

[policy.web]
allow_mutating_actions = true
allow_file_uploads = true
allow_pipeline_controls = true
allow_model_training_actions = true
allow_corpus_open_paths = true

[policy.docs.vlm]
enabled = true
mode = "local"
provider = "ollama"

[policy.docs.vlm.ollama]
endpoint = "http://127.0.0.1:11434"
model = "gemma4:e4b"
timeout_secs = 90

[policy.docs.vlm.gemini]
api_key_env = "GOOGLE_API_KEY"
model = "gemini-3-flash-preview"
endpoint_base = "https://generativelanguage.googleapis.com/v1beta"
rpm_limit = 12

[policy.docs.vlm.anthropic]
model = "claude-sonnet-4-6"

[policy.docs.vlm.openai_compat]
endpoint = "http://localhost:1234/v1"
model = "llava:13b"
api_key_env = "LMSTUDIO_API_KEY"
timeout_secs = 60
max_tokens = 768
temperature = 0.1

[policy.docs.pdf]
extract_embedded_images = true
min_image_dimension = 256
min_image_bytes = 8192
vlm_per_page_image = true
render_text_pdf_pages = false

[policy.docs.retrieval]
exact_weight = 0.7
semantic_weight = 0.3
"#,
    );
    let load = load_policy_from_sources(&[PolicySource {
        label: "workspace",
        path,
    }])
    .unwrap();
    assert!(load.policy.gametheory.enable_tier11);
    assert_eq!(load.policy.gametheory.max_agents_per_council, 8);
    assert!(load.policy.learning.auto_apply_low_risk);
    assert!(load.policy.world_model.allow_third_party_embeddings);
    assert!(load.policy.world_model.allow_llm_labeler);
    assert!(load.policy.world_model.allow_behavior_changes);
    assert!(load.policy.web.allow_mutating_actions);
    assert!(load.policy.web.allow_file_uploads);
    assert!(load.policy.web.allow_pipeline_controls);
    assert!(load.policy.web.allow_model_training_actions);
    assert!(load.policy.web.allow_corpus_open_paths);
    assert_eq!(load.policy.docs.vlm.mode, "local");
    assert_eq!(load.policy.docs.vlm.provider, "ollama");
    assert_eq!(
        load.policy.docs.vlm.ollama.endpoint,
        "http://127.0.0.1:11434"
    );
    assert_eq!(load.policy.docs.vlm.ollama.model, "gemma4:e4b");
    assert_eq!(load.policy.docs.vlm.ollama.timeout_secs, 90);
    assert_eq!(load.policy.docs.vlm.gemini.model, "gemini-3-flash-preview");
    assert_eq!(load.policy.docs.vlm.gemini.rpm_limit, 12);
    assert_eq!(load.policy.docs.vlm.anthropic.model, "claude-sonnet-4-6");
    assert_eq!(
        load.policy.docs.vlm.openai_compat.endpoint,
        "http://localhost:1234/v1"
    );
    assert_eq!(load.policy.docs.vlm.openai_compat.model, "llava:13b");
    assert_eq!(
        load.policy.docs.vlm.openai_compat.api_key_env,
        "LMSTUDIO_API_KEY"
    );
    assert_eq!(load.policy.docs.vlm.openai_compat.timeout_secs, 60);
    assert_eq!(load.policy.docs.vlm.openai_compat.max_tokens, 768);
    assert!((load.policy.docs.vlm.openai_compat.temperature - 0.1).abs() < f32::EPSILON);
    assert!(load.policy.docs.pdf.extract_embedded_images);
    assert_eq!(load.policy.docs.pdf.min_image_dimension, 256);
    assert_eq!(load.policy.docs.pdf.min_image_bytes, 8192);
    assert!(load.policy.docs.pdf.vlm_per_page_image);
    assert!(!load.policy.docs.pdf.render_text_pdf_pages);
    assert_eq!(load.policy.docs.retrieval.exact_weight, 0.7);
    assert_eq!(load.policy.docs.retrieval.semantic_weight, 0.3);
}

#[test]
fn missing_policy_defaults_to_deny() {
    let dir = tempfile::tempdir().unwrap();
    let load = load_policy_from_sources(&[PolicySource {
        label: "workspace",
        path: dir.path().join("missing.toml"),
    }])
    .unwrap();
    assert!(load.loaded_sources.is_empty());
    assert!(!load.policy.gametheory_tier11_decision().allowed);
    assert!(!load.policy.docs_vlm_decision().allowed);
    assert!(
        !load
            .policy
            .world_model_third_party_embeddings_decision()
            .allowed
    );
    assert!(!load.policy.world_model_llm_labeler_decision().allowed);
    assert!(!load.policy.world_model_behavior_change_decision().allowed);
    assert!(!load.policy.web.allow_mutating_actions);
    assert!(
        !load
            .policy
            .learning_auto_apply_decision("RetrievalProfile", "Low")
            .allowed
    );
}

#[test]
fn workspace_overrides_user_and_system_policy() {
    let dir = tempfile::tempdir().unwrap();
    let system = write_policy(
        &dir,
        "system.toml",
        "[policy.gametheory]\nenable_tier11 = false\n",
    );
    let user = write_policy(
        &dir,
        "user.toml",
        "[policy.gametheory]\nenable_tier11 = false\nmax_cost_usd = 10.0\n",
    );
    let workspace = write_policy(
        &dir,
        "workspace.toml",
        "[policy.gametheory]\nenable_tier11 = true\n",
    );
    let load = load_policy_from_sources(&[
        PolicySource {
            label: "system",
            path: system,
        },
        PolicySource {
            label: "user",
            path: user,
        },
        PolicySource {
            label: "workspace",
            path: workspace,
        },
    ])
    .unwrap();
    assert!(load.policy.gametheory.enable_tier11);
    assert_eq!(load.policy.gametheory.max_cost_usd, 10.0);
}

#[test]
fn local_vlm_requires_docs_enabled_and_worker_allow() {
    let policy = EffectivePolicy {
        docs: archon_policy::DocsPolicy {
            vlm: archon_policy::VlmPolicy {
                enabled: true,
                mode: "local".into(),
                provider: "ollama".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!policy.docs_vlm_decision().allowed);
    let mut allowed = policy.clone();
    allowed.workers.vlm = "allow-local".into();
    assert!(allowed.docs_vlm_decision().allowed);
}

#[test]
fn cloud_vlm_requires_dual_cloud_policy() {
    let mut policy = EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "cloud".into();
    policy.docs.vlm.provider = "gemini".into();
    policy.docs.vlm.allow_cloud = true;
    assert!(!policy.docs_vlm_decision().allowed);
    policy.network.allow_cloud_vlm = true;
    assert!(!policy.docs_vlm_decision().allowed);
    policy.workers.vlm = "allow-cloud".into();
    assert!(policy.docs_vlm_decision().allowed);
}

#[test]
fn openai_compat_local_mode_uses_local_worker_gate() {
    let mut policy = EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "local".into();
    policy.docs.vlm.provider = "openai-compat".into();
    policy.workers.vlm = "allow-local".into();

    assert!(policy.docs_vlm_decision().allowed);
}

#[test]
fn openai_compat_cloud_mode_requires_cloud_gate() {
    let mut policy = EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "cloud".into();
    policy.docs.vlm.provider = "openai-compat".into();
    policy.docs.vlm.openai_compat.endpoint = "https://api.openai.com/v1".into();
    policy.docs.vlm.allow_cloud = true;
    policy.network.allow_cloud_vlm = true;
    assert!(!policy.docs_vlm_decision().allowed);

    policy.workers.vlm = "allow-cloud".into();
    assert!(policy.docs_vlm_decision().allowed);
}

#[test]
fn cloud_provider_denied_when_mode_is_local() {
    let mut policy = EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "local".into();
    policy.docs.vlm.provider = "anthropic".into();
    policy.docs.vlm.allow_cloud = true;
    policy.network.allow_cloud_vlm = true;

    let decision = policy.docs_vlm_decision();
    assert!(!decision.allowed);
    assert!(decision.reason.contains("cloud VLM provider requires"));
}

#[test]
fn provider_disabled_overrides_enabled_mode() {
    let mut policy = EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "local".into();
    policy.docs.vlm.provider = "disabled".into();
    policy.workers.vlm = "allow-local".into();

    assert!(!policy.docs_vlm_decision().allowed);
}

#[test]
fn tier11_gate_tracks_gametheory_policy() {
    let mut policy = EffectivePolicy::default();
    assert!(!policy.gametheory_tier11_decision().allowed);
    policy.gametheory.enable_tier11 = true;
    assert!(policy.gametheory_tier11_decision().allowed);
}

#[test]
fn high_impact_learning_changes_remain_approval_gated() {
    let mut policy = EffectivePolicy::default();
    policy.learning.auto_apply_low_risk = true;
    assert!(
        policy
            .learning_auto_apply_decision("RetrievalProfile", "Low")
            .allowed
    );
    assert!(
        !policy
            .learning_auto_apply_decision("PromptProfile", "Low")
            .allowed
    );
    assert!(
        !policy
            .learning_auto_apply_decision("RetrievalProfile", "High")
            .allowed
    );
}

#[test]
fn world_model_cloud_embeddings_require_policy_worker_and_network() {
    let mut policy = EffectivePolicy::default();
    policy.world_model.allow_third_party_embeddings = true;
    assert!(!policy.world_model_third_party_embeddings_decision().allowed);

    policy.workers.embedding = "allow-cloud".into();
    assert!(!policy.world_model_third_party_embeddings_decision().allowed);

    policy.network.default = "allow".into();
    assert!(policy.world_model_third_party_embeddings_decision().allowed);
}

#[test]
fn repository_policy_template_parses_all_vlm_provider_fields() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".archon")
        .join("policy.toml");
    let load = load_policy_from_sources(&[PolicySource {
        label: "workspace",
        path: path.clone(),
    }])
    .unwrap_or_else(|e| panic!("load {}: {e}", path.display()));

    // User-customizable fields: assert validity/presence, not exact value, so
    // local overrides (e.g. pointing openai_compat at a self-hosted LM Studio
    // instance) don't break the template-parses test. The exact template
    // values still ship in .archon/policy.toml; CI checks them implicitly by
    // running this test against the committed file.
    assert!(
        matches!(
            load.policy.docs.vlm.provider.as_str(),
            "disabled" | "ollama" | "gemini" | "anthropic" | "openai-compat"
        ),
        "vlm.provider must be a valid enum value, got {:?}",
        load.policy.docs.vlm.provider
    );
    assert_eq!(load.policy.docs.vlm.ollama.model, "gemma4:e4b");
    assert_eq!(load.policy.docs.vlm.ollama.timeout_secs, 120);
    assert_eq!(load.policy.docs.vlm.gemini.api_key_env, "GOOGLE_API_KEY");
    assert_eq!(load.policy.docs.vlm.gemini.model, "gemini-3-flash-preview");
    assert_eq!(load.policy.docs.vlm.gemini.rpm_limit, 12);
    assert_eq!(load.policy.docs.vlm.anthropic.model, "claude-sonnet-4-6");
    assert!(
        load.policy
            .docs
            .vlm
            .openai_compat
            .endpoint
            .starts_with("http"),
        "openai_compat.endpoint must be an http(s) URL, got {:?}",
        load.policy.docs.vlm.openai_compat.endpoint
    );
    assert!(
        !load.policy.docs.vlm.openai_compat.model.is_empty(),
        "openai_compat.model must be set"
    );
    assert_eq!(
        load.policy.docs.vlm.openai_compat.api_key_env,
        "OPENAI_API_KEY"
    );
    // codex's own policy.toml comment says "comfortably above 1024";
    // pinning to a single value (== 8192) breaks any user who tunes for a
    // larger model. Strict `> 1024` still catches the regression codex was
    // guarding against (the template stuck at the legacy 1024 default).
    assert!(
        load.policy.docs.vlm.openai_compat.max_tokens > 1024,
        "openai_compat.max_tokens must be > 1024 (template comment), got {}",
        load.policy.docs.vlm.openai_compat.max_tokens
    );
    assert!(load.policy.docs.pdf.extract_embedded_images);
    assert_eq!(load.policy.docs.pdf.min_image_dimension, 200);
    assert_eq!(load.policy.docs.pdf.min_image_bytes, 4096);
    assert!(load.policy.docs.pdf.vlm_per_page_image);
    assert!(!load.policy.docs.pdf.render_text_pdf_pages);
    assert_eq!(load.policy.docs.retrieval.exact_weight, 0.45);
    assert_eq!(load.policy.docs.retrieval.semantic_weight, 0.55);
    assert!(!load.policy.web.allow_mutating_actions);
    assert!(!load.policy.web.allow_file_uploads);
    assert!(!load.policy.web.allow_pipeline_controls);
    assert!(!load.policy.web.allow_model_training_actions);
    assert!(!load.policy.web.allow_corpus_open_paths);
}

#[test]
fn pdf_policy_defaults_to_embedded_extraction_without_page_rendering() {
    let policy = EffectivePolicy::default();

    assert!(policy.docs.pdf.extract_embedded_images);
    assert_eq!(policy.docs.pdf.min_image_dimension, 200);
    assert_eq!(policy.docs.pdf.min_image_bytes, 4096);
    assert!(policy.docs.pdf.vlm_per_page_image);
    assert!(!policy.docs.pdf.render_text_pdf_pages);
}
