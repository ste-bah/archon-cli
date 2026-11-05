//! The scanner receives both authored shell and policy's quoted path-only form.
use super::*;

#[test]
#[allow(non_snake_case)]
fn R4_D8_cd_forms_by_layout_preserve_expected_sibling_sets() {
    // Input shell/path-only form -> expected (path, required) set. Layouts
    // must not change this lexical result or the discovered external alias.
    let forms: &[(&str, &[&str])] = &[
        ("test -f ../other/data", &["../other/data"]),
        ("'../other/data'", &["../other/data"]),
        ("../other/data", &["../other/data"]),
        (
            "cd .. && cd other && test -f data",
            &["../other", "../other/data"],
        ),
        ("cd -P D; test -f ../../other/data", &["../other/data"]),
        ("cd -L D; test -f ../../other/data", &["../other/data"]),
        ("cd D | cat; test -f ../other/data", &["../other/data"]),
        ("cd D & test -f ../other/data", &["../other/data"]),
        ("(cd D); test -f ../other/data", &["../other/data"]),
        ("cd D; cd -; test -f ../other/data", &["../other/data"]),
        ("pushd D; popd; test -f ../other/data", &["../other/data"]),
        (
            "cd D\npython3 - <<'END'\nopen('../../other/data')\nEND\ncd E",
            &["../other/data"],
        ),
        (
            "cd ../other && test -f src/new.rs",
            &["../other", "../other/src/new.rs"],
        ),
        ("test -f ../other/src/new.rs", &["../other/src/new.rs"]),
        ("cd ../absent && test -f x", &["../absent", "../absent/x"]),
        ("test -f ../absent/x", &["../absent/x"]),
        (
            "cd ../other && cd sub && test -f y",
            &["../other", "../other/sub", "../other/sub/y"],
        ),
    ];
    for (layout, project, repository) in [
        ("neutral", "workspace", "repository"),
        ("same_path", "shared", "shared"),
        ("site_names", "project", "repo"),
        (
            "repository_inside_project",
            "workspace",
            "workspace/repository",
        ),
        (
            "project_inside_repository",
            "repository/workspace",
            "repository",
        ),
    ] {
        let tree = tempfile::tempdir().unwrap();
        let project = tree.path().join(project);
        let repository = tree.path().join(repository);
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        for project in [&project, &repository] {
            std::fs::create_dir_all(project.join("D")).unwrap();
            let other = project.parent().unwrap().join("other");
            std::fs::create_dir_all(&other).unwrap();
            std::fs::write(other.join("data"), "input").unwrap();
            for (command, expected) in forms {
                assert_eq!(
                    missing_root(project, command).is_some(),
                    command.contains("../absent"),
                    "B1 root availability: {layout}: {command}"
                );
                let actual = scan(command);
                let expected: Vec<_> = expected
                    .iter()
                    .map(|path| SiblingPath {
                        path: PathBuf::from(path),
                        required: !matches!(*command, "'../other/data'" | "../other/data"),
                    })
                    .collect();
                assert_eq!(actual, expected, "{layout}: {command}");
                // Every copying/site-path/observation/reuse caller receives this
                // same alias set, including policy's path-only adapter.
                let aliases = discover(&project, &[command.to_string()]);
                assert_eq!(
                    aliases,
                    if command.contains("../absent") {
                        vec![]
                    } else {
                        vec![SiblingSource {
                            alias: archon_shell::paths::canonicalize(&other).unwrap(),
                            source: archon_shell::paths::canonicalize(&other).unwrap(),
                        }]
                    },
                    "{layout}: {command}"
                );
            }
            // Direct parent files are copied too; re-entering the selected root
            // is excluded and remains available to the refusal rule to explain.
            std::fs::write(project.parent().unwrap().join("loose.txt"), "input").unwrap();
            assert_eq!(
                discover(&project, &["'../loose.txt'".into()]).len(),
                1,
                "{layout}"
            );
            let own = format!("'../{}/D'", project.file_name().unwrap().to_string_lossy());
            assert!(discover(project, &[own]).is_empty(), "{layout}");
        }
    }
}
