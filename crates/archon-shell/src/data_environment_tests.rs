use super::*;

/// One known injection or toolchain-control name from each family, as its
/// own documentation spells it.
const CONTROLS: &[&str] = &[
    // Process bindings the host sets itself.
    "PATH",
    "HOME",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "PATHEXT",
    "COMSPEC",
    // Dynamic loaders and the C library.
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "GLIBC_TUNABLES",
    "GCONV_PATH",
    "LIBPATH",
    // Rust and Cargo.
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
    "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER",
    "CARGO_SOURCE_CRATES_IO_REPLACE_WITH",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "RUSTUP_TOOLCHAIN",
    // C toolchain read by build scripts.
    "CC",
    "CXX",
    "CFLAGS",
    "LDFLAGS",
    "CC_x86_64_unknown_linux_gnu",
    "TARGET_CC",
    "PKG_CONFIG_PATH",
    "CMAKE_TOOLCHAIN_FILE",
    "OPENSSL_CONF",
    "LIBCLANG_PATH",
    "MAKEFLAGS",
    // Python.
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "PYTHONBREAKPOINT",
    "PYTHON_GIL",
    "PIP_INDEX_URL",
    "UV_PYTHON",
    "PYTEST_ADDOPTS",
    "VIRTUAL_ENV",
    // Node.js.
    "NODE_OPTIONS",
    "NODE_PATH",
    "NPM_CONFIG_SCRIPT_SHELL",
    "YARN_RC_FILENAME",
    // Shells.
    "BASH_ENV",
    "ENV",
    "SHELLOPTS",
    "PS4",
    "PROMPT_COMMAND",
    "ZDOTDIR",
    "IFS",
    // Other interpreters and toolchains.
    "PERL5OPT",
    "PERL5LIB",
    "RUBYOPT",
    "GEM_HOME",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "CLASSPATH",
    "GOFLAGS",
    "GOTOOLCHAIN",
    "GOPROXY",
    "CGO_LDFLAGS",
    // Git and the helper programs tools run.
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_SSH_COMMAND",
    "GIT_EXEC_PATH",
    "GIT_DIR",
    "EDITOR",
    "PAGER",
    "SSH_ASKPASS",
    // Windows runtimes.
    "COR_PROFILER_PATH",
    "COR_ENABLE_PROFILING",
    "CORECLR_PROFILER_PATH",
    "DOTNET_STARTUP_HOOKS",
    "COMPLUS_ENABLEEVENTPIPE",
    "PSMODULEPATH",
    "__COMPAT_LAYER",
];

#[test]
fn every_known_execution_control_is_refused_and_named() {
    for name in CONTROLS {
        let error = check_data_variable(name).expect_err(name);
        assert!(error.contains(&format!("'{name}'")), "{name}: {error}");
        assert!(error.contains("only carry data"), "{name}: {error}");
    }
}

/// Windows environment names are case-insensitive, so `Ld_Preload` and
/// `node_options` reach the same loader and runtime as the documented
/// spelling; one config must mean the same on every platform.
#[test]
fn refusal_is_case_insensitive() {
    for name in [
        "ld_preload",
        "Dyld_Insert_Libraries",
        "node_options",
        "Cargo_Build_Rustc",
        "rustc_workspace_wrapper",
        "PythonPath",
        "git_config_count",
        "npm_config_registry",
        "Path",
        "dotnet_startup_hooks",
    ] {
        assert!(check_data_variable(name).is_err(), "{name} accepted");
    }
}

/// Provider endpoints and keys stay allowed, including names that only
/// look like a refused family (`GITHUB_` is not `GIT_`, `LDAP_` is not
/// `LD_`, `CARGOWISE_` is not `CARGO_`, `NODEJS_` is not `NODE_`).
#[test]
fn data_names_are_accepted() {
    for name in [
        "POLYGON_API_KEY",
        "OPENBB_API_URL",
        "ANTHROPIC_API_KEY",
        "GOOGLE_API_KEY",
        "GITHUB_TOKEN",
        "LDAP_URL",
        "CARGOWISE_TOKEN",
        "NODEJS_SERVICE_URL",
        "DATABASE_URL",
        "COREDATA_ENDPOINT",
        "ACCESS_TOKEN",
    ] {
        assert_eq!(check_data_variable(name), Ok(()), "{name} refused");
    }
}

