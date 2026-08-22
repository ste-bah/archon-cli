//! `/permissions presets` and `/permissions preset <name>` (#200 Phase 3).
//!
//! Split out of `permissions.rs` so that file stays under the 500-line ceiling
//! the repository gate enforces.
//!
//! # What a preset does here, and what it deliberately does not
//!
//! Applying a preset calls [`archon_core::config::save_permission_preset`],
//! which writes the five knobs the preset names through their ordinary config
//! fields and validates the result. It then stashes the existing
//! [`CommandEffect::SetPermissionMode`] so the live session's permission mode
//! moves through the same path `/permissions <mode>` has always used.
//!
//! Nothing else happens. No preset name is recorded in the session, no checker
//! is told a preset is active, and no sandbox backend is asked to reconsider.
//! The preset layer records intent; the checker in `archon-permissions` and
//! the backends under `archon_core::sandbox` keep reading exactly the fields
//! they read before this module existed.
//!
//! The sandbox half lands on the next start, which is the same contract every
//! `[sandbox]` field has today — the backends are constructed from config at
//! session setup and there is no runtime setter for them. The confirmation
//! text says so rather than implying the container changed underneath you.

use archon_core::config::{
    CUSTOM_PRESET, PERMISSION_PRESETS, permission_preset_names, save_permission_preset,
};
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect};

/// The two preset sub-commands, as parsed from `/permissions`'s argument tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PresetArg {
    /// `/permissions presets` — open the selector.
    List,
    /// `/permissions preset <name>` — apply one.
    Apply(String),
    /// `/permissions preset` with no name — a usage error, not a list.
    ApplyMissingName,
}

/// Recognise the preset sub-commands inside `/permissions`' argument tail.
///
/// Returns `None` for everything else so the caller falls through to the mode
/// path unchanged. `preset` and `presets` are the only two tokens claimed, and
/// neither is a permission mode, so no existing input changes meaning.
pub(crate) fn parse_preset_arg(arg: &str) -> Option<PresetArg> {
    let mut tokens = arg.split_whitespace();
    match tokens.next()? {
        "presets" => Some(PresetArg::List),
        "preset" => match tokens.next() {
            Some(name) => Some(PresetArg::Apply(name.to_string())),
            None => Some(PresetArg::ApplyMissingName),
        },
        _ => None,
    }
}

/// Render the preset table as text.
///
/// Emitted alongside the overlay event because print mode drops TUI events,
/// and a `/permissions presets` that shows nothing outside the TUI would be a
/// command that only appears to work.
pub(crate) fn render_preset_list(active: &str) -> String {
    let mut out = format!("\nPermission presets (in force: {active})\n");
    for preset in PERMISSION_PRESETS {
        let marker = if preset.name == active { "*" } else { " " };
        out.push_str(&format!(
            "{marker} {name}  [permissions.mode = {mode}, sandbox.backend = {backend}]\n    {description}\n",
            name = preset.name,
            mode = preset.permission_mode,
            backend = preset.sandbox_backend,
            description = preset.description,
        ));
    }
    if active == CUSTOM_PRESET {
        out.push_str(
            "\nThe current settings match no preset, so they are reported as \"custom\". \
             That is a normal state — individual knobs stay settable, and a hand-edited \
             config keeps working.\n",
        );
    }
    out.push_str("\nUsage: /permissions preset <name>\n");
    out
}

/// Handle one preset sub-command.
///
/// `active` is the preset the live session's knobs correspond to, or `custom`.
pub(crate) fn handle(ctx: &mut CommandContext, arg: PresetArg, active: &str) -> anyhow::Result<()> {
    match arg {
        PresetArg::List => {
            ctx.emit(TuiEvent::TextDelta(render_preset_list(active)));
            ctx.emit(TuiEvent::ShowPermissionPresets {
                active: active.to_string(),
                presets: PERMISSION_PRESETS
                    .iter()
                    .map(|preset| {
                        (
                            preset.name.to_string(),
                            preset.description.to_string(),
                            preset.permission_mode.to_string(),
                            preset.sandbox_backend.to_string(),
                        )
                    })
                    .collect(),
            });
            Ok(())
        }
        PresetArg::ApplyMissingName => {
            ctx.emit(TuiEvent::Error(format!(
                "/permissions preset needs a preset name; known presets: {}. \
                 Use /permissions presets to see what each one does.",
                permission_preset_names()
            )));
            Ok(())
        }
        PresetArg::Apply(name) => apply(ctx, &name),
    }
}

