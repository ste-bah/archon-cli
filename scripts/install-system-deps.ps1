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
#   scripts\install-system-deps.ps1 -WithOcr        # RapidOCR image OCR
#   scripts\install-system-deps.ps1 -WithJava       # JDK, Gradle, Maven
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
    [switch]$WithTradingTools,
    [switch]$WithOcr,
    [switch]$WithJava
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
#
# Temurin's MSI leaves JAVA_HOME unset under its default feature selection. Maven
# reads JAVA_HOME directly and fails without it, so the two optional features
# that set the variable and extend PATH are requested explicitly. `--custom`
# appends to the msiexec command line; `--override` would replace winget's own
# silent-install arguments and is wrong here.
$PackageArgs = @{
    'Microsoft.VisualStudio.2022.BuildTools' =
        @('--override', '--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
    'EclipseAdoptium.Temurin.25.JDK' =
        @('--custom', 'ADDLOCAL=FeatureMain,FeatureEnvironment,FeatureJavaHome')
}

if ($WithDocker)        { $Packages['Docker.DockerDesktop'] = @('docker') }
if ($WithTradingTools)  {
    $Packages['OpenJS.NodeJS']       = @('node', 'npm')
    $Packages['Python.Python.3.12']  = @('python')
}
# RapidOCR needs an interpreter to build its virtualenv from.
if ($WithOcr) { $Packages['Python.Python.3.12'] = @('python') }
# Java toolchain. `javac` as well as `java` is probed deliberately: a JRE
# satisfies `java` and cannot compile anything, which is the failure this
# would otherwise hide until the first build.
#
# Only the JDK comes from winget: neither Gradle nor Maven is in the winget
# repository at all (searching either name returns unrelated packages), so both
# are installed from their projects' official archives by Install-JavaBuildTools
# below.
#
# Temurin 25 rather than 26: 26 is a six-month feature release that goes out of
# support when 27 ships, so pinning it means pinning something already at the
# end of its life. 25 is the current LTS. Gradle 9 runs on 17-26, so both would
# work — this is about how long the pin stays good, not compatibility.
#
# Note this JDK's job is to RUN Gradle and Maven. What a project compiles
# against is that project's own choice, through Gradle toolchains or
# maven.compiler.release, and is not constrained by the version here.
if ($WithJava) {
    $Packages['EclipseAdoptium.Temurin.25.JDK'] = @('java', 'javac')
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
if ($WithOcr -and -not $WithTradingTools) { $RequiredBinaries += 'python' }
if ($WithJava)         { $RequiredBinaries += @('java', 'javac', 'gradle', 'mvn') }

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

# ---------------------------------------------------------------------------
# Gradle and Maven — official archives, not winget
# ---------------------------------------------------------------------------
#
# Neither tool is packaged in winget. Both publish a versioned zip with a
# published checksum beside it, which is what is used here.
#
# Everything lands under %LOCALAPPDATA%\Programs and the PATH entries are
# written to the *user* environment, so no part of this needs elevation — which
# also means no UAC prompt can block a non-interactive run.
#
# The checksum comes from the same host as the archive, so it guards against a
# truncated or corrupted download rather than against a compromised origin. It
# is the strongest check either project publishes.

# Maven has no version-metadata endpoint that also serves a checksum, so the
# version is pinned. It is the version maven.apache.org/download.cgi names as
# "the recommended version for all users"; bumping it is this one line.
# archive.apache.org rather than dlcdn.apache.org: the archive is permanent,
# while the mirror drops releases as they age out and would start 404ing.
$MavenVersion = '3.9.16'
$MavenBaseUrl = "https://archive.apache.org/dist/maven/maven-3/$MavenVersion/binaries"

$JavaToolsRoot = Join-Path $env:LOCALAPPDATA 'Programs'

function Install-ZipDistribution {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Url,
        [Parameter(Mandatory)][string]$ExpectedHash,
        [Parameter(Mandatory)][ValidateSet('SHA256', 'SHA512')][string]$Algorithm,
        [Parameter(Mandatory)][string]$BinDir
    )

    if (Test-Path $BinDir) {
        Write-Host "  present: $Name ($BinDir)"
        return $true
    }

    $zip = Join-Path ([System.IO.Path]::GetTempPath()) "archon-$Name.zip"
    Write-Host "+ downloading $Name from $Url"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $zip -UseBasicParsing
    } catch {
        Write-Host "  FAILED: could not download $Name : $_" -ForegroundColor Red
        return $false
    }

    # Verified before extraction, never after: an archive that fails the check
    # is not written anywhere but the temp file it arrived in.
    $actual = (Get-FileHash -Path $zip -Algorithm $Algorithm).Hash
    if ($actual -ne $ExpectedHash.Trim().ToUpperInvariant()) {
        Write-Host "  FAILED: $Name checksum mismatch - refusing to install" -ForegroundColor Red
        Write-Host "    expected: $ExpectedHash"
        Write-Host "    actual:   $actual"
        Remove-Item $zip -Force -ErrorAction SilentlyContinue
        return $false
    }

    try {
        if (-not (Test-Path $JavaToolsRoot)) {
            New-Item -ItemType Directory -Force -Path $JavaToolsRoot | Out-Null
        }
        Expand-Archive -Path $zip -DestinationPath $JavaToolsRoot -Force
    } catch {
        Write-Host "  FAILED: could not unpack $Name : $_" -ForegroundColor Red
        Remove-Item $zip -Force -ErrorAction SilentlyContinue
        return $false
    }
    Remove-Item $zip -Force -ErrorAction SilentlyContinue
    return (Test-Path $BinDir)
}

