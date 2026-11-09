//! Author-visible policy proofs for #631.
#[path = "../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r3_p2_python_body_prompt_uses_neutral_root_terms() {
    let file = fixture::root().join("src/command/workflow_decompose_v1_body.js");
    let script = r#"const vm=require('node:vm'),fs=require('node:fs');const code=fs.readFileSync(process.argv[1],'utf8');console.log(vm.runInNewContext(code+';bodyPolicy({taskId: \"TASK-PY-001\",fileName: \"TASK-PY-001.md\"},[]).prompt()',{args:{prdPath:'PRD.md',taskRoot:'tasks',projectRoot:'.'},groundingRules:()=>'',observationRule:()=>'',constraintContext:()=>[],sectionContext:()=>[],BODY_SHAPE:''}));"#;
    let out = fixture::run(
        &fixture::fixture("python"),
        "node",
        &["-e", script, file.to_str().unwrap()],
    )
    .expect("required fixture toolchain must be available");
    assert!(
        out.status.success(),
        "body-policy fixture must render: {}",
        fixture::text(&out)
    );
    assert!(
        !fixture::text(&out).contains("never to a crate or module directory"),
        "Python body author receives Rust-only root terminology"
    );
}
