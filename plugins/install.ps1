# Install an Archon plugin bundle from this collection.
#
# Usage:
#   .\install.ps1 <plugin>|all [-ProjectDir <path>]   # project-local install (default: cwd)
#   .\install.ps1 <plugin>|all -User                  # user-global install
#   .\install.ps1 <plugin> -NoHooks                   # skip enabling the plugin's hooks
#
# Skills   -> <project>\.archon\skills\<name>\             or  %APPDATA%\archon\skills\<name>\
# Agents   -> <project>\.archon\plugins\<plugin>\agents\   or  ~\.archon\plugins\<plugin>\agents\
# Scripts  -> <project>\.archon\plugins\<plugin>\scripts\  or  ~\.archon\plugins\<plugin>\scripts\
# Hooks    -> merged into .archon\settings.json by default (requires node; -NoHooks to skip)

param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Plugin,

    [string]$ProjectDir = (Get-Location).Path,

    [switch]$User,

    [switch]$NoHooks
)

$ErrorActionPreference = 'Stop'
$collectionDir = Split-Path -Parent $MyInvocation.MyCommand.Path

if ($User) {
    $pluginsRoot = Join-Path $HOME '.archon\plugins'
    $skillsRoot = Join-Path $env:APPDATA 'archon\skills'
    $settingsFile = Join-Path $HOME '.archon\settings.json'
} else {
    if (-not (Test-Path $ProjectDir)) {
        throw "Project dir not found: $ProjectDir"
    }
    $pluginsRoot = Join-Path $ProjectDir '.archon\plugins'
    $skillsRoot = Join-Path $ProjectDir '.archon\skills'
    $settingsFile = Join-Path $ProjectDir '.archon\settings.json'
}

function Install-One {
    param([string]$Name)

    $src = Join-Path $collectionDir $Name
    if (-not (Test-Path $src)) {
        throw "No such plugin: $Name"
    }
    $scopeLabel = 'project'
    if ($User) { $scopeLabel = 'user' }
    Write-Host "Installing $Name ($scopeLabel)"

    $agentsSrc = Join-Path $src 'agents'
    if (Test-Path $agentsSrc) {
        $dest = Join-Path $pluginsRoot $Name
        New-Item -ItemType Directory -Force $dest | Out-Null
        Copy-Item -Recurse -Force $agentsSrc $dest
        Write-Host "  agents  -> $dest\agents\"
    }

    $scriptsSrc = Join-Path $src 'scripts'
    if (Test-Path $scriptsSrc) {
        $dest = Join-Path $pluginsRoot $Name
        New-Item -ItemType Directory -Force $dest | Out-Null
        Copy-Item -Recurse -Force $scriptsSrc $dest
        Write-Host "  scripts -> $dest\scripts\"
    }

    $skillsSrc = Join-Path $src 'skills'
    if (Test-Path $skillsSrc) {
        New-Item -ItemType Directory -Force $skillsRoot | Out-Null
        Get-ChildItem -Directory $skillsSrc | ForEach-Object {
            Copy-Item -Recurse -Force $_.FullName $skillsRoot
            Write-Host "  skill   -> $skillsRoot\$($_.Name)\"
        }
    }

    $hooksSnippet = Join-Path $src 'hooks\settings.snippet.json'
    if (Test-Path $hooksSnippet) {
        $node = Get-Command node -ErrorAction SilentlyContinue
        if ((-not $NoHooks) -and $node) {
            $settingsDir = Split-Path -Parent $settingsFile
            New-Item -ItemType Directory -Force $settingsDir | Out-Null
            & node (Join-Path $collectionDir 'merge-hooks.js') $settingsFile $hooksSnippet
            Write-Host "  hooks   -> enabled in $settingsFile (use -NoHooks to skip)"
        } else {
            Write-Host ""
            if ($NoHooks) {
                Write-Host "  -NoHooks: hooks NOT enabled. To enable later, merge this into ${settingsFile}:"
            } else {
                Write-Host "  node not found - hooks NOT enabled. Merge this into $settingsFile manually:"
            }
            Write-Host "  --- $hooksSnippet ---"
            Get-Content $hooksSnippet | Write-Host
            Write-Host "  ---"
        }
    }
}

if ($Plugin -eq 'all') {
    Get-ChildItem -Directory $collectionDir | ForEach-Object {
        $hasContent = (Test-Path (Join-Path $_.FullName 'skills')) -or (Test-Path (Join-Path $_.FullName 'agents'))
        if ($hasContent) { Install-One $_.Name }
    }
} else {
    Install-One $Plugin
}

Write-Host ""
Write-Host "Done. Restart archon to pick up new skills and agents."
