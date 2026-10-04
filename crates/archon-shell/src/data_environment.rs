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
//! arbitrary keys from it (`CARGO_`, `GIT_CONFIG_`, `DYLD_`). Application
//! namespaces such as PYTHON_API_KEY and GIT_SERVICE_TOKEN remain usable.
//! Names must also end in DATA_SUFFIXES: a positive, documented data shape.
//! Neither membership in the allowlist nor a data suffix overrides a control.
//!
//! Additional primary references: lua.org/manual/5.4/lua.html;
//! stat.math.ethz.ch/R-manual/R-devel/library/base/html/Startup.html;
//! php.net/manual/en/configuration.file.php; Microsoft's CL environment
//! variables and MSVC linker reference; rust-lang/cc-rs src/lib.rs;
//! apple-oss-distributions/libmalloc src/nano_malloc_common.c;
//! docs.julialang.org/en/v1/manual/environment-variables/;
//! stat.math.ethz.ch/CRAN/doc/manuals/r-devel/R-admin.pdf;
//! docs.docker.com/reference/cli/docker/ and
//! docs.podman.io/en/stable/markdown/podman.1.html.

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
        prefixes: &["LD_", "DYLD_", "MALLOC"],
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
            "CXXSTDLIB",
            "CL",
            "_CL_",
            "LINK",
            "_LINK_",
            "LIB",
            "LIBPATH",
            "INCLUDE",
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
            "CXXSTDLIB",
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
                 (CPython controls use PYTHON<OPTION> and specific PYTHON_* branches)",
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
        prefixes: &[
            "GIT_CONFIG",
            "GIT_SSH",
            "GIT_EXEC",
            "GIT_ASKPASS",
            "GIT_EDITOR",
            "GIT_SEQUENCE_EDITOR",
            "GIT_PAGER",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT",
            "GIT_ALTERNATE",
            "GIT_INDEX",
            "GIT_CEILING",
            "GIT_DISCOVERY",
            "GIT_NAMESPACE",
            "GIT_TEMPLATE",
            "GIT_TRACE",
            "GIT_ATTR",
            "GIT_SSL",
            "GIT_PROXY",
            "GIT_PROTOCOL",
        ],
    },
    Family {
        reason: "Lua runs LUA_INIT* before scripts and uses LUA_PATH*/LUA_CPATH* for \
                 module loading; R loads R_PROFILE*/R_ENVIRON* startup files and R_LIBS* \
                 packages; R_MAKEVARS* selects build configuration; PHP reads \
                 PHPRC/PHP_INI_SCAN_DIR configuration; Julia's JULIA_PROJECT and \
                 JULIA_LOAD_PATH/JULIA_DEPOT_PATH select the code-loading environment",
        names: &[
            "PHPRC",
            "PHP_INI_SCAN_DIR",
            "R_HOME",
            "JULIA_PROJECT",
            "JULIA_BINDIR",
        ],
        prefixes: &[
            "LUA_INIT",
            "LUA_PATH",
            "LUA_CPATH",
            "R_PROFILE",
            "R_ENVIRON",
            "R_LIBS",
            "R_STARTUP",
            "R_MAKEVARS",
            "JULIA_LOAD_PATH",
            "JULIA_DEPOT_PATH",
        ],
    },
    Family {
        reason: "Docker and Podman read these bindings to select the container engine, \
                 remote execution host, client configuration or SSH identity",
        names: &[
            "DOCKER_HOST",
            "DOCKER_CONTEXT",
            "DOCKER_CONFIG",
            "DOCKER_API_VERSION",
            "CONTAINER_HOST",
            "CONTAINER_CONNECTION",
            "CONTAINER_SSHKEY",
        ],
        prefixes: &[],
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
                || family.prefixes.iter().any(|p| {
                    if *p == "PYTHON" {
                        // CPython controls use PYTHON<OPTION>; the underscored
                        // namespace has only these documented control branches.
                        (upper.starts_with("PYTHON") && !upper.starts_with("PYTHON_"))
                            || [
                                "PYTHON_GIL",
                                "PYTHON_CPU_COUNT",
                                "PYTHON_THREAD",
                                "PYTHON_JIT",
                                "PYTHON_PERF",
                                "PYTHON_CONTEXT",
                                "PYTHON_TLBC",
                                "PYTHON_PRESITE",
                                "PYTHON_REMOTE",
                            ]
                            .iter()
                            .any(|p| upper.starts_with(p))
                    } else if *p == "NODE_" {
                        [
                            "NODE_OPTIONS",
                            "NODE_PATH",
                            "NODE_REPL",
                            "NODE_EXTRA_CA_CERTS",
                            "NODE_ICU_DATA",
                            "NODE_V8",
                            "NODE_REDIRECT",
                            "NODE_TLS",
                            "NODE_PENDING",
                            "NODE_NO_",
                            "NODE_DISABLE",
                            "NODE_COMPILE",
                            "NODE_USE_",
                        ]
                        .iter()
                        .any(|p| upper.starts_with(p))
                    } else {
                        upper.starts_with(p)
                    }
                })
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
             '_', not starting with a digit); rename the application's data binding with \
             valid characters and a documented data suffix"
        ));
    }
    if let Some(reason) = execution_control(name) {
        return Err(format!(
            "'{name}' cannot be forwarded: {reason}. A forwarded variable may only carry data \
             (a provider endpoint or key); configure toolchains and runtimes on the host, not \
             through the allowlist"
        ));
    }
    let upper = name.to_ascii_uppercase();
    if !DATA_SUFFIXES
        .iter()
        .any(|suffix| upper.ends_with(suffix) && upper.len() > suffix.len())
    {
        return Err(format!(
            "'{name}' cannot be forwarded: data-name rule requires one of {}. \
             A forwarded variable may only carry data; rename the application's data binding \
             with a documented suffix (for example SERVICE_API_KEY), or configure execution \
             controls on the host outside the allowlist",
            DATA_SUFFIXES.join(", ")
        ));
    }
    Ok(())
}

/// Reviewed shapes for credentials, provider addresses and account identifiers.
const DATA_SUFFIXES: &[&str] = &[
    "_API_KEY",
    "_KEY",
    "_TOKEN",
    "_SECRET",
    "_PASSWORD",
    "_URL",
    "_URI",
    "_ENDPOINT",
    "_HOST",
    "_PORT",
    "_REGION",
    "_ACCOUNT",
    "_PROJECT",
    "_ORG",
    "_USER",
    "_ID",
];

#[cfg(test)]
#[path = "data_environment_tests.rs"]
mod tests;
