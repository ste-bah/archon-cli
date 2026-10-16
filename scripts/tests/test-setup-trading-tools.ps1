$ErrorActionPreference = 'Stop'
$scriptPath = (Resolve-Path 'scripts/setup-trading-tools.ps1').Path
. $scriptPath -LoadFunctionsOnly
$python = (Get-Command python -ErrorAction Stop).Source
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

$root = Join-Path $env:RUNNER_TEMP 'trading setup safety checks'
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
  mkdir "%3\src\cli" 2>nul
  mkdir "%3\scripts" 2>nul
  type nul > "%3\src\server.js"
  type nul > "%3\src\cli\index.js"
  echo {}> "%3\package.json"
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
$previousNpmLog = $env:FAKE_NPM_LOG
try {
    $env:Path = "$fakeBin$([IO.Path]::PathSeparator)$previousPath"
    $env:FAKE_NPM_LOG = $npmLog
    & $scriptPath -Target $fakeTarget -SkipOpenBB
    if ($LASTEXITCODE -ne 0) { throw "Spaced-path npm invocation exited $LASTEXITCODE" }
} finally {
    $env:Path = $previousPath
    if ($null -eq $previousNpmLog) { Remove-Item Env:FAKE_NPM_LOG -ErrorAction SilentlyContinue }
    else { $env:FAKE_NPM_LOG = $previousNpmLog }
}
$expectedTvDir = Join-Path $fakeTarget '.archon/tools/tradingview-mcp'
$npmInvocation = Get-Content -LiteralPath $npmLog
if ($npmInvocation.Count -lt 2 -or $npmInvocation[0] -ne $expectedTvDir -or $npmInvocation[1] -ne 'install --omit=dev') {
    throw "Spaced-path npm assertion failed; expected cwd '$expectedTvDir' and arguments 'install --omit=dev', got: $($npmInvocation -join ' | ')"
}
if (-not (Test-Path -LiteralPath (Join-Path $expectedTvDir 'package.json'))) { throw 'Spaced-path npm assertion failed: package.json was not created by the fake clone' }
Write-Output 'PASS npm install runs in the TradingView directory with spaces'
Write-Output 'test result: PowerShell specifier and refusal checks passed'
exit 0
