$ErrorActionPreference = 'Stop'
trap {
    Write-Output "FAIL: PowerShell specifier and refusal checks: $($_.Exception.Message)"
    exit 1
}
$scriptPath = (Resolve-Path 'scripts/setup-trading-tools.ps1').Path
. $scriptPath -LoadFunctionsOnly
$python = (Get-Command python -ErrorAction Stop).Source
$externalConsoleOutput = [IO.StringWriter]::new()
$originalConsoleOut = [Console]::Out
try {
    [Console]::SetOut($externalConsoleOutput)
    try {
        Invoke-External $python @('-c', "import sys; print('external stderr marker', file=sys.stderr); sys.exit(7)") | Out-Null
        throw 'External command assertion failed: nonzero exit was accepted'
    } catch {
        if ($_.Exception.Message -notmatch 'Command failed with exit 7') { throw }
    }
} finally { [Console]::SetOut($originalConsoleOut) }
if ($externalConsoleOutput.ToString() -notmatch '(?m)^external stderr marker\r?$') {
    throw "External command assertion failed: stderr was not written before failure: $($externalConsoleOutput.ToString())"
}
Write-Output 'PASS failing external command stderr reaches console output'
$cases = @(
    @{ Specifier = '>=3.10,<3.13'; Version = '3.12'; Expected = $true },
    @{ Specifier = '>=3.10,<3.13'; Version = '3.13'; Expected = $false },
    @{ Specifier = '>=3.11.17'; Version = '3.11.0'; Expected = $false },
    @{ Specifier = '>=3.11.17'; Version = '3.11.17'; Expected = $true },
    @{ Specifier = '!=3.12.*'; Version = '3.12.4'; Expected = $false },
    @{ Specifier = '>=3.13'; Version = '3.13.0rc1'; Expected = $false },
    @{ Specifier = '>=3.13rc1'; Version = '3.13.0rc2'; Expected = $true },
    @{ Specifier = '>=3.13rc1'; Version = '3.13.0b1'; Expected = $false }
)
foreach ($case in $cases) {
    $actual = Test-PythonCandidate $python @() $case.Specifier $case.Version
    if ($actual -ne $case.Expected) {
        throw "Specifier assertion failed: '$($case.Specifier)' with Python $($case.Version); expected $($case.Expected), got $actual"
    }
    Write-Output "PASS specifier '$($case.Specifier)' with Python $($case.Version) => $actual"
}

# Get-PythonCandidates must parse both py.exe -0p listing formats. The fake
# launcher reports real fake interpreter paths so this exercises discovery.
$root = Join-Path $env:RUNNER_TEMP 'trading setup safety checks'
$candidateRoot = Join-Path $root 'py launcher candidates'
$candidateBin = Join-Path $candidateRoot 'bin'
New-Item -ItemType Directory -Force -Path $candidateBin | Out-Null
$python315 = Join-Path $candidateBin 'python315.cmd'
$python312 = Join-Path $candidateBin 'python312.cmd'
Set-Content -LiteralPath $python315 -Value @('@echo off', 'echo Python 3.15.0')
Set-Content -LiteralPath $python312 -Value @('@echo off', 'echo Python 3.12.0')
$pyLauncher = Join-Path $candidateBin 'py.cmd'
Set-Content -LiteralPath $pyLauncher -Value @(
    '@echo off',
    "echo  -V:3.15 *        $python315",
    "echo  -3.12-64        $python312"
)
$savedCandidatePath = $env:Path
$savedCandidatePathExt = $env:PATHEXT
try {
    $env:Path = "$candidateBin$([IO.Path]::PathSeparator)$savedCandidatePath"
    $env:PATHEXT = '.CMD;.BAT;.EXE;.COM'
    $foundCandidates = @(Get-PythonCandidates)
} finally {
    $env:Path = $savedCandidatePath
    $env:PATHEXT = $savedCandidatePathExt
}
if (-not ($foundCandidates.Program -contains $python315) -or -not ($foundCandidates.Program -contains $python312)) {
    throw "py.exe candidate parsing failed; expected both launcher formats, got: $($foundCandidates.Program -join ', ')"
}
Write-Output 'PASS py.exe candidate discovery parses -V and architecture launcher formats'