fn apply(ctx: &mut CommandContext, name: &str) -> anyhow::Result<()> {
    // A failure here is reported, never swallowed: a preset that silently did
    // not persist would leave the session claiming a posture it does not have.
    let (path, preset) = match save_permission_preset(name) {
        Ok(saved) => saved,
        Err(error) => {
            ctx.emit(TuiEvent::Error(format!(
                "could not apply permission preset \"{name}\": {error}"
            )));
            return Ok(());
        }
    };

    ctx.emit(TuiEvent::TextDelta(format!(
        "\nPermission preset set to {name}.\n\
         permissions.mode = {mode}\n\
         sandbox.backend = {backend}, mode = {sandbox_mode}, scope = {scope}, \
         workspace_access = {workspace}\n\
         Saved: {path}\n\
         The permission mode applies to this session now; the sandbox settings \
         are read when a backend is built, so they take effect on the next start.\n",
        mode = preset.permission_mode,
        backend = preset.sandbox_backend,
        sandbox_mode = preset.sandbox_mode,
        scope = preset.sandbox_scope,
        workspace = preset.sandbox_workspace_access,
        path = path.display(),
    )));

    // The one and only write into live permission state, through the effect
    // `/permissions <mode>` already uses. No second path.
    ctx.pending_effect = Some(CommandEffect::SetPermissionMode(
        preset.permission_mode.to_string(),
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::permissions::{PermissionsHandler, PermissionsSnapshot};
    use crate::command::registry::CommandHandler;

    fn snapshot(mode: &str, active_preset: &str) -> PermissionsSnapshot {
        PermissionsSnapshot {
            current_mode: mode.to_string(),
            rules: Vec::new(),
            allow_bypass_permissions: false,
            active_preset: active_preset.to_string(),
        }
    }

    fn run(arg: &str, snap: PermissionsSnapshot) -> (Vec<TuiEvent>, Option<CommandEffect>) {
        let (mut ctx, mut rx) = crate::command::test_support::CtxBuilder::new()
            .with_permissions_snapshot(snap)
            .build();
        let args: Vec<String> = arg.split_whitespace().map(str::to_string).collect();
        PermissionsHandler
            .execute(&mut ctx, &args)
            .expect("execute");
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        (events, ctx.pending_effect)
    }

    #[test]
    fn presets_and_preset_are_the_only_claimed_tokens() {
        assert_eq!(parse_preset_arg("presets"), Some(PresetArg::List));
        assert_eq!(
            parse_preset_arg("preset sandboxed"),
            Some(PresetArg::Apply("sandboxed".into()))
        );
        assert_eq!(
            parse_preset_arg("preset"),
            Some(PresetArg::ApplyMissingName)
        );
    }

    #[test]
    fn every_permission_mode_still_falls_through_to_the_mode_path() {
        // If any of these started parsing as a preset sub-command, an existing
        // `/permissions <mode>` would silently change meaning.
        for mode in [
            "default",
            "acceptEdits",
            "plan",
            "auto",
            "dontAsk",
            "bubble",
            "bypassPermissions",
            "ask",
            "yolo",
            "",
        ] {
            assert_eq!(parse_preset_arg(mode), None, "{mode} was claimed");
        }
    }

    #[test]
    fn the_list_marks_what_is_in_force_and_explains_each_row() {
        let rendered = render_preset_list("sandboxed");

        assert!(rendered.contains("* sandboxed"), "{rendered}");
        assert!(rendered.contains("  read-only"), "{rendered}");
        assert!(
            rendered.contains("Auto-approve everything, but only inside a container."),
            "{rendered}"
        );
        assert!(rendered.contains("permissions.mode = bubble"), "{rendered}");
        assert!(rendered.contains("sandbox.backend = docker"), "{rendered}");
    }

    #[test]
    fn a_custom_posture_is_explained_rather_than_reported_as_an_error() {
        let rendered = render_preset_list(CUSTOM_PRESET);

        assert!(rendered.contains("in force: custom"), "{rendered}");
        assert!(rendered.contains("match no preset"), "{rendered}");
        assert!(
            rendered.contains("hand-edited config keeps working"),
            "{rendered}"
        );
    }

    #[test]
    fn presets_opens_the_selector_and_marks_what_is_in_force() {
        let (events, effect) = run("presets", snapshot("bubble", "sandboxed"));

        assert!(effect.is_none(), "listing presets must change nothing");
        let opened = events.iter().find_map(|event| match event {
            TuiEvent::ShowPermissionPresets { active, presets } => Some((active, presets)),
            _ => None,
        });
        let (active, presets) = opened.expect("selector event");
        assert_eq!(active, "sandboxed");
        assert_eq!(presets.len(), PERMISSION_PRESETS.len());
        assert!(
            presets
                .iter()
                .any(|(name, ..)| name == "sandboxed-throwaway")
        );
    }

    #[test]
    fn a_custom_posture_opens_the_selector_saying_custom() {
        let (events, _) = run("presets", snapshot("dontAsk", CUSTOM_PRESET));

        assert!(events.iter().any(|event| matches!(
            event,
            TuiEvent::ShowPermissionPresets { active, .. } if active == CUSTOM_PRESET
        )));
    }

    #[test]
    fn preset_without_a_name_errors_and_lists_the_names() {
        let (events, effect) = run("preset", snapshot("default", CUSTOM_PRESET));

        assert!(effect.is_none());
        let message = events
            .iter()
            .find_map(|event| match event {
                TuiEvent::Error(message) => Some(message.clone()),
                _ => None,
            })
            .expect("error event");
        assert!(message.contains("sandboxed-throwaway"), "{message}");
    }

    #[test]
    fn an_unknown_preset_name_is_reported_not_silently_ignored() {
        let (events, effect) = run("preset nonesuch", snapshot("default", CUSTOM_PRESET));

        assert!(effect.is_none(), "nothing may be applied for a bad name");
        let message = events
            .iter()
            .find_map(|event| match event {
                TuiEvent::Error(message) => Some(message.clone()),
                _ => None,
            })
            .expect("error event");
        assert!(message.contains("nonesuch"), "{message}");
    }

    #[test]
    fn a_permission_mode_still_reaches_the_mode_path() {
        // The preset branch sits ahead of the mode branch. If it ever claimed
        // a mode token, `/permissions plan` would stop setting a mode.
        let (_, effect) = run("plan", snapshot("default", CUSTOM_PRESET));

        assert!(matches!(
            effect,
            Some(CommandEffect::SetPermissionMode(ref mode)) if mode == "plan"
        ));
    }
}
