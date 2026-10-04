//! Which host environment names may be forwarded to a child as data.
//!
//! Issue 282. An operator allowlists host variables so a check can reach a
//! provider (an endpoint, a key). Such a value must never change what code
//! runs or how it is built. Many variables do exactly that: the dynamic loader
//! preloads `LD_PRELOAD`, Cargo reads every configuration key from
//! `CARGO_<KEY>` (`target.<triple>.runner`, `build.rustc-wrapper`, source
//! replacement), Python runs `PYTHONSTARTUP`, Node preloads `NODE_OPTIONS
//! --require`, bash sources `BASH_ENV`, git takes `core.hooksPath` from
//! `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`, and .NET loads
//! `COR_PROFILER_PATH`. Each family below names the program that reads it.
//!
//! Sources: glibc's unsecure-environment list (`sysdeps/generic/unsecvars.h`),
//! dyld(1), the Cargo book (environment variables, configuration), the cc,
//! cmake, pkg-config and bindgen crates, the CPython, Node.js, npm, bash,
//! zsh, Perl, Ruby, JVM, Go and git documentation, and the .NET profiling
//! and startup-hook documentation.
//!
//! The match is ASCII case-insensitive on every platform: Windows names are
//! case-insensitive (`Path` is PATH), and one configuration must mean the same
//! on every machine. A whole namespace is refused where its owner reads
//! arbitrary keys from it (`CARGO_`, `GIT_`, `PYTHON`, `NODE_`, `DYLD_`),
//! so a key added in a later toolchain release is refused too.

struct Family {
    reason: &'static str,
    names: &'static [&'static str],
    prefixes: &'static [&'static str],
}

