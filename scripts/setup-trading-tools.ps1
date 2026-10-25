param(
    [string]$Target = (Get-Location).Path,
    [switch]$Check,
    [switch]$DryRun,
    [switch]$SkipTradingView,
    [switch]$SkipOpenBB,
    [switch]$LoadFunctionsOnly
)
$ErrorActionPreference = 'Stop'
$Target = [IO.Path]::GetFullPath($Target)
$ToolsDir = Join-Path $Target '.archon/tools'
$TvDir = Join-Path $ToolsDir 'tradingview-mcp'
$Venv = Join-Path $ToolsDir 'openbb-venv'
$Python = Join-Path $Venv 'Scripts/python.exe'
$Api = Join-Path $Venv 'Scripts/openbb-api.exe'
$McpFile = Join-Path $Target '.mcp.json'
function Invoke-PythonUtf8([string]$Program, [string[]]$Arguments) { $encoding = $env:PYTHONIOENCODING; $utf8 = $env:PYTHONUTF8; try { $env:PYTHONIOENCODING = 'utf-8'; $env:PYTHONUTF8 = '1'; & $Program @Arguments } finally { if ($null -eq $encoding) { Remove-Item Env:PYTHONIOENCODING -ErrorAction SilentlyContinue } else { $env:PYTHONIOENCODING = $encoding }; if ($null -eq $utf8) { Remove-Item Env:PYTHONUTF8 -ErrorAction SilentlyContinue } else { $env:PYTHONUTF8 = $utf8 } } }
function Get-OpenBBState {
    if (-not (Test-Path -LiteralPath $Venv -PathType Container)) { return "missing ($Venv)" }
    if (-not (Test-Path -LiteralPath $Python -PathType Leaf)) { return "broken (missing interpreter: $Python)" }
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $probeOutput = @(& $Python -c 'import openbb' 2>&1 | ForEach-Object { $_.ToString() })
        $probeExit = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousErrorAction }
    if ($probeExit -eq 0) { return "healthy ($Python)" }
    $fullOutput = $probeOutput -join [Environment]::NewLine
    return "broken ($Python; import exit $probeExit; output):$([Environment]::NewLine)$fullOutput"
}
function Show-Status {
    Write-Output "setup-trading-tools: target=$Target"
    foreach ($bin in @('node', 'npm', 'python', 'git')) {
        $found = Get-Command $bin -ErrorAction SilentlyContinue
        if ($found) {
            $previousErrorAction = $ErrorActionPreference
            try {
                $ErrorActionPreference = 'Continue'
                $statusOutput = @(& $bin --version 2>&1)
                $statusExit = $LASTEXITCODE
            } finally { $ErrorActionPreference = $previousErrorAction }
            foreach ($line in $statusOutput) { Write-Output $line.ToString() }
        } else { Write-Output "${bin}: missing" }
    }
    foreach ($path in @((Join-Path $TvDir 'src/server.js'), (Join-Path $TvDir 'src/cli/index.js'), $Api, $McpFile)) {
        if (Test-Path -LiteralPath $path) { Write-Output "present: $path" } else { Write-Output "missing: $path" }
    }
    Write-Output "OpenBB venv: $(Get-OpenBBState)"
}
function Invoke-External([string]$Program, [string[]]$Arguments) {
    if ($DryRun) { Write-Output "[dry-run] $Program $($Arguments -join ' ')"; return }
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        if ($Arguments -contains 'pip') { $commandOutput = @(Invoke-PythonUtf8 $Program $Arguments 2>&1) } else { $commandOutput = @(& $Program @Arguments 2>&1) }
        $commandExit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    foreach ($line in $commandOutput) { [Console]::Out.WriteLine($line.ToString()) }
    if ($commandExit -ne 0) {
        $commandLine = (@($Program) + @($Arguments) | ForEach-Object {
            $argument = [string]$_
            if ($argument -match '[\s"]') { '"' + $argument.Replace('"', '\"') + '"' }
            else { $argument }
        }) -join ' '
        throw "Command failed with exit ${commandExit}: $commandLine"
    }
}
function Invoke-TradingViewNpmInstall {
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $npmOutput = @(& npm install --omit=dev 2>&1)
        $npmExit = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousErrorAction }
    foreach ($line in $npmOutput) { [Console]::Out.WriteLine($line.ToString()) }
    if ($npmExit -eq 0) { return }

    $packageJson = Join-Path $TvDir 'package.json'
    Write-Output "npm package.json: $packageJson (exists: $(Test-Path -LiteralPath $packageJson -PathType Leaf))"
    $logPath = ''
    foreach ($line in $npmOutput) {
        if ($line.ToString() -match 'log of this run can be found in:\s*(.+)$') { $logPath = $Matches[1].Trim() }
    }
    if (-not $logPath) {
        $cache = if ($env:npm_config_cache) { $env:npm_config_cache } elseif ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'npm-cache' } else { '' }
        if ($cache) {
            $log = Get-ChildItem -LiteralPath (Join-Path $cache '_logs') -Filter '*-debug-0.log' -File -ErrorAction SilentlyContinue |
                Sort-Object LastWriteTime -Descending | Select-Object -First 1
            if ($log) { $logPath = $log.FullName }
        }
    }
    if ($logPath -and (Test-Path -LiteralPath $logPath -PathType Leaf)) {
        Write-Output "npm debug log: $logPath"
        Get-Content -LiteralPath $logPath | ForEach-Object { Write-Output $_ }
    } else {
        Write-Output "npm debug log: no log file found (reported path: $logPath)"
    }
    throw "npm failed with exit $npmExit"
}
function Get-OpenBBSpecifier {
    $metadata = Get-ChildItem -LiteralPath $Venv -Filter 'openbb-*.dist-info' -Directory -Recurse -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName 'METADATA' } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Select-Object -First 1
    if ($metadata) {
        $line = Get-Content -LiteralPath $metadata | Where-Object { $_ -like 'Requires-Python: *' } | Select-Object -First 1
        if ($line) { return $line.Substring('Requires-Python: '.Length) }
    }
    return '>=3.10,<4'
}
function Test-PythonCandidate([string]$Program, [string[]]$Prefix, [string]$Specifier, [string]$VersionOverride = '') {
    $probe = @'
import os, re, sys
spec = os.environ['ARCHON_SETUP_PYTHON_SPECIFIER']
version = sys.version_info
override = os.environ.get("ARCHON_SETUP_PYTHON_VERSION", "")
if override:
    match = re.fullmatch(r"(\d+(?:\.\d+){1,2})(a|b|rc)?(\d+)?", override)
    if not match: raise SystemExit("invalid test version override: " + override)
    parts = [int(part) for part in match.group(1).split(".")]
    parts += [0] * (3 - len(parts))
    label, serial = match.group(2), match.group(3)
    level = {"a": "alpha", "b": "beta", "rc": "candidate"}.get(label, "final")
    version = type("Version", (), {"__getitem__": lambda self, key: (parts + [level, int(serial or 0)])[key], "releaselevel": level, "serial": int(serial or 0)})()
release = tuple(version[:3])
pre = {"alpha": (0, version.serial), "beta": (1, version.serial), "candidate": (2, version.serial)}.get(version.releaselevel, (3, 0))
def compare(left, right):
    a, a_pre = left
    b, b_pre = right
    a = tuple(a[:next((i for i in range(len(a) - 1, -1, -1) if a[i] != 0), -1) + 1])
    b = tuple(b[:next((i for i in range(len(b) - 1, -1, -1) if b[i] != 0), -1) + 1])
    width = max(len(a), len(b))
    a, b = a + (0,) * (width - len(a)), b + (0,) * (width - len(b))
    if a != b: return (a > b) - (a < b)
    return (a_pre > b_pre) - (a_pre < b_pre)
def check_clause(clause):
    match = re.fullmatch(r"(>=|<=|==|!=|~=|>|<)\s*(\d+(?:\.\d+)*)(a|b|rc)?(\d+)?(\.\*)?", clause.strip(), re.I)
    if not match:
        op = re.match(r"([^\s\d]+)", clause.strip())
        return False, "unknown operator %s" % (op.group(1) if op else "(invalid clause)")
    op, raw, pre_label, pre_serial, wildcard = match.groups()
    if pre_label and pre_serial is None: return False, "invalid pre-release clause"
    wanted_pre = (({"a": 0, "b": 1, "rc": 2}[(pre_label or "").lower()], int(pre_serial)) if pre_label else (3, 0))
    wanted = (tuple(map(int, raw.split("."))), wanted_pre)
    order = compare((release, pre), wanted)
    if wildcard and op in ("==", "!="):
        equal = release[:len(wanted[0])] == wanted[0]
        return (equal if op == "==" else not equal), None
    if wildcard: return False, "wildcard is only valid with == or !="
    if op == ">=": return order >= 0, None
    if op == ">": return order > 0, None
    if op == "<=": return order <= 0, None
    if op == "<": return order < 0, None
    if op == "==": return order == 0, None
    if op == "!=": return order != 0, None
    if op == "~=":
        if len(wanted[0]) < 2: return False, "invalid compatible-release clause"
        upper = list(wanted[0][:-1]); upper[-1] += 1
        return order >= 0 and compare((release, pre), (tuple(upper), (3, 0))) < 0, None
clauses = spec.split(",")
for clause in clauses:
    _, error = check_clause(clause)
    if error:
        print("%s (treated as unsatisfied)" % error)
        raise SystemExit(2)
for clause in clauses:
    passed, _ = check_clause(clause)
    if not passed: raise SystemExit(1)
raise SystemExit(0)
'@
    $probeTemp = New-TemporaryFile
    $probeFile = "$($probeTemp.FullName).py"
    $previousSpecifier = $env:ARCHON_SETUP_PYTHON_SPECIFIER
    $previousVersion = $env:ARCHON_SETUP_PYTHON_VERSION
    try {
        Move-Item -LiteralPath $probeTemp.FullName -Destination $probeFile
        Set-Content -LiteralPath $probeFile -Value $probe -Encoding utf8
        $env:ARCHON_SETUP_PYTHON_SPECIFIER = $Specifier
        if ($VersionOverride) { $env:ARCHON_SETUP_PYTHON_VERSION = $VersionOverride }
        else { Remove-Item Env:ARCHON_SETUP_PYTHON_VERSION -ErrorAction SilentlyContinue }
        $pythonArguments = @($Prefix) + @($probeFile)
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $script:PythonProbeError = (Invoke-PythonUtf8 $Program $pythonArguments 2>&1 | Out-String).Trim()
            $probeExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $previousErrorAction }
        return $probeExit -eq 0
    } catch { return $false }
    finally {
        if ($null -eq $previousSpecifier) { Remove-Item Env:ARCHON_SETUP_PYTHON_SPECIFIER -ErrorAction SilentlyContinue }
        else { $env:ARCHON_SETUP_PYTHON_SPECIFIER = $previousSpecifier }
        if ($null -eq $previousVersion) { Remove-Item Env:ARCHON_SETUP_PYTHON_VERSION -ErrorAction SilentlyContinue }
        else { $env:ARCHON_SETUP_PYTHON_VERSION = $previousVersion }
        Remove-Item -LiteralPath $probeFile, $probeTemp.FullName -Force -ErrorAction SilentlyContinue
    }
}
function Test-PythonWheelResolution([string]$Program, [string[]]$Prefix, [string[]]$Packages) {
    $reportFile = [IO.Path]::GetTempFileName()
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $pipOutput = @(Invoke-PythonUtf8 $Program ($Prefix + @('-m', 'pip', 'install', '--dry-run', '--only-binary=:all:', '--quiet', '--report', $reportFile, '--ignore-installed') + $Packages) 2>&1)
        $pipExit = $LASTEXITCODE
    } catch { Remove-Item -LiteralPath $reportFile -Force -ErrorAction SilentlyContinue; throw } finally { $ErrorActionPreference = $previousErrorAction }
    $script:PythonWheelProbeError = @($pipOutput | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
    try {
        if ($pipExit -ne 0) { return $false }
        $report = (Get-Content -LiteralPath $reportFile -Raw -Encoding utf8 | ConvertFrom-Json -ErrorAction Stop)
        $openbb = @($report.install | Where-Object { $_.metadata.name -ieq 'openbb' } | Select-Object -First 1)
        if ($openbb.Count -eq 0 -or -not $openbb[0].metadata.version) { throw 'pip report did not contain openbb metadata/version' }
        $script:PythonWheelProbeVersion = [string]$openbb[0].metadata.version
        return $true
    } catch {
        $script:PythonWheelProbeError = "pip succeeded but its report could not be read: $($_.Exception.Message)`n$script:PythonWheelProbeError"
        return $false
    } finally { Remove-Item -LiteralPath $reportFile -Force -ErrorAction SilentlyContinue }
}
function Compare-OpenBBVersions([hashtable]$Candidate, [string]$Left, [string]$Right) {
    $compareArguments = @($Candidate.FixedArguments) + @((Join-Path $PSScriptRoot 'setup-trading-tools-python-probe.py'), '--compare', $Left, $Right)
    $previousErrorAction = $ErrorActionPreference
    try { $ErrorActionPreference = 'Continue'; $comparisonText = (Invoke-PythonUtf8 $Candidate.Program $compareArguments 2>&1 | Out-String).Trim(); $comparisonExit = $LASTEXITCODE } finally { $ErrorActionPreference = $previousErrorAction }
    if ($comparisonExit -ne 0 -or $comparisonText -notmatch '^(?:-1|0|1)$') {
        throw "Could not compare OpenBB versions '$Left' and '$Right': $comparisonText"
    }
    return [int]$comparisonText
}
function Get-PathApplications([string]$Name) {
    $extensions = @($env:PATHEXT -split ';' | Where-Object { $_ -match '^\.(COM|EXE|BAT|CMD)$' })
    if ($extensions.Count -eq 0) { $extensions = @('.COM', '.EXE', '.BAT', '.CMD') }
    foreach ($directory in ($env:PATH -split [IO.Path]::PathSeparator)) {
        if (-not $directory) { $directory = (Get-Location).Path }
        foreach ($extension in $extensions) {
            $path = Join-Path $directory ($Name + $extension)
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
            $command = Get-Command -Name $path -CommandType Application -ErrorAction SilentlyContinue
            if ($command) { Write-Output $path }
        }
    }
}
function Get-PythonCandidates {
    $seen = @{}
    $candidates = @()
    $candidateOrder = 0
    foreach ($py in @(Get-PathApplications 'py')) {
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $installed = @(& $py -0p 2>&1)
        } finally { $ErrorActionPreference = $previousErrorAction }
        foreach ($line in $installed) {
            if ($line.ToString() -match '^\s*-(?:V:)?[^\s]+\s+(?:\*\s+)?(.+?)\s*$') {
                $path = $Matches[1].Trim()
                $path = $path -replace '\s+\*$', ''
                if (Test-Path -LiteralPath $path -PathType Leaf) {
                    $resolvedPath = (Resolve-Path -LiteralPath $path).Path
                    if ($seen.ContainsKey($resolvedPath)) { continue }
                    $seen[$resolvedPath] = $true
                    $candidates += @{ Program = $resolvedPath; FixedArguments = [string[]]@(); Order = $candidateOrder }
                    $candidateOrder++
                }
            }
        }
    }
    foreach ($name in @('python', 'python3')) {
        foreach ($program in @(Get-PathApplications $name)) {
            $resolvedPath = (Resolve-Path -LiteralPath $program).Path
            if (-not $seen.ContainsKey($resolvedPath)) {
                $seen[$resolvedPath] = $true
                $candidates += @{ Program = $resolvedPath; FixedArguments = [string[]]@(); Order = $candidateOrder }
                $candidateOrder++
            }
        }
    }
    $ranked = @()
    $unranked = @()
    foreach ($candidate in $candidates) {
        $versionArgs = @($candidate.FixedArguments) + @('--version')
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $versionText = (& $candidate.Program @versionArgs 2>&1 | Out-String).Trim()
            $versionExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $previousErrorAction }
        if ($versionExit -eq 0 -and $versionText -match 'Python\s+(\d+(?:\.\d+){1,2})') {
            $ranked += @{ Candidate = $candidate; Version = [version]$Matches[1]; Order = $candidate.Order }
        } else { $unranked += $candidate }
    }
    return @($ranked | Sort-Object -Property @{ Expression = { $_.Version }; Descending = $true }, @{ Expression = { $_.Order }; Descending = $false } | ForEach-Object { $_.Candidate }) + $unranked
}
function Resolve-BasePython([string]$Specifier, [object[]]$Candidates = @()) {
    $script:VenvProbed = $false
    if ($Candidates.Count -eq 0) { $Candidates = @(Get-PythonCandidates) }
    $failures = @()
    $specifierSatisfied = $false
    $selected = $null
    $selectedVersion = $null
    foreach ($candidate in $Candidates) {
        if (-not (Test-PythonCandidate $candidate.Program $candidate.FixedArguments $Specifier)) {
            $detail = if ($script:PythonProbeError) { ": $script:PythonProbeError" } else { '' }
            Write-Host "OpenBB wheel probe: $($candidate.Program) skipped; Python does not satisfy Requires-Python $Specifier$detail"
            if ($detail) { $failures += "$($candidate.Program) Requires-Python probe$detail" }
            continue
        }
        $specifierSatisfied = $true
        Assert-SafeVenvRebuild
        $venvArguments = @($candidate.FixedArguments) + @('-m', 'venv', '--clear', $Venv)
        try {
            $previousErrorAction = $ErrorActionPreference
            try {
                $ErrorActionPreference = 'Continue'
                $createOutput = @(Invoke-PythonUtf8 $candidate.Program $venvArguments 2>&1)
                $createExit = $LASTEXITCODE
            } finally { $ErrorActionPreference = $previousErrorAction }
            if ($createExit -ne 0 -or -not (Test-Path -LiteralPath $Python -PathType Leaf)) {
                $detail = @($createOutput | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
                Write-Host "OpenBB wheel probe: $($candidate.Program) pip error during venv creation (exit $createExit)`n$detail"
                $failures += "$($candidate.Program) venv creation (exit $createExit): $detail"
                continue
            }
            $previousErrorAction = $ErrorActionPreference
            try {
                $ErrorActionPreference = 'Continue'
                $upgradeOutput = @(Invoke-PythonUtf8 $Python @('-m', 'pip', 'install', '--upgrade', 'pip') 2>&1)
                $upgradeExit = $LASTEXITCODE
            } finally { $ErrorActionPreference = $previousErrorAction }
            if ($upgradeExit -ne 0) {
                $detail = @($upgradeOutput | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
                Write-Host "OpenBB wheel probe: $($candidate.Program) pip error during upgrade (exit $upgradeExit)`n$detail"
                $failures += "$($candidate.Program) pip upgrade (exit $upgradeExit): $detail"
                continue
            }
            if (Test-PythonWheelResolution $Python @() @('openbb', 'openbb-platform-api')) {
                $resolvedVersion = $script:PythonWheelProbeVersion
                Write-Host "OpenBB wheel probe: $($candidate.Program) resolves openbb $resolvedVersion"
                if ($null -eq $selectedVersion -or (Compare-OpenBBVersions $candidate $resolvedVersion $selectedVersion) -gt 0) {
                    $selected = $candidate
                    $selectedVersion = $resolvedVersion
                }
                continue
            }
            Write-Host "OpenBB wheel probe: $($candidate.Program) pip error`n$script:PythonWheelProbeError"
            $failures += "$($candidate.Program) wheel probe: $script:PythonWheelProbeError"
        } catch {
            if ($_.Exception.Message -like 'Could not compare OpenBB versions*') { throw }
            Write-Host "OpenBB wheel probe: $($candidate.Program) pip error`n$($_.Exception.Message)"
            $failures += "$($candidate.Program): $($_.Exception.Message)"
        }
    }
    if ($null -ne $selected) {
        Assert-SafeVenvRebuild
        $selectedVenvArgs = @($selected.FixedArguments) + @('-m', 'venv', '--clear', $Venv)
        $previousErrorAction = $ErrorActionPreference
        try { $ErrorActionPreference = 'Continue'; $selectedVenvOutput = @(Invoke-PythonUtf8 $selected.Program $selectedVenvArgs 2>&1); $selectedVenvExit = $LASTEXITCODE } finally { $ErrorActionPreference = $previousErrorAction }
        if ($selectedVenvExit -ne 0 -or -not (Test-Path -LiteralPath $Python -PathType Leaf)) {
            $detail = @($selectedVenvOutput | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
            throw "Selected Python '$($selected.Program)' could not recreate its OpenBB venv: $detail"
        }
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $selectedUpgradeOutput = @(Invoke-PythonUtf8 $Python @('-m', 'pip', 'install', '--upgrade', 'pip') 2>&1)
            $selectedUpgradeExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $previousErrorAction }
        if ($selectedUpgradeExit -ne 0) {
            $detail = @($selectedUpgradeOutput | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
            throw "Selected Python '$($selected.Program)' could not upgrade pip: $detail"
        }
        $selected.ResolvedOpenBBVersion = $selectedVersion
        $script:VenvProbed = $true
        Write-Host "OpenBB Python: $($selected.Program) chosen because it resolves the highest OpenBB version, $selectedVersion"
        return $selected
    }
    if (-not $specifierSatisfied) { throw "No Python interpreter satisfies OpenBB Requires-Python specifier '$Specifier'. $($failures -join [Environment]::NewLine)" }
    throw "No Python interpreter can resolve OpenBB entirely from wheels. $($failures -join [Environment]::NewLine)"
}
function Assert-SafeVenvRebuild {
    if (Test-Path -LiteralPath $Venv) {
        $attributes = [IO.File]::GetAttributes($Venv)
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            $linkTarget = 'unknown target'
            try {
                $item = Get-Item -Force -LiteralPath $Venv
                if ($item.Target) { $linkTarget = $item.Target -join ', ' }
            } catch { }
            throw "Refusing to rebuild symlinked or reparse-point OpenBB venv: $Venv -> $linkTarget"
        }
        $item = Get-Item -Force -LiteralPath $Venv
        if (-not $item.PSIsContainer) { throw "Refusing to rebuild non-venv file path: $Venv (missing pyvenv.cfg)" }
        if (-not (Test-Path -LiteralPath (Join-Path $Venv 'pyvenv.cfg') -PathType Leaf)) {
            $contents = @(Get-ChildItem -Force -LiteralPath $Venv -ErrorAction SilentlyContinue)
            if ($contents.Count -gt 0) { throw "Refusing to rebuild non-venv directory with contents: $Venv (missing pyvenv.cfg)" }
        }
    }
}
if ($LoadFunctionsOnly) { return }
if ($Check -or $DryRun) { Show-Status; exit 0 }
New-Item -ItemType Directory -Force -Path $ToolsDir, (Join-Path $Target 'scripts') | Out-Null
if (-not $SkipTradingView) {
    foreach ($bin in @('git', 'node', 'npm')) { if (-not (Get-Command $bin -ErrorAction SilentlyContinue)) { throw "Missing required binary: $bin. Run scripts/install-system-deps.ps1 -WithTradingTools." } }
    if (Test-Path -LiteralPath (Join-Path $TvDir '.git')) { Invoke-External git @('-C', $TvDir, 'pull', '--ff-only') }
    else { Invoke-External git @('clone', 'https://github.com/tradesdontlie/tradingview-mcp.git', $TvDir) }
    if (-not $DryRun) {
        if (-not (Test-Path -LiteralPath $TvDir -PathType Container)) {
            throw "TradingView directory is missing after git clone or pull: $TvDir"
        }
        $packageJson = Join-Path $TvDir 'package.json'
        if (-not (Test-Path -LiteralPath $packageJson -PathType Leaf)) {
            throw "TradingView package.json is missing after git clone or pull: $TvDir (expected $packageJson)"
        }
    }
    Push-Location -LiteralPath $TvDir
    try { Invoke-TradingViewNpmInstall }
    finally { Pop-Location }
    $configScript = @'
const fs=require('fs'), file=process.argv[2], server=process.argv[3];
let c={}; if(fs.existsSync(file)) c=JSON.parse(fs.readFileSync(file,'utf8'));
c.mcpServers=c.mcpServers||{}; c.mcpServers.tradingview={command:'node',args:[server],env:{TV_CDP_HOST:'localhost',TV_CDP_PORT:'9222'},toolPolicy:{trustServerHints:false,toolPermissions:{tv_health_check:'safe',tv_discover:'safe',tv_ui_state:'safe',chart_get_state:'safe',quote_get:'safe',data_get_ohlcv:'safe',data_get_study_values:'safe',data_get_pine_lines:'safe',data_get_pine_labels:'safe',data_get_pine_tables:'safe',capture_screenshot:'safe',pine_analyze:'safe',pine_check:'safe',pine_get_errors:'safe',pine_get_console:'safe',pine_get_source:'risky',pine_set_source:'risky',pine_compile:'risky',pine_smart_compile:'risky',pine_save:'risky',alert_create:'risky',alert_delete:'risky',replay_trade:'risky',tv_launch:'risky'}}};
fs.writeFileSync(file,JSON.stringify(c,null,2)+'\n');
'@
    if ($DryRun) { Write-Output '[dry-run] write TradingView MCP config' }
    else {
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $nodeOutput = @($configScript | & node - $McpFile (Join-Path $TvDir 'src/server.js') 2>&1)
            $nodeExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $previousErrorAction }
        foreach ($line in $nodeOutput) { [Console]::Out.WriteLine($line.ToString()) }
        if ($nodeExit -ne 0) { throw 'Failed to write TradingView MCP config' }
    }
    @'
$ErrorActionPreference='Stop'
$Root=Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Port=if($args.Count -gt 0){$args[0]}else{'9222'}
$Launcher=Join-Path $Root '.archon/tools/tradingview-mcp/scripts/launch_tv_debug_windows.ps1'
if(-not(Test-Path -LiteralPath $Launcher -PathType Leaf)){throw "TradingView CDP launcher is missing: $Launcher. Install the external TradingView launcher project."}
$PreviousErrorAction=$ErrorActionPreference
try{$ErrorActionPreference='Continue';& $Launcher $Port;$LauncherExit=$LASTEXITCODE}finally{$ErrorActionPreference=$PreviousErrorAction}
exit $LauncherExit
'@ | Set-Content -LiteralPath (Join-Path $Target 'scripts/start-tradingview-cdp.ps1') -Encoding utf8
}
if (-not $SkipOpenBB) {
    $basePython = $null
    if ((Get-OpenBBState) -notlike 'healthy*') {
        Assert-SafeVenvRebuild
        $specifier = Get-OpenBBSpecifier
        $basePython = Resolve-BasePython $specifier
        $versionArguments = @($basePython.FixedArguments) + @('--version')
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $pythonVersion = (& $basePython.Program @versionArguments 2>&1 | Out-String).Trim()
            $versionExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $previousErrorAction }
        if ($versionExit -ne 0) { throw "Could not read Python version from '$($basePython.Program)'" }
        $prefixText = if ($basePython.FixedArguments.Count) { " $($basePython.FixedArguments -join ' ')" } else { '' }
        Write-Output "OpenBB Python: $($basePython.Program)$prefixText ($pythonVersion)"
        if (-not (Test-Path -LiteralPath $Python -PathType Leaf)) {
            throw "OpenBB venv creation with base interpreter '$($basePython.Program)' did not create expected interpreter '$Python' in venv '$Venv'."
        }
    }
    if (-not $VenvProbed) { Invoke-External $Python @('-m', 'pip', 'install', '--upgrade', 'pip') }
    Invoke-External $Python @('-m', 'pip', 'install', '--only-binary=:all:', 'openbb', 'openbb-platform-api')
    if ($basePython -and $basePython.ResolvedOpenBBVersion) {
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $installedVersion = (& $Python -c 'import importlib.metadata as m; print(m.version("openbb"))' 2>&1 | Out-String).Trim()
            $installedVersionExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $previousErrorAction }
        if ($installedVersionExit -ne 0 -or $installedVersion -ne $basePython.ResolvedOpenBBVersion) {
            throw "Installed OpenBB version '$installedVersion' differs from wheel probe version '$($basePython.ResolvedOpenBBVersion)'"
        }
    }
    $openbbBuilder = Join-Path $Venv 'Scripts/openbb-build.exe'
    if (Test-Path -LiteralPath $openbbBuilder -PathType Leaf) { Invoke-External $openbbBuilder @() }
    $openbbState = Get-OpenBBState
    if ($openbbState -notlike 'healthy*') { throw "OpenBB environment remains broken after installation: $Venv`n$openbbState" }
    @'
