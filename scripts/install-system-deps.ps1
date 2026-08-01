# install-system-deps.ps1 - Windows dependency installer for archon-cli.
#
# The PowerShell counterpart to install-system-deps.sh, which is POSIX-only and
# exits 1 on Windows. Installs build deps, Strawberry Perl (required - see
# below), poppler PDF utilities (`pdftotext`, `pdfimages`, `pdftoppm`),
# Tesseract OCR, and video ingest helpers (`ffmpeg`, `ffprobe`, `yt-dlp`).
#
# Does NOT install whisper.cpp builds, Python RapidOCR/OpenCV packages, model
# files, provider credentials, or enable sandbox backends in config.toml.
#
# Usage:
#   scripts\install-system-deps.ps1                # install everything
#   scripts\install-system-deps.ps1 -DryRun        # show what would run
#   scripts\install-system-deps.ps1 -Check         # verify deps, change nothing
#   scripts\install-system-deps.ps1 -WithDocker
#   scripts\install-system-deps.ps1 -WithTradingTools
#
# Exit codes (matching install-system-deps.sh):
#   0   success, or all deps present in -Check mode
#   1   usage / unsupported host
#   2   missing dependency (in -Check mode)
#   3   package install failed

[CmdletBinding()]
param(
    [switch]$Check,
    [switch]$DryRun,
    [switch]$WithDocker,
    [switch]$WithTradingTools
)

$ErrorActionPreference = 'Stop'

if ($env:OS -ne 'Windows_NT') {
    Write-Error 'install-system-deps.ps1: Windows only. On Linux/macOS use scripts/install-system-deps.sh'
    exit 1
}

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    Write-Error @'
install-system-deps.ps1: winget not found.
Install "App Installer" from the Microsoft Store, or install the packages
listed by -DryRun by hand.
'@
    exit 1
}

# Package id -> every binary that proves it landed. All of them must be present
# to count as installed: poppler and ffmpeg each ship several, and a partial
# install would otherwise be skipped here while still failing -Check.
#
# Perl is deliberately absent from this table: a `perl` on PATH is not
# sufficient, see Test-BuildPerl.
$Packages = [ordered]@{
    'Rustlang.Rustup'                        = @('cargo')
    'Git.Git'                                = @('git')
    'Microsoft.VisualStudio.2022.BuildTools' = @()   # no binary on PATH to probe
    'StrawberryPerl.StrawberryPerl'          = @('perl')
    'oschwartz10612.Poppler'                 = @('pdftotext', 'pdfimages', 'pdftoppm')
    'UB-Mannheim.TesseractOCR'               = @('tesseract')
    'Gyan.FFmpeg'                            = @('ffmpeg', 'ffprobe')
    'yt-dlp.yt-dlp'                          = @('yt-dlp')
}