const FAMILIES: &[Family] = &[
    Family {
        reason: "it is a process binding the host sets itself (executable search, home, \
                 temporary or configuration directories, shell); a forwarded value would \
                 relocate where programs and their configuration are found",
        names: &[
            "PATH",
            "HOME",
            "TMPDIR",
            "TMP",
            "TEMP",
            "SHELL",
            "USERPROFILE",
            "HOMEDRIVE",
            "HOMEPATH",
            "SYSTEMROOT",
            "SYSTEMDRIVE",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "APPDATA",
            "LOCALAPPDATA",
            "PROGRAMFILES",
            "PROGRAMW6432",
            "PROGRAMDATA",
        ],
        prefixes: &["XDG_"],
    },
    Family {
        reason: "the dynamic loader or C library reads it to preload, audit or relocate \
                 shared libraries and modules (glibc LD_*/GCONV_PATH/GLIBC_TUNABLES, macOS \
                 DYLD_*, AIX LIBPATH, HP-UX SHLIB_PATH)",
        names: &[
            "LIBPATH",
            "SHLIB_PATH",
            "GCONV_PATH",
            "GETCONF_DIR",
            "GLIBC_TUNABLES",
            "HOSTALIASES",
            "LOCALDOMAIN",
            "LOCPATH",
            "NIS_PATH",
            "NLSPATH",
            "RESOLV_HOST_CONF",
            "RES_OPTIONS",
            "TZDIR",
        ],
        prefixes: &["LD_", "DYLD_", "MALLOC_"],
    },
    Family {
        reason: "Cargo reads every configuration key from CARGO_<KEY> (build.rustc, \
                 build.rustc-wrapper, target.<triple>.runner and linker, source replacement, \
                 registries, directories), and RUSTC*/RUSTDOC*/RUSTFLAGS/RUSTUP_* select, \
                 wrap or flag the compiler",
        names: &["RUSTFLAGS"],
        prefixes: &["CARGO_", "RUSTC", "RUSTDOC", "RUSTUP_"],
    },
    Family {
        reason: "build scripts and build tools (the cc, cmake, pkg-config, bindgen and \
                 openssl-sys crates, cgo, make, the Apple SDK tools) read it to choose the \
                 compiler, linker, flags, SDK or library search",
        names: &[
            "CC",
            "CXX",
            "CPP",
            "AR",
            "AS",
            "LD",
            "NM",
            "RANLIB",
            "STRIP",
            "OBJCOPY",
            "CFLAGS",
            "CXXFLAGS",
            "CPPFLAGS",
            "LDFLAGS",
            "LDLIBS",
            "ARFLAGS",
            "ASFLAGS",
            "TARGET_CC",
            "TARGET_CXX",
            "TARGET_AR",
            "TARGET_CFLAGS",
            "TARGET_CXXFLAGS",
            "TARGET_ARFLAGS",
            "HOST_CC",
            "HOST_CXX",
            "HOST_AR",
            "HOST_CFLAGS",
            "HOST_CXXFLAGS",
            "HOST_ARFLAGS",
            "CRATE_CC_NO_DEFAULTS",
            "MAKE",
            "MAKEFLAGS",
            "MFLAGS",
            "GNUMAKEFLAGS",
            "MAKEFILES",
            "CMAKE",
            "LIBRARY_PATH",
            "CPATH",
            "C_INCLUDE_PATH",
            "CPLUS_INCLUDE_PATH",
            "OBJC_INCLUDE_PATH",
            "GCC_EXEC_PREFIX",
            "COMPILER_PATH",
            "LIBCLANG_PATH",
            "CLANG_PATH",
            "BINDGEN_EXTRA_CLANG_ARGS",
            "SDKROOT",
            "DEVELOPER_DIR",
            "MACOSX_DEPLOYMENT_TARGET",
        ],
        prefixes: &[
            "CC_",
            "CXX_",
            "CFLAGS_",
            "CXXFLAGS_",
            "AR_",
            "ARFLAGS_",
            "RANLIB_",
            "PKG_CONFIG",
            "CMAKE_",
            "OPENSSL_",
            "CGO_",
        ],
    },
    Family {
        reason: "the Python interpreter, pip, uv or pytest reads it to choose the \
                 interpreter, module search path, startup code, plugins or package index \
                 (CPython reserves the PYTHON* names)",
        names: &["VIRTUAL_ENV"],
        prefixes: &["PYTHON", "PIP_", "UV_", "PYTEST_"],
    },
    Family {
        reason: "Node.js, npm, yarn, Bun or Deno reads it to preload code (NODE_OPTIONS \
                 --require), change module resolution, or set package-manager \
                 configuration such as the registry and script shell",
        names: &[],
        prefixes: &["NODE_", "NPM_CONFIG_", "YARN_", "BUN_", "DENO_"],
    },
    Family {
        reason: "a shell reads it at startup to source a file, define functions or run \
                 commands (bash BASH_ENV and BASH_FUNC_*, POSIX ENV, zsh ZDOTDIR, xtrace \
                 PS4, PROMPT_COMMAND, IFS word splitting)",
        names: &[
            "BASH_ENV",
            "ENV",
            "SHELLOPTS",
            "BASHOPTS",
            "PS4",
            "PROMPT_COMMAND",
            "CDPATH",
            "GLOBIGNORE",
            "ZDOTDIR",
            "INPUTRC",
            "IFS",
        ],
        prefixes: &["BASH_FUNC_"],
    },
    Family {
        reason: "the Perl, Ruby, JVM or Go toolchain reads it to load modules or agents, \
                 add interpreter options, or select the toolchain and module source",
        names: &[
            "PERL5LIB",
            "PERL5OPT",
            "PERLLIB",
            "PERL5DB",
            "RUBYOPT",
            "RUBYLIB",
            "JAVA_TOOL_OPTIONS",
            "_JAVA_OPTIONS",
            "JDK_JAVA_OPTIONS",
            "CLASSPATH",
            "JAVA_HOME",
            "GOFLAGS",
            "GOROOT",
            "GOPATH",
            "GOBIN",
            "GOPROXY",
            "GOSUMDB",
            "GONOSUMDB",
            "GONOPROXY",
            "GOPRIVATE",
            "GOINSECURE",
            "GOTOOLCHAIN",
            "GOENV",
            "GOWORK",
            "GOEXPERIMENT",
            "GODEBUG",
            "GOOS",
            "GOARCH",
            "GOCACHE",
            "GOMODCACHE",
            "GO111MODULE",
        ],
        prefixes: &["GEM_", "BUNDLE_"],
    },
    Family {
        reason: "git reads GIT_* to run hooks, helpers and commands (GIT_CONFIG_COUNT and \
                 GIT_CONFIG_KEY_n can set core.hooksPath or core.fsmonitor; GIT_SSH_COMMAND, \
                 GIT_EXEC_PATH, GIT_DIR), and tools run the EDITOR, PAGER and askpass programs",
        names: &[
            "EDITOR",
            "VISUAL",
            "PAGER",
            "LESSOPEN",
            "LESSCLOSE",
            "SSH_ASKPASS",
            "SUDO_ASKPASS",
        ],
        prefixes: &["GIT_"],
    },
    Family {
        reason: ".NET, PowerShell or Windows reads it to load a profiler, startup-hook \
                 assembly or module, or to apply compatibility shims (COR_PROFILER_PATH, \
                 CORECLR_PROFILER_PATH, DOTNET_STARTUP_HOOKS, COMPlus_*, PSModulePath, \
                 __COMPAT_LAYER)",
        names: &["PSMODULEPATH", "__COMPAT_LAYER"],
        prefixes: &["COR_", "CORECLR_", "DOTNET_", "COMPLUS_"],
    },
];

/// Why `name` may not be forwarded as data, or `None` when it may. Only the
/// name is examined, never a value.
pub fn execution_control(name: &str) -> Option<&'static str> {
    let upper = name.to_ascii_uppercase();
    FAMILIES
        .iter()
        .find(|family| {
            family.names.contains(&upper.as_str())
                || family.prefixes.iter().any(|p| upper.starts_with(p))
        })
        .map(|family| family.reason)
}

/// `Ok` when `name` is a well-formed variable name that only carries data;
/// otherwise the refusal, naming the variable and why.
pub fn check_data_variable(name: &str) -> Result<(), String> {
    let well_formed = !name.is_empty()
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()));
    if !well_formed {
        return Err(format!(
            "'{name}' is not a valid environment variable name (ASCII letters, digits and \
             '_', not starting with a digit)"
        ));
    }
    match execution_control(name) {
        None => Ok(()),
        Some(reason) => Err(format!(
            "'{name}' cannot be forwarded: {reason}. A forwarded variable may only carry data \
             (a provider endpoint or key); configure toolchains and runtimes on the host, not \
             through the allowlist"
        )),
    }
}

#[cfg(test)]
#[path = "data_environment_tests.rs"]
mod tests;
