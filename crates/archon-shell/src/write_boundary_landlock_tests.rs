//! Issue-227: the Landlock boundary's plan, everywhere, and its enforcement
//! on a Linux child: no write outside the roots (absolute, `..`, symlink,
//! hard link, rename, truncate), every write inside them.
use super::*;

struct Layout {
    _base: tempfile::TempDir,
    base: PathBuf,
    project: PathBuf,
    checkout: PathBuf,
    worktree: PathBuf,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    data: PathBuf,
    beside: PathBuf,
}

fn layout() -> Layout {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let project = base.join("project");
    let checkout = base.join("checkout");
    let worktree = project.join(".archon/workflows/run/v2/worktrees/a/a-0");
    let data = project.join("data/registry.json");
    let beside = base.join("beside");
    for dir in [
        &worktree,
        &checkout,
        &beside,
        &data.parent().unwrap().to_path_buf(),
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&data, "host").unwrap();
    std::fs::write(checkout.join("lib.rs"), "base").unwrap();
    Layout {
        _base: dir,
        base,
        project,
        checkout,
        worktree,
        data,
        beside,
    }
}

fn sealed(layout: &Layout) -> Vec<PathBuf> {
    vec![layout.project.clone(), layout.checkout.clone()]
}

#[test]
fn the_plan_grants_the_complement_and_never_a_sealed_root() {
    let layout = layout();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&layout.project, layout.base.join("link")).unwrap();
    let writable = vec![layout.worktree.clone(), layout.base.clone()];
    let grants = plan(&sealed(&layout), &writable);
    assert!(grants.contains(&layout.beside), "{grants:?}");
    assert!(grants.contains(&layout.worktree), "{grants:?}");
    for grant in &grants {
        let opens_a_root = sealed(&layout).iter().any(|root| root.starts_with(grant));
        assert!(!opens_a_root, "{} re-opens a sealed root", grant.display());
        if grant.starts_with(&layout.project) {
            assert_eq!(grant, &layout.worktree, "only the re-opened entry");
        }
    }
    // The base contains the roots (dropped), the link points into one.
    assert!(!grants.contains(&layout.base));
    assert!(!grants.contains(&layout.base.join("link")));
    assert!(
        !grants
            .iter()
            .any(|grant| grant.starts_with(&layout.checkout))
    );
}

#[test]
fn a_writable_entry_resolves_before_it_is_granted() {
    let layout = layout();
    #[cfg(unix)]
    {
        // A writable entry that is a link to a sealed root's ancestor.
        let link = layout.beside.join("up");
        std::os::unix::fs::symlink(&layout.base, &link).unwrap();
        let grants = plan(&sealed(&layout), std::slice::from_ref(&link));
        assert!(!grants.contains(&link) && !grants.contains(&layout.base));
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn landlock_is_refused_off_linux() {
    let error = LandlockSandbox::build(&[], &[], &[]).unwrap_err();
    assert!(error.contains("Linux-only"), "{error}");
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    /// The CI runner's kernel must have it: a skip there would prove nothing.
    #[test]
    fn landlock_is_available_on_ci() {
        let probed = probe();
        eprintln!("landlock probe: {probed:?}");
        if std::env::var_os("CI").is_some() {
            assert!(probed.is_ok(), "{probed:?}");
        }
    }

    fn run(sandbox: &LandlockSandbox, cwd: &Path, script: &str) -> std::process::Output {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", script]).current_dir(cwd);
        if let Some(temp) = sandbox.private_temp() {
            command.env("TMPDIR", temp);
        }
        sandbox.install_std(&mut command);
        command.output().unwrap()
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn a_landlocked_child_cannot_write_outside_its_roots() {
        if probe().is_err() {
            eprintln!("skipped: no usable Landlock here");
            return;
        }
        let layout = layout();
        let temp = [layout.base.parent().unwrap().to_path_buf()];
        let sandbox =
            LandlockSandbox::build(&sealed(&layout), &[layout.worktree.clone()], &temp).unwrap();
        let data = layout.data.display().to_string();
        let lib = layout.checkout.join("lib.rs").display().to_string();
        for script in [
            format!("printf x > {data}"),
            "printf x > ../../../../../../../data/registry.json".to_string(),
            format!("ln -s {data} link && printf x > link"),
            format!("ln {data} hard && printf x >> hard"),
            format!("mv {data} moved"),
            format!("truncate -s 0 {data}"),
            format!("rm {data}"),
            format!("printf x > {lib}"),
            format!("printf x > {}/new.txt", layout.project.display()),
            format!(
                "mv {} {}-moved",
                layout.project.display(),
                layout.project.display()
            ),
        ] {
            let out = run(&sandbox, &layout.worktree, &script);
            assert!(!out.status.success(), "{script} was allowed");
        }
        assert_eq!(read(&layout.data), "host");
        assert_eq!(read(&layout.checkout.join("lib.rs")), "base");
        assert!(!layout.project.join("new.txt").exists());
        assert!(!layout.worktree.join("moved").exists());
    }

    #[test]
    fn a_landlocked_child_writes_inside_its_roots() {
        if probe().is_err() {
            eprintln!("skipped: no usable Landlock here");
            return;
        }
        let layout = layout();
        let temp = [layout.base.parent().unwrap().to_path_buf()];
        let sandbox =
            LandlockSandbox::build(&sealed(&layout), &[layout.worktree.clone()], &temp).unwrap();
        let private = sandbox
            .private_temp()
            .expect("the temp dir holds a sealed root");
        assert!(private.starts_with(&temp[0]));
        let script = format!(
            "printf ok > own.txt && mkdir -p src/deep && mv own.txt src/deep/own.txt \
             && t=$(mktemp) && printf x > \"$t\" && rm \"$t\" \
             && printf ok > {}/beside.txt && printf ok > /dev/null",
            layout.beside.display()
        );
        let out = run(&sandbox, &layout.worktree, &script);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(read(&layout.worktree.join("src/deep/own.txt")), "ok");
        assert_eq!(read(&layout.beside.join("beside.txt")), "ok");
        // Stricter than the macOS profile: nothing new directly in an
        // ancestor of a sealed root.
        let out = run(
            &sandbox,
            &layout.worktree,
            &format!("printf x > {}/new", layout.base.display()),
        );
        assert!(!out.status.success());
        let private = private.to_path_buf();
        drop(sandbox);
        assert!(!private.exists(), "the private temp dir is removed");
    }
}
