//! Named permission/sandbox presets and the cross-field coherence check
//! (#200 Phase 3).
//!
//! # Why this module exists
//!
//! Permissions and the sandbox are two subsystems with nine enumerable knobs
//! between them, and nothing in the tree knew the knobs were related.
//! [`SandboxPolicy::validate`](crate::sandbox::SandboxPolicy::validate) checks
//! each field against its own allowed set, independently, so this validates
//! cleanly and means nothing:
//!
//! ```toml
//! [sandbox]
//! backend = "disabled"           # nothing is isolated
//! workspace_access = "scratch"   # so which workspace mount is scratch?
//! scope = "tool"                 # so what container is scoped to what?
//! ```
//!
//! Three of the four sandbox fields are only read by a backend that actually
//! isolates something ([`SandboxBackendKind::is_real_isolation`]). Under
//! `disabled` or `logical` they are silently inert. The same hole spans the
//! two subsystems: `PermissionMode::Bubble` is documented "auto-approve ONLY
//! within sandbox limits", and paired with `backend = "disabled"` that means
//! auto-approve within limits that do not exist.
//!
//! # The one discipline this layer keeps
//!
//! **A preset records intent. It never intercepts.**
//!
//! [`apply_permission_preset`] writes the five knobs it names and nothing
//! else. `archon-permissions`' checker and every sandbox backend keep reading
//! exactly the fields they read today; no decision path consults a preset
//! name, and none of their code changed for this module to exist. If a future
//! change makes a checker branch on a preset, the preset layer has become a
//! second enforcement path with its own bugs and the value here is gone.
//!
//! For the same reason [`CUSTOM_PRESET`] is *derived*, never stored. A
//! hand-edited config and a per-agent override such as the `Bubble` set on the
//! synthetic fork agent in `crate::agents::built_in` keep working untouched;
//! they simply read back as `custom`.

use archon_permissions::mode::PermissionMode;

use super::{ArchonConfig, ConfigError};
use crate::sandbox::{SandboxBackendKind, SandboxConfig};

/// The name reported when the current knob values match no preset.
///
/// Derived on read, never written to a config file. A config that says
/// `custom` anywhere is a bug, not a state.
pub const CUSTOM_PRESET: &str = "custom";

/// The preset a fresh install is closest to, and the one the selector marks.
pub const DEFAULT_PRESET: &str = "workspace-write";

/// One coherent point in the permission x sandbox space.
///
/// Every field is populated, including the sandbox fields a preset does not
/// care about — those carry [`SandboxConfig::default`]'s values. Storing the
/// full tuple rather than a sparse overlay is what makes [`apply_permission_preset`]
/// and [`derive_permission_preset`] exact inverses: applying then deriving
/// returns the same name, with no "unspecified means whatever was there
/// before" ambiguity in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionPreset {
    /// Stable identifier, as typed at `/permissions preset <name>`.
    pub name: &'static str,
    /// One line explaining what the combination buys, shown in the selector.
    pub description: &'static str,
    /// Canonical [`PermissionMode`] name written to `permissions.mode`.
    pub permission_mode: &'static str,
    /// Written to `sandbox.backend`.
    pub sandbox_backend: &'static str,
    /// Written to `sandbox.mode`.
    pub sandbox_mode: &'static str,
    /// Written to `sandbox.scope`.
    pub sandbox_scope: &'static str,
    /// Written to `sandbox.workspace_access`.
    pub sandbox_workspace_access: &'static str,
}