# Extra winget arguments for packages that need more than a bare install.
#
# Installing the Build Tools bootstrapper alone gets you the installer shell and
# no compiler: without a workload there is no MSVC toolchain, so linking fails
# later with no obvious connection to this step. `--override` selects the C++
# workload non-interactively, which is what the docs' "select Desktop
# development with C++ during install" means for anyone clicking through.
$PackageArgs = @{
    'Microsoft.VisualStudio.2022.BuildTools' =
        @('--override', '--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
}

if ($WithDocker)        { $Packages['Docker.DockerDesktop'] = @('docker') }
if ($WithTradingTools)  {
    $Packages['OpenJS.NodeJS']       = @('node', 'npm')
    $Packages['Python.Python.3.12']  = @('python')
}

# poppler ships three binaries and the PDF pipeline needs all of them:
#   pdftotext  - text-layer extraction
#   pdfimages  - embedded image extraction
#   pdftoppm   - page-render fallback for scanned PDFs
$RequiredBinaries = @(
    'cargo', 'git', 'pdftotext', 'pdfimages', 'pdftoppm',
    'tesseract', 'ffmpeg', 'ffprobe', 'yt-dlp'
)
if ($WithDocker)       { $RequiredBinaries += 'docker' }
if ($WithTradingTools) { $RequiredBinaries += @('node', 'npm', 'python') }

# Whether the Perl on PATH can actually configure vendored OpenSSL.
#
# `openssl` is pinned with the `vendored` feature, so OpenSSL is compiled from
# source and its ./Configure is a Perl program needing modules that Git for
# Windows' cut-down msys Perl does not ship. Merely finding `perl` is therefore
# not enough - Git's perl answers `perl --version` quite happily and then fails
# the build with "Can't locate Locale/Maketext/Simple.pm in @INC". This probes
# for the module itself, which is the condition that actually matters.
function Test-BuildPerl {
    $perl = Get-Command perl -ErrorAction SilentlyContinue
    if (-not $perl) { return [pscustomobject]@{ Ok = $false; Reason = 'no perl on PATH'; Path = $null } }

    & $perl.Source -MLocale::Maketext::Simple -e '1' 2>$null
    if ($LASTEXITCODE -eq 0) {
        return [pscustomobject]@{ Ok = $true; Reason = 'ok'; Path = $perl.Source }
    }
    return [pscustomobject]@{
        Ok     = $false
        Reason = 'perl lacks Locale::Maketext::Simple (this is Git for Windows'' msys perl, not Strawberry Perl)'
        Path   = $perl.Source
    }
}

function Write-PerlRemedy {
    param([string]$FoundAt)
    Write-Host ''
    Write-Host 'Perl cannot build vendored OpenSSL.' -ForegroundColor Yellow
    if ($FoundAt) { Write-Host "  found: $FoundAt" }
    Write-Host '  fix:   winget install --id StrawberryPerl.StrawberryPerl -e'
    Write-Host '  then, if Git''s perl still wins on PATH:'
    Write-Host '         setx PERL "C:\Strawberry\perl\bin\perl.exe"'
}

# ---------------------------------------------------------------------------
# -Check: verify only, change nothing
# ---------------------------------------------------------------------------
if ($Check) {
    $missing = @()
    foreach ($bin in $RequiredBinaries) {
        if (-not (Get-Command $bin -ErrorAction SilentlyContinue)) { $missing += $bin }
    }

    $perl = Test-BuildPerl
    if (-not $perl.Ok) { $missing += 'perl (usable for vendored OpenSSL)' }

    if ($missing.Count -gt 0) {
        Write-Host "install-system-deps.ps1: missing: $($missing -join ', ')" -ForegroundColor Red
        if (-not $perl.Ok) { Write-PerlRemedy -FoundAt $perl.Path }
        Write-Host ''
        Write-Host '  Run: scripts\install-system-deps.ps1'
        exit 2
    }

    Write-Host "install-system-deps.ps1: all required binaries present ($($RequiredBinaries -join ', '))"
    Write-Host "install-system-deps.ps1: perl usable for vendored OpenSSL ($($perl.Path))"
    exit 0
}

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------
Write-Host 'install-system-deps.ps1: installing Windows build and ingest dependencies'
Write-Host "install-system-deps.ps1: docker=$WithDocker trading-tools=$WithTradingTools"
Write-Host ''

$failed = @()
foreach ($id in $Packages.Keys) {
    $probes = $Packages[$id]
    if ($probes.Count -gt 0) {
        $absent = @($probes | Where-Object { -not (Get-Command $_ -ErrorAction SilentlyContinue) })
        if ($absent.Count -eq 0) {
            Write-Host "  present: $id ($($probes -join ', ') already on PATH)"
            continue
        }
        if ($absent.Count -lt $probes.Count) {
            Write-Host "  partial: $id (missing $($absent -join ', ')) - reinstalling" -ForegroundColor Yellow
        }
    }

    $extra = @()
    if ($PackageArgs.ContainsKey($id)) { $extra = $PackageArgs[$id] }

    if ($DryRun) {
        Write-Host "[dry-run] winget install --id $id -e --accept-package-agreements --accept-source-agreements $($extra -join ' ')"
        continue
    }

    Write-Host "+ winget install --id $id -e $($extra -join ' ')"
    winget install --id $id -e --accept-package-agreements --accept-source-agreements --disable-interactivity @extra
    # 0 = installed, -1978335189 = already installed. Anything else is a failure.
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne -1978335189) {
        Write-Host "  FAILED: $id (winget exit $LASTEXITCODE)" -ForegroundColor Red
        $failed += $id
    }
}

if ($DryRun) {
    Write-Host ''
    Write-Host 'install-system-deps.ps1: dry run, nothing changed.'
    exit 0
}

if ($failed.Count -gt 0) {
    Write-Host ''
    Write-Host "install-system-deps.ps1: package install failed: $($failed -join ', ')" -ForegroundColor Red
    exit 3
}

# ---------------------------------------------------------------------------
# Post-install verification
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host 'install-system-deps.ps1: verifying installs...'
Write-Host 'Note: a new shell may be needed before freshly installed binaries appear on PATH.'

foreach ($bin in $RequiredBinaries) {
    $cmd = Get-Command $bin -ErrorAction SilentlyContinue
    if ($cmd) {
        Write-Host "  ok: $bin  ($($cmd.Source))"
    } else {
        Write-Host "  MISSING: $bin (post-install check failed; try a new shell)" -ForegroundColor Yellow
    }
}

$perl = Test-BuildPerl
if ($perl.Ok) {
    Write-Host "  ok: perl  ($($perl.Path)) - can build vendored OpenSSL"
} else {
    Write-Host "  PROBLEM: $($perl.Reason)" -ForegroundColor Yellow
    Write-PerlRemedy -FoundAt $perl.Path
}

Write-Host ''
Write-Host 'install-system-deps.ps1: done. Next steps:'
Write-Host '  1. Open a new shell so PATH changes take effect'
Write-Host '  2. Build archon-cli: cargo build --release --bin archon'
Write-Host '     (rust-toolchain.toml pins the toolchain; rustup fetches it on first build)'
Write-Host '  3. Initialise a project: scripts\archon-init.sh --target <path>   (via Git Bash or WSL)'
Write-Host '  4. Verify: scripts\install-system-deps.ps1 -Check'
if ($WithDocker) {
    Write-Host '  5. Enable Docker sandboxing with [sandbox].backend="docker" and [sandbox.docker].enabled=true'
}
