use super::*;

fn commands() -> Language {
    Language::Commands {
        build: GateCommand {
            program: "custom-builder".into(),
            args: vec!["--check".into()],
        },
        test: GateCommand {
            program: "custom-runner".into(),
            args: vec!["a".into(), "b".into()],
        },
    }
}

#[test]
fn explicit_commands_preserve_program_and_selectors() {
    let restored: Language =
        serde_json::from_str(&serde_json::to_string(&commands()).unwrap()).unwrap();
    let build = restored.build_command();
    assert_eq!(build.program, "custom-builder");
    assert_eq!(build.args, ["--check"]);
    let test = commands().test_command();
    assert_eq!(test.program, "custom-runner");
    assert_eq!(test.args, ["a", "b"]);
}

#[cfg(unix)]
#[tokio::test]
async fn explicit_gate_commands_use_real_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let language = |code: &str| Language::Commands {
        build: GateCommand {
            program: "sh".into(),
            args: vec!["-c".into(), code.into()],
        },
        test: GateCommand {
            program: "sh".into(),
            args: vec!["-c".into(), code.into()],
        },
    };
    assert!(
        CompilationGate
            .run(root.path(), language("exit 0"))
            .await
            .gate_passed
    );
    assert!(
        !CompilationGate
            .run(root.path(), language("exit 7"))
            .await
            .gate_passed
    );
    assert!(
        TestsRunGate
            .run(root.path(), language("exit 0"))
            .await
            .gate_passed
    );
    assert!(
        !TestsRunGate
            .run(root.path(), language("exit 7"))
            .await
            .gate_passed
    );
}
