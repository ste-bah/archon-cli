use std::path::Path;

pub(crate) fn openbb_health(venv: &Path) -> String {
    if !venv.is_dir() {
        return format!("missing ({})", venv.display());
    }
    let python = if cfg!(windows) {
        venv.join("Scripts/python.exe")
    } else {
        venv.join("bin/python")
    };
    if std::fs::symlink_metadata(&python).is_err() {
        return format!("missing interpreter ({})", python.display());
    }
    let output = archon_shell::spawn::command(&python)
        .args(["-c", "import openbb"])
        .output();
    match output {
        Ok(output) if output.status.success() => format!("healthy ({})", python.display()),
        _ => format!("broken ({})", python.display()),
    }
}