$candidateBin = Join-Path $root 'wheel probe candidates'
New-Item -ItemType Directory -Force -Path $candidateBin | Out-Null
$failingCandidate = Join-Path $candidateBin 'newer-python.cmd'
$workingCandidate = Join-Path $candidateBin 'older-python.cmd'
Set-Content -LiteralPath $failingCandidate -Value @('@echo off', 'echo no wheel for dependency >&2', 'echo complete resolver diagnostic >&2', 'exit /b 1')
Set-Content -LiteralPath $workingCandidate -Value @('@echo off', 'exit /b 0')
$candidates = @(
    @{ Program = $failingCandidate; FixedArguments = [string[]]@() },
    @{ Program = $workingCandidate; FixedArguments = [string[]]@() }
)
$selected = Resolve-BasePython '>=3.10,<4' $candidates
if ($selected.Program -ne $workingCandidate) { throw "Wheel resolution selected '$($selected.Program)' instead of '$workingCandidate'" }
try {
    Resolve-BasePython '>=3.10,<4' @($candidates[0]) | Out-Null
    throw 'Wheel resolution assertion failed: all failed candidates were accepted'
} catch {
    if ($_.Exception.Message -notmatch 'no wheel for dependency' -or $_.Exception.Message -notmatch 'complete resolver diagnostic') { throw }
}
Write-Output 'PASS wheel-only resolution skips a failing candidate and retains its full pip error'

$junctionTarget = Join-Path $root 'junction target'
$junctionProject = Join-Path $root 'junction project'
$junctionVenv = Join-Path $junctionProject '.archon/tools/openbb-venv'
New-Item -ItemType Directory -Force -Path $junctionTarget, (Split-Path -Parent $junctionVenv) | Out-Null
$junctionSentinel = Join-Path $junctionTarget 'sentinel.txt'
Set-Content -LiteralPath $junctionSentinel -Value 'preserve'
New-Item -ItemType Junction -Path $junctionVenv -Target $junctionTarget | Out-Null
try {
    & $scriptPath -Target $junctionProject -SkipTradingView
    throw 'Refusal assertion failed: junctioned venv was accepted'
} catch {
    if ($_.Exception.Message -notmatch 'reparse-point OpenBB venv') { throw }
}
if ((Get-Content -LiteralPath $junctionSentinel -Raw).Trim() -ne 'preserve') {
    throw 'Refusal assertion failed: junction target sentinel changed'
}
Write-Output 'PASS junctioned venv refused with sentinel intact'

$nonVenvProject = Join-Path $root 'non-venv project'
$nonVenv = Join-Path $nonVenvProject '.archon/tools/openbb-venv'
New-Item -ItemType Directory -Force -Path $nonVenv | Out-Null
$nonVenvSentinel = Join-Path $nonVenv 'sentinel.txt'
Set-Content -LiteralPath $nonVenvSentinel -Value 'preserve'
try {
    & $scriptPath -Target $nonVenvProject -SkipTradingView
    throw 'Refusal assertion failed: non-venv directory with contents was accepted'
} catch {
    if ($_.Exception.Message -notmatch 'non-venv directory with contents') { throw }
}
if ((Get-Content -LiteralPath $nonVenvSentinel -Raw).Trim() -ne 'preserve') {
    throw 'Refusal assertion failed: non-venv sentinel changed'
}
Write-Output 'PASS non-venv directory with contents refused with sentinel intact'

