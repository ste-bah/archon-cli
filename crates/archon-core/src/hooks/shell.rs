use std::path::PathBuf;
use std::sync::LazyLock;

static HOOK_SHELL: LazyLock<HookShell> = LazyLock::new(|| {
    select_hook_shell(
        cfg!(windows),
        which::which("sh").ok(),
        which::which("bash").ok(),
    )
});

#[derive(Debug, PartialEq, Eq)]
pub(super) struct HookShell {
    pub(super) program: PathBuf,
    pub(super) command_arg: &'static str,
}

pub(super) fn resolve_hook_shell() -> &'static HookShell {
    &HOOK_SHELL
}

fn select_hook_shell(is_windows: bool, sh: Option<PathBuf>, bash: Option<PathBuf>) -> HookShell {
    if is_windows {
        return HookShell {
            program: PathBuf::from("cmd"),
            command_arg: "/C",
        };
    }

    if let Some(program) = sh.or(bash) {
        return HookShell {
            program,
            command_arg: "-c",
        };
    }

    HookShell {
        program: PathBuf::from("sh"),
        command_arg: "-c",
    }
}

#[cfg(test)]
mod tests {
    use super::{HookShell, select_hook_shell};
    use std::path::PathBuf;

    #[test]
    fn windows_uses_native_cmd_even_when_posix_shells_are_discovered() {
        let shell = select_hook_shell(
            true,
            Some(PathBuf::from(r"C:\Program Files\Git\bin\sh.exe")),
            Some(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe")),
        );

        assert_eq!(
            shell,
            HookShell {
                program: PathBuf::from("cmd"),
                command_arg: "/C",
            }
        );
    }

    #[test]
    fn windows_uses_native_cmd_when_only_bash_is_discovered() {
        let shell = select_hook_shell(
            true,
            None,
            Some(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe")),
        );

        assert_eq!(
            shell,
            HookShell {
                program: PathBuf::from("cmd"),
                command_arg: "/C",
            }
        );
    }

    #[test]
    fn plain_windows_falls_back_to_cmd() {
        let shell = select_hook_shell(true, None, None);

        assert_eq!(
            shell,
            HookShell {
                program: PathBuf::from("cmd"),
                command_arg: "/C",
            }
        );
    }

    #[test]
    fn unix_falls_back_to_sh() {
        let shell = select_hook_shell(false, None, None);

        assert_eq!(
            shell,
            HookShell {
                program: PathBuf::from("sh"),
                command_arg: "-c",
            }
        );
    }
}