#[test]
fn malformed_names_are_refused() {
    for name in [
        "",
        "1KEY",
        "KEY-NAME",
        "KEY NAME",
        "KEY=VALUE",
        "PROGRAMFILES(X86)",
    ] {
        let error = check_data_variable(name).expect_err(name);
        assert!(error.contains("not a valid"), "{name}: {error}");
    }
}

#[test]
fn reason_names_the_family() {
    let loader = check_data_variable("LD_PRELOAD").unwrap_err();
    assert!(loader.contains("dynamic loader"), "{loader}");
    let cargo = check_data_variable("CARGO_ENCODED_RUSTFLAGS").unwrap_err();
    assert!(cargo.contains("Cargo"), "{cargo}");
    let git = check_data_variable("GIT_CONFIG_COUNT").unwrap_err();
    assert!(git.contains("git"), "{git}");
}

#[test]
fn round2_controls_and_unknown_names_are_refused() {
    for name in [
        "LUA_INIT",
        "LUA_INIT_5_4",
        "LUA_PATH",
        "LUA_CPATH",
        "R_PROFILE_USER",
        "R_ENVIRON",
        "R_ENVIRON_USER",
        "PHPRC",
        "PHP_INI_SCAN_DIR",
        "CL",
        "_CL_",
        "LINK",
        "_LINK_",
        "CXXSTDLIB",
        "MallocNanoZone",
        "mallocStackLogging",
        "LUA_INIT_TOKEN",
        "R_PROFILE_URL",
        "MallocCustom_KEY",
        "UNKNOWN_CONTROL",
        "HTTPS_PROXY",
        "SSL_CERT_FILE",
        "RUST_LOG",
    ] {
        assert!(check_data_variable(name).is_err(), "{name} accepted");
    }
}

#[test]
fn round2_application_data_names_are_accepted() {
    for name in [
        "POLYGON_API_KEY",
        "OPENBB_API_URL",
        "ANTHROPIC_API_KEY",
        "PYTHON_API_KEY",
        "NODE_API_URL",
        "GIT_SERVICE_TOKEN",
        "JULIA_API_KEY",
        "DOCKER_API_URL",
    ] {
        assert_eq!(check_data_variable(name), Ok(()), "{name}");
    }
}

#[test]
fn round2_data_shaped_runtime_controls_are_refused() {
    let accepted: Vec<_> = [
        "JULIA_PROJECT",
        "R_MAKEVARS_USER",
        "DOCKER_HOST",
        "CONTAINER_HOST",
    ]
    .into_iter()
    .filter(|name| check_data_variable(name).is_ok())
    .collect();
    assert!(
        accepted.is_empty(),
        "execution controls accepted: {accepted:?}"
    );
}

#[test]
fn cluster_bindings_and_download_sources_are_refused() {
    for name in [
        "NODE_UNIQUE_ID",
        "NODE_CHANNEL_FD",
        "node_channel_serialization_mode",
        "BUILDKIT_HOST",
        "PLAYWRIGHT_DOWNLOAD_HOST",
        "PUPPETEER_DOWNLOAD_BASE_URL",
        "POETRY_REPOSITORIES_PRIVATE_URL",
        "PDM_PYPI_URL",
    ] {
        assert!(check_data_variable(name).is_err(), "{name} was accepted");
    }
    for name in [
        "NODE_AUTH_TOKEN",
        "POLYGON_API_KEY",
        "OPENBB_API_URL",
        "SERVICE_HOST",
    ] {
        assert!(check_data_variable(name).is_ok(), "{name} was refused");
    }
}