$fakeBin = Join-Path $root 'fake npm bin'
$fakeTarget = Join-Path $root 'TradingView target with spaces'
$npmLog = Join-Path $root 'npm invocation.log'
New-Item -ItemType Directory -Force -Path $fakeBin, $fakeTarget | Out-Null
$gitShim = @'
@echo off
if "%1"=="clone" (
  if defined FAKE_GIT_NO_CREATE exit /b 0
  mkdir "%~3\src\cli" 2>nul
  mkdir "%~3\scripts" 2>nul
  type nul > "%~3\src\server.js"
  type nul > "%~3\src\cli\index.js"
  if not defined FAKE_GIT_NO_PACKAGE echo {}> "%~3\package.json"
)
exit /b 0
'@
$npmShim = @'
@echo off
if "%1"=="install" (
  echo %CD%> "%FAKE_NPM_LOG%"
  echo %*>> "%FAKE_NPM_LOG%"
  exit /b 0
)
echo 10.0.0
exit /b 0
'@
Set-Content -LiteralPath (Join-Path $fakeBin 'git.cmd') -Value $gitShim
Set-Content -LiteralPath (Join-Path $fakeBin 'npm.cmd') -Value $npmShim
Set-Content -LiteralPath (Join-Path $fakeBin 'node.cmd') -Value @('@echo off', 'exit /b 0')
$previousPath = $env:Path
$previousPathExt = $env:PATHEXT
$previousNpmLog = $env:FAKE_NPM_LOG
$previousNoCreate = $env:FAKE_GIT_NO_CREATE
$previousNoPackage = $env:FAKE_GIT_NO_PACKAGE
try {
    $env:Path = "$fakeBin$([IO.Path]::PathSeparator)$previousPath"
    $env:PATHEXT = '.CMD;.BAT;.EXE;.COM'
    $env:FAKE_NPM_LOG = $npmLog
    $resolvedGit = Get-Command git -ErrorAction Stop
    if ($resolvedGit.Source -notlike "$fakeBin*") {
        throw "Mock git was not selected from '$fakeBin': $($resolvedGit.Source)"
    }
    & $scriptPath -Target $fakeTarget -SkipOpenBB
    if ($LASTEXITCODE -ne 0) { throw "Spaced-path npm invocation exited $LASTEXITCODE" }
} finally {
    $env:Path = $previousPath
    $env:PATHEXT = $previousPathExt
    if ($null -eq $previousNpmLog) { Remove-Item Env:FAKE_NPM_LOG -ErrorAction SilentlyContinue }
    else { $env:FAKE_NPM_LOG = $previousNpmLog }
    if ($null -eq $previousNoCreate) { Remove-Item Env:FAKE_GIT_NO_CREATE -ErrorAction SilentlyContinue }
    else { $env:FAKE_GIT_NO_CREATE = $previousNoCreate }
    if ($null -eq $previousNoPackage) { Remove-Item Env:FAKE_GIT_NO_PACKAGE -ErrorAction SilentlyContinue }
    else { $env:FAKE_GIT_NO_PACKAGE = $previousNoPackage }
}
$expectedTvDir = Join-Path $fakeTarget '.archon/tools/tradingview-mcp'
$npmInvocation = Get-Content -LiteralPath $npmLog
if ($npmInvocation.Count -lt 2 -or $npmInvocation[0] -ne $expectedTvDir -or $npmInvocation[1] -ne 'install --omit=dev') {
    throw "Spaced-path npm assertion failed; expected cwd '$expectedTvDir' and arguments 'install --omit=dev', got: $($npmInvocation -join ' | ')"
}
if (-not (Test-Path -LiteralPath (Join-Path $expectedTvDir 'package.json'))) { throw 'Spaced-path npm assertion failed: package.json was not created by the fake clone' }
Write-Output 'PASS npm install runs in the TradingView directory with spaces'

$missingTarget = Join-Path $root 'missing clone target with spaces'
$missingPackageTarget = Join-Path $root 'missing package target with spaces'
foreach ($missingCase in @(
    @{ Target = $missingTarget; Variable = 'FAKE_GIT_NO_CREATE'; Message = 'TradingView directory is missing after git clone or pull:' },
    @{ Target = $missingPackageTarget; Variable = 'FAKE_GIT_NO_PACKAGE'; Message = 'TradingView package.json is missing after git clone or pull:' }
)) {
    $env:Path = "$fakeBin$([IO.Path]::PathSeparator)$previousPath"
    $env:PATHEXT = '.CMD;.BAT;.EXE;.COM'
    $env:FAKE_GIT_NO_CREATE = $null
    $env:FAKE_GIT_NO_PACKAGE = $null
    Set-Item -Path "Env:$($missingCase.Variable)" -Value '1'
    try {
        & $scriptPath -Target $missingCase.Target -SkipOpenBB 2>&1 | Out-String
        throw "Missing clone assertion failed: setup accepted $($missingCase.Variable)"
    } catch {
        $expectedMissingDir = Join-Path $missingCase.Target '.archon/tools/tradingview-mcp'
        if ($_.Exception.Message -notlike "*$($missingCase.Message)*$expectedMissingDir*") { throw }
    } finally {
        Remove-Item -Path "Env:$($missingCase.Variable)" -ErrorAction SilentlyContinue
        $env:Path = $previousPath
        $env:PATHEXT = $previousPathExt
    }
}
Write-Output 'PASS missing TradingView checkout content is reported clearly'
Write-Output 'test result: PowerShell specifier and refusal checks passed'
exit 0
