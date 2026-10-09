//! Issue 366: the author-step entry validator refuses a check that names a
//! live root by its absolute path, in the freeze probe's own words, so the
//! author repairs it in the same call. Each test drives the real native
//! binding, the cases through the fixed script's author step as well.
use super::shape_tests::script_source;
use super::*;
use crate::command::workflow_task_set::live_root::{live_root_finding, root_forms};
use serde_json::{Value, json};

/// Runs `body` (an async function body returning a JSON string) after the
/// fixed script, with `repository` and `project` as the script's roots.
fn run(repository: &Path, project: &Path, body: &str) -> Value {
    let source = script_source();
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    let roots = json!({"repositoryRoot": repository, "projectRoot": project});
    context.with(|ctx| {
        install_entry_validator(&ctx).unwrap();
        let script = [
            &format!("const args = Object.assign({roots}, {{authorMaxParallelism:1, gateMode:'enforce'}});"),
            &source,
            "(async () => {",
            body,
            "})()",
        ]
        .join("\n");
        let promise: rquickjs::Promise = ctx.eval(script).unwrap();
        let result: String = promise.finish().unwrap();
        serde_json::from_str(&result).unwrap()
    })
}

struct Roots {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    repository: PathBuf,
    project: PathBuf,
}

fn roots() -> Roots {
    let (repository, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    Roots {
        repository: repository.path().to_path_buf(),
        project: project.path().to_path_buf(),
        _dirs: (repository, project),
    }
}

fn command(text: &str) -> Value {
    json!({"id": "A", "criterion": "", "check": {"kind": "command", "command": text, "cwd": "project_root"}})
}

/// The validator's refusals of `entry`, given the script's own roots.
fn refusals(roots: &Roots, entry: &Value) -> Vec<Value> {
    let body = format!(
        "return __archonValidateAcceptanceEntry('A', {}, liveRootsText());",
        serde_json::to_string(&entry.to_string()).unwrap()
    );
    run(&roots.repository, &roots.project, &body)
        .as_array()
        .unwrap()
        .clone()
}

/// The one live-root refusal text for check 'A' naming `root`.
fn expected(root: &Path) -> String {
    format!(
        "acceptance entry 'A' was refused: {}",
        live_root_finding("A", root)
    )
}

fn live_root_texts(found: &[Value]) -> Vec<String> {
    (found.iter())
        .filter(|refusal| refusal["deterministic_defect"]["code"] == "live_root_path")
        .map(|refusal| refusal["text"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_check_naming_the_repository_root_is_refused_in_the_freeze_words() {
    let roots = roots();
    let text = format!("test -f {}/out/result.json", roots.repository.display());
    let found = refusals(&roots, &command(&text));
    assert_eq!(
        live_root_texts(&found),
        [expected(&roots.repository)],
        "{found:?}"
    );
    assert_eq!(found.len(), 1, "only the live root is refused: {found:?}");
}

#[test]
fn a_check_naming_the_project_root_itself_is_refused() {
    let roots = roots();
    let text = format!("ls {}", roots.project.display());
    let found = refusals(&roots, &command(&text));
    assert_eq!(
        live_root_texts(&found),
        [expected(&roots.project)],
        "{found:?}"
    );
}

#[test]
fn a_quoted_live_root_path_is_refused() {
    let roots = roots();
    for text in [
        format!("cat \"{}/a b.txt\"", roots.repository.display()),
        format!("cd '{}' && test -f x", roots.project.display()),
        format!("ls {}/*.json", roots.project.display()),
    ] {
        let found = refusals(&roots, &command(&text));
        assert_eq!(live_root_texts(&found).len(), 1, "{text}: {found:?}");
    }
}

#[test]
fn the_canonical_form_of_a_root_is_refused_too() {
    let roots = roots();
    let canonical = (roots.repository.canonicalize())
        .map(archon_shell::paths::plain)
        .unwrap();
    let text = format!("test -f {}/x", canonical.display());
    let found = refusals(&roots, &command(&text));
    assert_eq!(live_root_texts(&found), [expected(&canonical)], "{found:?}");
}

#[test]
fn a_longer_sibling_name_that_starts_with_a_root_is_not_the_root() {
    let roots = roots();
    for text in [
        format!("test -f {}-other/x", roots.repository.display()),
        format!("test -f {}.bak", roots.project.display()),
        format!("test -f {}_2/x", roots.project.display()),
    ] {
        let found = refusals(&roots, &command(&text));
        assert!(live_root_texts(&found).is_empty(), "{text}: {found:?}");
    }
}

#[test]
fn relative_paths_are_accepted() {
    let roots = roots();
    for text in [
        "test -f out/result.json",
        "cd sub && ./run --input ../data/in.csv",
        "test -f /etc/hosts",
    ] {
        assert_eq!(
            refusals(&roots, &command(text)),
            Vec::<Value>::new(),
            "{text}"
        );
    }
}

#[test]
fn a_floor_verifier_naming_a_live_root_is_refused() {
    let roots = roots();
    let entry = json!({"id": "A", "criterion": "", "check": {"kind": "floor", "contract": {
        "kind": "report", "artifact_path": "out/report.json", "artifact_format": "json",
        "required_true_fields": ["ok"],
        "typed_verifier_command": format!("{}/bin/verify out/report.json", roots.repository.display())}}});
    let found = refusals(&roots, &entry);
    assert_eq!(
        live_root_texts(&found),
        [expected(&roots.repository)],
        "{found:?}"
    );
}

/// A caller that passes no roots gets the shape validation alone.
#[test]
fn without_roots_only_the_shape_is_validated() {
    let roots = roots();
    let entry = command(&format!("test -f {}/x", roots.repository.display()));
    let body = format!(
        "return __archonValidateAcceptanceEntry('A', {});",
        serde_json::to_string(&entry.to_string()).unwrap()
    );
    let found = run(&roots.repository, &roots.project, &body);
    assert_eq!(found, json!([]));
    assert_eq!(root_forms([Path::new("/")]), Vec::<PathBuf>::new());
}

/// The author step itself refuses the entry and keeps it out of the round.
#[test]
fn the_author_step_refuses_a_live_root_check_in_the_same_call() {
    let roots = roots();
    let text = format!("test -f {}/out.json", roots.repository.display());
    let reply = serde_json::to_string(&command(&text).to_string()).unwrap();
    let body = format!(
        "args.acceptanceCriteria = {{A:'a'}};
const w = {{agent: async () => ({{status:'accepted', stopReason:'end_turn', content:{reply}}})}};
const state = {{entries:new Map(), retryIds:null}};
const out = await authorAcceptanceEntries(w, 'author', 1, state);
return JSON.stringify({{out, kept:state.entries.has('A')}});"
    );
    let outcome = run(&roots.repository, &roots.project, &body);
    assert_eq!(outcome["out"]["status"], "failed", "{outcome}");
    assert_eq!(outcome["kept"], false);
    let texts: Vec<&str> = (outcome["out"]["findings"].as_array().unwrap().iter())
        .filter_map(|finding| finding["text"].as_str())
        .collect();
    assert_eq!(texts, [expected(&roots.repository)], "{outcome}");
}

#[test]
fn a_relative_root_is_a_binding_fault_never_a_match() {
    let body = "try { __archonValidateAcceptanceEntry('A', '{}', JSON.stringify(['r'])); return '\"applied\"'; } catch (e) { return JSON.stringify(String(e.message)); }";
    let roots = roots();
    let message = run(&roots.repository, &roots.project, body);
    let message = message.as_str().unwrap();
    assert!(
        message.contains("not absolute"),
        "a relative root is refused, not applied: {message}"
    );
}

/// A git checkout with one commit.
fn committed_repository(dir: &Path) {
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "base",
    ]);
}

#[test]
fn the_author_step_refuses_the_repository_the_freeze_probes_from_the_same_source() {
    // The freeze probes a configured scratch policy's repository; the author
    // step reads the task set's roots from the same source, so a check
    // naming that repository is refused at author time too, even when the
    // script's own repository root is another checkout.
    let roots = roots();
    let policy_repo = tempfile::tempdir().unwrap();
    committed_repository(policy_repo.path());
    let policy_repo = policy_repo.path().canonicalize().unwrap();
    let tasks = roots.project.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let scratch = roots.project.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::create_dir_all(roots.project.join(".archon")).unwrap();
    // The host's own PATH syntax: a toolchain the policy can capture.
    let toolchain = std::env::join_paths([&scratch]).unwrap();
    std::fs::write(
        roots.project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={policy_repo:?}\nscratch_parent={scratch:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path={toolchain:?}\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n"
        ),
    )
    .unwrap();
    let entry = command(&format!("test -f {}/out", policy_repo.display()));
    let body = format!(
        "args.taskRoot = {}; return __archonValidateAcceptanceEntry('A', {}, liveRootsText());",
        serde_json::to_string(&tasks).unwrap(),
        serde_json::to_string(&entry.to_string()).unwrap()
    );
    let found = run(&roots.repository, &roots.project, &body);
    assert_eq!(
        live_root_texts(found.as_array().unwrap()),
        [expected(&policy_repo)]
    );
}

#[test]
fn both_acceptance_authors_are_told_one_check_path_rule() {
    // The host re-author's text is the decomposition author's, byte for byte.
    let roots = roots();
    let script = run(
        &roots.repository,
        &roots.project,
        "return JSON.stringify(checkPathRule());",
    );
    assert_eq!(
        script.as_str().unwrap(),
        crate::command::workflow_task_set::live_root::check_path_rule(
            &roots.repository.display().to_string(),
            &roots.project.display().to_string()
        )
    );
}

/// The binding fault `rootsJson` (JSON text) raises, or "applied".
fn roots_fault(roots: &Roots, roots_json: &str) -> String {
    let entry = command("true").to_string();
    let body = format!(
        "try {{ __archonValidateAcceptanceEntry('A', {}, {}); return '\"applied\"'; }} catch (e) {{ return JSON.stringify(String(e.message)); }}",
        serde_json::to_string(&entry).unwrap(),
        serde_json::to_string(roots_json).unwrap()
    );
    let message = run(&roots.repository, &roots.project, &body);
    message.as_str().unwrap().to_string()
}

/// Issue 366 N2: task-set roots without the repository or the project would
/// turn the rule off without a signal; they are a binding fault.
#[test]
fn task_set_roots_without_the_repository_or_the_project_are_a_binding_fault() {
    let roots = roots();
    let (repository, project) = (&roots.repository, &roots.project);
    // Built as JSON: a Windows path's `\` is escaped, as any caller's is.
    for roots_json in [
        json!({}),
        json!({"tasks": project.join("tasks")}),
        json!({"repository": repository}),
        json!({"repository": "", "project": project}),
        json!({"repository": repository, "project": "  "}),
        json!({"repository": repository, "project": project, "tasks": ""}),
    ] {
        let message = roots_fault(&roots, &roots_json.to_string());
        assert!(
            message.contains("must name both the repository and the project"),
            "{roots_json}: refused, never shape-only: {message}"
        );
    }
    let complete = json!({"repository": repository, "project": project});
    assert_eq!(roots_fault(&roots, &complete.to_string()), "applied");
}

/// Issue 366 N1: the author step reads the task set's roots from the same
/// source as the freeze probe; a lock it cannot believe is a binding fault
/// naming the lock, never the project taken as the repository.
#[test]
fn an_unbelievable_repository_lock_is_a_binding_fault_naming_it() {
    for (case, lock) in crate::command::workflow_task_set::live_root::tests::UNBELIEVABLE_LOCKS {
        let roots = roots();
        let tasks = roots.project.join("tasks");
        std::fs::create_dir_all(&tasks).unwrap();
        let path = archon_workflow::repository_record::repository_record_path(&tasks);
        std::fs::write(&path, lock).unwrap();
        let roots_json =
            json!({"repository": roots.repository, "project": roots.project, "tasks": tasks});
        let message = roots_fault(&roots, &roots_json.to_string());
        assert!(
            message.contains(&path.display().to_string()) && message.contains("restore"),
            "{case}: {message}"
        );
    }
}