$ErrorActionPreference='Stop'
$Root=Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$HostName=if($env:OPENBB_HOST){$env:OPENBB_HOST}else{'127.0.0.1'}
$Port=if($env:OPENBB_PORT){$env:OPENBB_PORT}else{'6900'}
$Python=Join-Path $Root '.archon/tools/openbb-venv/Scripts/python.exe'
$Api=Join-Path $Root '.archon/tools/openbb-venv/Scripts/openbb-api.exe'
if(-not(Test-Path -LiteralPath $Python -PathType Leaf)){throw "OpenBB interpreter is missing: $Python. Run archon trading setup to repair the environment."}
$PreviousErrorAction=$ErrorActionPreference
try{$ErrorActionPreference='Continue';& $Python -c 'import openbb' 2>&1 | ForEach-Object { [Console]::Out.WriteLine($_.ToString()) };$PythonExit=$LASTEXITCODE}finally{$ErrorActionPreference=$PreviousErrorAction}
if($PythonExit -ne 0){throw 'OpenBB environment is broken. Run archon trading setup to recreate it.'}
if(-not(Test-Path -LiteralPath $Api -PathType Leaf)){throw "OpenBB API launcher is missing: $Api. Run archon trading setup to install packages."}
$PreviousErrorAction=$ErrorActionPreference
try{$ErrorActionPreference='Continue';& $Api --host $HostName --port $Port 2>&1 | ForEach-Object { [Console]::Out.WriteLine($_.ToString()) };$ApiExit=$LASTEXITCODE}finally{$ErrorActionPreference=$PreviousErrorAction}
exit $ApiExit
'@ | Set-Content -LiteralPath (Join-Path $Target 'scripts/start-openbb-api.ps1') -Encoding utf8
}
Write-Output 'setup-trading-tools: complete'
Show-Status
