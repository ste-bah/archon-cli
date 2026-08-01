#!/usr/bin/env sh
# Install an Archon plugin bundle from this collection.
#
# Usage:
#   install.sh [--user] [--no-hooks] <plugin>|all [project_dir]
#
#   --user       user-global install instead of project-local (default: cwd)
#   --no-hooks   skip enabling the plugin's hooks (they are enabled by default,
#                matching Claude Code plugin behavior; snippet printed instead)
#
# Skills   -> <project>/.archon/skills/<name>/            or  $XDG_CONFIG_HOME/archon/skills/<name>/
# Agents   -> <project>/.archon/plugins/<plugin>/agents/  or  ~/.archon/plugins/<plugin>/agents/
# Scripts  -> <project>/.archon/plugins/<plugin>/scripts/ or  ~/.archon/plugins/<plugin>/scripts/
# Hooks    -> merged into .archon/settings.json (requires node; --no-hooks to skip)

set -eu

COLLECTION_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

SCOPE=project
HOOKS=yes
while [ $# -gt 0 ]; do
    case "$1" in
        --user) SCOPE=user; shift ;;
        --no-hooks) HOOKS=no; shift ;;
        *) break ;;
    esac
done

PLUGIN=${1:-}
if [ -z "$PLUGIN" ]; then
    echo "usage: install.sh [--user] [--no-hooks] <plugin>|all [project_dir]" >&2
    exit 1
fi

if [ "$SCOPE" = "user" ]; then
    PLUGINS_ROOT="$HOME/.archon/plugins"
    SKILLS_ROOT="${XDG_CONFIG_HOME:-$HOME/.config}/archon/skills"
    SETTINGS_FILE="$HOME/.archon/settings.json"
else
    PROJECT_DIR=${2:-$(pwd)}
    if [ ! -d "$PROJECT_DIR" ]; then
        echo "error: project dir not found: $PROJECT_DIR" >&2
        exit 1
    fi
    PLUGINS_ROOT="$PROJECT_DIR/.archon/plugins"
    SKILLS_ROOT="$PROJECT_DIR/.archon/skills"
    SETTINGS_FILE="$PROJECT_DIR/.archon/settings.json"
fi

install_one() {
    name=$1
    src="$COLLECTION_DIR/$name"
    if [ ! -d "$src" ]; then
        echo "error: no such plugin: $name" >&2
        exit 1
    fi
    echo "Installing $name ($SCOPE)"

    if [ -d "$src/agents" ]; then
        mkdir -p "$PLUGINS_ROOT/$name"
        cp -R "$src/agents" "$PLUGINS_ROOT/$name/"
        echo "  agents  -> $PLUGINS_ROOT/$name/agents/"
    fi

    if [ -d "$src/scripts" ]; then
        mkdir -p "$PLUGINS_ROOT/$name"
        cp -R "$src/scripts" "$PLUGINS_ROOT/$name/"
        chmod +x "$PLUGINS_ROOT/$name/scripts"/*.sh 2>/dev/null || true
        echo "  scripts -> $PLUGINS_ROOT/$name/scripts/"
    fi

    if [ -d "$src/skills" ]; then
        mkdir -p "$SKILLS_ROOT"
        for skill in "$src/skills"/*/; do
            [ -d "$skill" ] || continue
            cp -R "$skill" "$SKILLS_ROOT/"
            echo "  skill   -> $SKILLS_ROOT/$(basename "$skill")/"
        done
    fi

    if [ -f "$src/hooks/settings.snippet.json" ]; then
        if [ "$HOOKS" = "yes" ] && command -v node >/dev/null 2>&1; then
            mkdir -p "$(dirname "$SETTINGS_FILE")"
            node "$COLLECTION_DIR/merge-hooks.js" "$SETTINGS_FILE" "$src/hooks/settings.snippet.json"
            echo "  hooks   -> enabled in $SETTINGS_FILE (re-run with --no-hooks to skip)"
        else
            echo ""
            if [ "$HOOKS" = "yes" ]; then
                echo "  node not found — hooks NOT enabled. Merge this into $SETTINGS_FILE manually:"
            else
                echo "  --no-hooks: hooks NOT enabled. To enable later, merge this into $SETTINGS_FILE:"
            fi
            echo "  --- $src/hooks/settings.snippet.json ---"
            cat "$src/hooks/settings.snippet.json"
            echo "  ---"
        fi
    fi
}

if [ "$PLUGIN" = "all" ]; then
    for dir in "$COLLECTION_DIR"/*/; do
        [ -f "$dir/README.md" ] || [ -d "$dir/skills" ] || [ -d "$dir/agents" ] || continue
        install_one "$(basename "$dir")"
    done
else
    install_one "$PLUGIN"
fi

echo ""
echo "Done. Restart archon to pick up new skills and agents."
