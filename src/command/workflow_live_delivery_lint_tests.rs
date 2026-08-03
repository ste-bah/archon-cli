//! Nothing on a run's own path may discard a status it failed to deliver.
//!
//! Split out of `workflow_live_tests` so that file holds the 500-line ceiling,
//! and because it is a different kind of test from its neighbours: a source
//! scan over the production paths rather than an assertion about one function.

#[test]
fn workflow_and_session_paths_do_not_ignore_tui_delivery() {
    fn collect(path: &std::path::Path, offenders: &mut Vec<std::path::PathBuf>) {
        if path.is_file() {
            inspect_source(path, offenders);
            return;
        }
        for entry in std::fs::read_dir(path).expect("read source directory") {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                collect(&path, offenders);
            } else {
                inspect_source(&path, offenders);
            }
        }
    }

    fn inspect_source(path: &std::path::Path, offenders: &mut Vec<std::path::PathBuf>) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") || name.contains("test") {
            return;
        }
        let compact: String = std::fs::read_to_string(path)
            .expect("read source")
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        // Two vocabularies, because two layers now emit. `session*` still
        // holds a `TuiEventSender` directly; workflow execution reaches the
        // same channel through `archon_workflow::ui_sink_port`. Dropping a
        // status is the same bug either way, so both spellings are offences.
        let ignores_tui_delivery = compact.split(';').any(|statement| {
            statement.contains("let_=")
                && (statement.contains(".send(TuiEvent")
                    || statement.contains(".send(archon_tui::app::TuiEvent")
                    || statement.contains(".emit(WorkflowUiEvent")
                    || statement.contains(".emit(archon_workflow::WorkflowUiEvent"))
        });
        if ignores_tui_delivery {
            offenders.push(path.to_path_buf());
        }
    }

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    collect(&root.join("session"), &mut offenders);
    collect(&root.join("session_loop"), &mut offenders);
    for entry in std::fs::read_dir(root.join("command")).expect("read command directory") {
        let path = entry.expect("read command entry").path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("workflow_live") && !name.contains("test"))
        {
            collect(&path, &mut offenders);
        }
    }

    assert!(
        offenders.is_empty(),
        "production paths ignore bounded TUI delivery: {offenders:?}"
    );
}