/// The preset table.
///
/// `sandboxed` is the entry that makes `Bubble` mean what its doc comment
/// says. Nothing previously guided anyone to pair the two, which is how
/// `bubble` + `backend = "disabled"` — auto-approve with no limits to be
/// inside of — became reachable by accident.
pub const PERMISSION_PRESETS: &[PermissionPreset] = &[
    PermissionPreset {
        name: "read-only",
        description: "Explore and plan. No writes, no execution.",
        permission_mode: "plan",
        sandbox_backend: "disabled",
        sandbox_mode: "risky",
        sandbox_scope: "session",
        sandbox_workspace_access: "ro",
    },
    PermissionPreset {
        name: "workspace-write",
        description: "Edit files freely, confirm shell commands.",
        permission_mode: "acceptEdits",
        sandbox_backend: "logical",
        sandbox_mode: "risky",
        sandbox_scope: "session",
        sandbox_workspace_access: "rw",
    },
    PermissionPreset {
        name: "sandboxed",
        description: "Auto-approve everything, but only inside a container.",
        permission_mode: "bubble",
        sandbox_backend: "docker",
        sandbox_mode: "all",
        sandbox_scope: "session",
        sandbox_workspace_access: "rw",
    },
    PermissionPreset {
        name: "sandboxed-throwaway",
        description: "Run untrusted code. Nothing reaches the real tree.",
        permission_mode: "bubble",
        sandbox_backend: "docker",
        sandbox_mode: "all",
        sandbox_scope: "turn",
        sandbox_workspace_access: "scratch",
    },
    PermissionPreset {
        name: "unrestricted",
        description: "No checks at all. You are the sandbox.",
        permission_mode: "bypassPermissions",
        sandbox_backend: "disabled",
        sandbox_mode: "risky",
        sandbox_scope: "session",
        sandbox_workspace_access: "ro",
    },
];

/// Look a preset up by name. Exact match only — a near miss is reported by
/// [`apply_permission_preset`] with the full list rather than guessed at.
pub fn find_permission_preset(name: &str) -> Option<&'static PermissionPreset> {
    PERMISSION_PRESETS.iter().find(|preset| preset.name == name)
}

/// Comma-separated preset names, for error and help text.
pub fn permission_preset_names() -> String {
    PERMISSION_PRESETS
        .iter()
        .map(|preset| preset.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Write a preset's tuple through the existing knobs.
///
/// This is the whole of what selecting a preset does. Five field assignments,
/// no registration, no interception, nothing that any checker or backend will
/// later read as "a preset is active".
pub fn apply_permission_preset(
    config: &mut ArchonConfig,
    name: &str,
) -> Result<&'static PermissionPreset, ConfigError> {
    let preset = find_permission_preset(name).ok_or_else(|| {
        ConfigError::ValidationError(format!(
            "unknown permission preset \"{name}\"; known presets: {}",
            permission_preset_names()
        ))
    })?;

    config.permissions.mode = preset.permission_mode.to_string();
    config.sandbox.backend = preset.sandbox_backend.to_string();
    config.sandbox.mode = preset.sandbox_mode.to_string();
    config.sandbox.scope = preset.sandbox_scope.to_string();
    config.sandbox.workspace_access = preset.sandbox_workspace_access.to_string();

    Ok(preset)
}

/// Name the preset the current knob values correspond to, or [`CUSTOM_PRESET`].
///
/// The permission mode is compared after parsing, so the legacy aliases
/// (`ask`, `yolo`) resolve to the same point as their canonical spellings
/// instead of reading as `custom`. The sandbox fields are compared as written,
/// because they have no aliases.
pub fn derive_permission_preset(config: &ArchonConfig) -> &'static str {
    derive_permission_preset_from_parts(
        &config.permissions.mode,
        &config.sandbox.backend,
        &config.sandbox.mode,
        &config.sandbox.scope,
        &config.sandbox.workspace_access,
    )
}