# Append to the *user* PATH, persistently, without duplicating an existing entry.
function Add-UserPathEntry {
    param([Parameter(Mandatory)][string]$Directory)

    $current = [Environment]::GetEnvironmentVariable('PATH', 'User')
    if (-not $current) { $current = '' }
    $entries = $current -split ';' | Where-Object { $_ -ne '' }
    if ($entries -contains $Directory) {
        Write-Host "  PATH already contains $Directory"
        return
    }
    $updated = (@($entries) + $Directory) -join ';'
    [Environment]::SetEnvironmentVariable('PATH', $updated, 'User')
    # Also make it usable in THIS process, so the verification pass below sees it.
    $env:PATH = "$env:PATH;$Directory"
    Write-Host "  added to user PATH: $Directory"
}

function Install-JavaBuildTools {
    if (-not $WithJava) { return }

    Write-Host ''
    Write-Host 'install-system-deps.ps1: installing Gradle and Maven from official archives'

    if ($DryRun) {
        Write-Host '[dry-run] resolve https://services.gradle.org/versions/current, verify SHA-256, unpack Gradle to' $JavaToolsRoot
        Write-Host "[dry-run] download $MavenBaseUrl/apache-maven-$MavenVersion-bin.zip, verify SHA-512, unpack to $JavaToolsRoot"
        Write-Host '[dry-run] append both bin directories to the user PATH'
        return
    }

    # --- Gradle -------------------------------------------------------------
    # services.gradle.org/versions/current returns the current version, its
    # download URL and its SHA-256 in one document, so nothing is pinned here.
    try {
        $meta = Invoke-RestMethod -Uri 'https://services.gradle.org/versions/current' -UseBasicParsing
    } catch {
        Write-Host "  FAILED: could not resolve the current Gradle version: $_" -ForegroundColor Red
        $meta = $null
    }
    if ($meta -and $meta.version -and $meta.downloadUrl -and $meta.checksum) {
        $gradleBin = Join-Path $JavaToolsRoot "gradle-$($meta.version)\bin"
        $ok = Install-ZipDistribution -Name "gradle-$($meta.version)" -Url $meta.downloadUrl `
            -ExpectedHash $meta.checksum -Algorithm SHA256 -BinDir $gradleBin
        if ($ok) { Add-UserPathEntry -Directory $gradleBin }
    } elseif ($meta) {
        Write-Host '  FAILED: Gradle version metadata was missing expected fields' -ForegroundColor Red
    }

    # --- Maven --------------------------------------------------------------
    $mavenZipUrl = "$MavenBaseUrl/apache-maven-$MavenVersion-bin.zip"
    try {
        $mavenHash = (Invoke-WebRequest -Uri "$mavenZipUrl.sha512" -UseBasicParsing).Content
    } catch {
        Write-Host "  FAILED: could not fetch the Maven checksum: $_" -ForegroundColor Red
        $mavenHash = $null
    }
    if ($mavenHash) {
        $mavenBin = Join-Path $JavaToolsRoot "apache-maven-$MavenVersion\bin"
        $ok = Install-ZipDistribution -Name "apache-maven-$MavenVersion" -Url $mavenZipUrl `
            -ExpectedHash $mavenHash -Algorithm SHA512 -BinDir $mavenBin
        if ($ok) { Add-UserPathEntry -Directory $mavenBin }
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
    if ($WithJava -and -not $env:JAVA_HOME) {
        Write-Host 'install-system-deps.ps1: note - JAVA_HOME is unset. Gradle finds a JDK on PATH, but Maven reads JAVA_HOME.' -ForegroundColor Yellow
    }
    exit 0
}

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------
Write-Host 'install-system-deps.ps1: installing Windows build and ingest dependencies'
Write-Host "install-system-deps.ps1: docker=$WithDocker trading-tools=$WithTradingTools ocr=$WithOcr java=$WithJava"
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

Install-JavaBuildTools

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
# RapidOCR helper virtualenv
# ---------------------------------------------------------------------------
#
# `$env:USERPROFILE\.archon-marker-venv` is the path archon itself probes for a
# RapidOCR interpreter (crates/archon-docs/src/ocr/rapid.rs), so creating it
# here is what makes image OCR work with no environment variable and no config.
#
# Note the layout difference from the POSIX installer: a virtualenv puts its
# interpreter in Scripts\python.exe on Windows, not bin/python.
$MarkerVenv = Join-Path $env:USERPROFILE '.archon-marker-venv'
$MarkerPython = Join-Path $MarkerVenv 'Scripts\python.exe'

if ($WithOcr) {
    Write-Host ''
    Write-Host 'install-system-deps.ps1: setting up the RapidOCR virtualenv'
    $py = Get-Command python -ErrorAction SilentlyContinue
    if (-not $py) {
        Write-Host '  SKIPPED: python not on PATH yet - open a new shell and re-run with -WithOcr' -ForegroundColor Yellow
    } else {
        if (Test-Path $MarkerPython) {
            Write-Host "  present: $MarkerVenv"
        } else {
            Write-Host "+ python -m venv $MarkerVenv"
            & $py.Source -m venv $MarkerVenv
        }
        if (Test-Path $MarkerPython) {
            Write-Host "+ $MarkerPython -m pip install --upgrade pip"
            & $MarkerPython -m pip install --upgrade pip
            # opencv-python-headless, not opencv-python: this venv never opens a
            # window and the GUI build drags in a much larger dependency set.
            Write-Host "+ $MarkerPython -m pip install rapidocr opencv-python-headless"
            & $MarkerPython -m pip install rapidocr opencv-python-headless
        } else {
            Write-Host "  FAILED: could not create $MarkerVenv" -ForegroundColor Red
        }
    }
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

if ($WithOcr) {
    $rapidOk = $false
    if (Test-Path $MarkerPython) {
        # A separate statement rather than an `if` condition: PowerShell cannot
        # read $LASTEXITCODE inside the same expression that produced it.
        & $MarkerPython -c 'import rapidocr' 2>$null
        $rapidOk = ($LASTEXITCODE -eq 0)
    }
    if ($rapidOk) {
        Write-Host "  ok: rapidocr  ($MarkerVenv)"
    } else {
        Write-Host "  MISSING: rapidocr venv at $MarkerVenv (post-install check failed)" -ForegroundColor Yellow
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
if ($WithOcr) {
    Write-Host "     RapidOCR image OCR is installed at $MarkerVenv and needs no further configuration"
} else {
    Write-Host '     Optional RapidOCR image-OCR fallback: re-run with -WithOcr'
}
if ($WithJava) {
    Write-Host '     Java analysis needs no further system packages: Checkstyle, PMD, SpotBugs,'
    Write-Host "     FindSecBugs, Error Prone and PIT are declared by the project's own build."
    if (-not $env:JAVA_HOME) {
        Write-Host '     JAVA_HOME is not set in THIS shell yet - open a new one before running Maven.' -ForegroundColor Yellow
    }
} else {
    Write-Host '     Optional Java toolchain (JDK, Gradle, Maven): re-run with -WithJava'
}
if ($WithDocker) {
    Write-Host '  5. Enable Docker sandboxing with [sandbox].backend="docker" and [sandbox.docker].enabled=true'
}