/// [`derive_permission_preset`] over loose values.
///
/// The live session carries the permission mode and the sandbox knobs
/// separately — they are not reassembled into an `ArchonConfig` anywhere — so
/// the runtime asks in the shape it actually holds. One implementation, two
/// entry points: a second copy of the comparison is how a selector starts
/// disagreeing about which preset is in force.
pub fn derive_permission_preset_from_parts(
    permission_mode: &str,
    sandbox_backend: &str,
    sandbox_mode: &str,
    sandbox_scope: &str,
    sandbox_workspace_access: &str,
) -> &'static str {
    let mode = permission_mode.parse::<PermissionMode>().ok();

    PERMISSION_PRESETS
        .iter()
        .find(|preset| {
            mode.is_some()
                && mode == preset.permission_mode.parse::<PermissionMode>().ok()
                && sandbox_backend == preset.sandbox_backend
                && sandbox_mode == preset.sandbox_mode
                && sandbox_scope == preset.sandbox_scope
                && sandbox_workspace_access == preset.sandbox_workspace_access
        })
        .map(|preset| preset.name)
        .unwrap_or(CUSTOM_PRESET)
}

/// Cross-field coherence warnings for a loaded config.
///
/// Warnings, never errors: every config that loads today must keep loading.
/// Each message names **both** fields and says why they conflict, because a
/// warning that names one field sends the reader to the field that is fine.
///
/// A config that exactly matches a preset is coherent by definition and is not
/// checked further. That is not an escape hatch, it is what the table is for:
/// the presets are the curated statement of which combinations are meant, so a
/// rule that contradicts one of them is wrong about that combination, not the
/// other way round. It matters concretely — `workspace-write`, the default,
/// pairs `sandbox.backend = "logical"` with `workspace_access = "rw"`, and
/// `workspace_access` is only read by the Docker backend's mount flags
/// (`crate::sandbox::docker::exec`). The field is inert there, deliberately, as
/// the declaration of what the workspace should be if an isolation backend is
/// switched on later. Warning about it on every single load would teach the
/// reader to skip the one message that catches the real thing.
pub fn permission_coherence_warnings(config: &ArchonConfig) -> Vec<String> {
    let mut warnings = Vec::new();

    if derive_permission_preset(config) != CUSTOM_PRESET {
        return warnings;
    }

    let backend = match config.sandbox.backend_kind() {
        Ok(backend) => backend,
        Err(error) => {
            // Reachable only when this is called on a config that never went
            // through `validate` — which rejects an unknown backend outright.
            // Say so rather than returning an empty list, which would read as
            // "checked, coherent".
            warnings.push(format!(
                "sandbox.backend could not be parsed ({error}), so no coherence check was \
                 performed between sandbox.backend and permissions.mode / sandbox.mode / \
                 sandbox.scope / sandbox.workspace_access"
            ));
            return warnings;
        }
    };

    let defaults = SandboxConfig::default();
    if !backend.is_real_isolation() {
        for (field, value, default) in [
            ("sandbox.mode", &config.sandbox.mode, &defaults.mode),
            ("sandbox.scope", &config.sandbox.scope, &defaults.scope),
            (
                "sandbox.workspace_access",
                &config.sandbox.workspace_access,
                &defaults.workspace_access,
            ),
        ] {
            if value != default {
                warnings.push(format!(
                    "{field} = \"{value}\" conflicts with sandbox.backend = \"{backend}\": \
                     {field} is only read by a backend that really isolates (docker, ssh, \
                     openshell), and \"{backend}\" starts no container or remote host for it to \
                     describe, so the setting is silently inert. Either set sandbox.backend to an \
                     isolation backend or return {field} to its default \"{default}\"."
                ));
            }
        }
    }

    if config.permissions.mode.parse::<PermissionMode>().ok() == Some(PermissionMode::Bubble)
        && backend == SandboxBackendKind::Disabled
    {
        warnings.push(format!(
            "permissions.mode = \"{mode}\" conflicts with sandbox.backend = \"disabled\": bubble \
             auto-approves every tool call \"only within sandbox limits\", and a disabled backend \
             imposes no limits, so the pair is indistinguishable from dontAsk. Set \
             sandbox.backend to docker, ssh, or openshell (the \"sandboxed\" preset does both), \
             or choose a permission mode that still prompts.",
            mode = config.permissions.mode
        ));
    }

    warnings
}

#[cfg(test)]
#[path = "permission_presets_tests.rs"]
mod tests;
