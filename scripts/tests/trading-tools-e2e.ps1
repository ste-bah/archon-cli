$ErrorActionPreference = 'Stop'
$root = (Resolve-Path '.').Path
$target = Join-Path $env:RUNNER_TEMP 'trading tools with spaces'
New-Item -ItemType Directory -Force -Path $target | Out-Null
$skipTradingView = $false
$installFailed = $false
try {
    $output = & "$root/scripts/setup-trading-tools.ps1" -Target $target 2>&1 | Out-String
    $installFailed = $LASTEXITCODE -ne 0
} catch { $output = $_.ToString(); $installFailed = $true }
if ($installFailed) {
    Write-Output $output
    if ($output -notmatch '(git|npm) failed with exit|fatal: unable to access|npm ERR!|Failed to write TradingView MCP config') {
        throw 'Installer assertion failed: setup failed outside the TradingView clone/npm step'
    }
    # If the external TradingView clone or npm install is unavailable, retain
    # real OpenBB coverage and state the skip clearly in CI output.
    Remove-Item -LiteralPath (Join-Path $target '.archon/tools/tradingview-mcp') -Recurse -Force -ErrorAction SilentlyContinue
    $skipTradingView = $true
    Write-Warning 'TradingView setup skipped: its external clone/npm installation failed; retrying OpenBB with -SkipTradingView.'
    try {
        $output = & "$root/scripts/setup-trading-tools.ps1" -Target $target -SkipTradingView 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { throw "Installer exited $LASTEXITCODE`n$output" }
    } catch { throw "Installer assertion failed: real OpenBB setup did not complete`n$output`n$_" }
}
Write-Output $output
if (-not $skipTradingView -and -not (Test-Path -LiteralPath (Join-Path $target '.archon/tools/tradingview-mcp/src/server.js') -PathType Leaf)) {
    throw 'TradingView assertion failed: installer completed without the server entry point'
}
if ($output -notmatch 'OpenBB venv: healthy') { throw 'Installer assertion failed: final OpenBB status was not healthy' }
$venv = Join-Path $target '.archon/tools/openbb-venv'
$python = Join-Path $venv 'Scripts/python.exe'
& $python -c 'import openbb'
if ($LASTEXITCODE -ne 0) { throw 'Import assertion failed: venv Python could not import openbb' }

$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = ([Net.IPEndPoint]$listener.LocalEndpoint).Port
$listener.Stop()
$env:OPENBB_HOST = '127.0.0.1'
$env:OPENBB_PORT = "$port"
$launcher = Join-Path $target 'scripts/start-openbb-api.ps1'
$logPath = Join-Path $target 'openbb-api.log'
$errorLogPath = Join-Path $target 'openbb-api-error.log'
$process = Start-Process -FilePath 'pwsh' -ArgumentList @('-NoProfile', '-File', "`"$launcher`"") -WorkingDirectory $target -RedirectStandardOutput $logPath -RedirectStandardError $errorLogPath -PassThru
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $healthy = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process.HasExited) { throw 'OpenBB API server exited before /docs returned HTTP 200' }
        try {
            $response = Invoke-WebRequest -Uri "http://127.0.0.1:$port/docs" -TimeoutSec 2 -UseBasicParsing
            if ($response.StatusCode -eq 200) { $healthy = $true; break }
        } catch { Start-Sleep -Seconds 1 }
    }
    if (-not $healthy) { throw 'OpenBB API /docs did not return HTTP 200 within 90 seconds' }
    Write-Output "PASS OpenBB API /docs returned HTTP 200 on port $port"
} catch {
    [Console]::Error.WriteLine("API assertion failed: $($_.Exception.Message)")
    [Console]::Error.WriteLine('OpenBB API server log:')
    if (Test-Path -LiteralPath $logPath) { Get-Content -LiteralPath $logPath | ForEach-Object { [Console]::Error.WriteLine($_) } }
    if (Test-Path -LiteralPath $errorLogPath) { Get-Content -LiteralPath $errorLogPath | ForEach-Object { [Console]::Error.WriteLine($_) } }
    throw
} finally {
    if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
}

Remove-Item -LiteralPath $python -Force
$brokenStatus = & "$root/scripts/setup-trading-tools.ps1" -Target $target -Check -SkipTradingView 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "Repair assertion failed: status check exited $LASTEXITCODE`n$brokenStatus" }
if ($brokenStatus -notmatch 'OpenBB venv: broken') { throw "Repair assertion failed: installer did not report the missing interpreter as broken`n$brokenStatus" }
Write-Output 'PASS installer reported broken OpenBB venv after interpreter deletion'
try {
    $repairOutput = & "$root/scripts/setup-trading-tools.ps1" -Target $target -SkipTradingView 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Installer exited $LASTEXITCODE`n$repairOutput" }
} catch { throw "Repair assertion failed: installer did not complete`n$repairOutput`n$_" }
Write-Output $repairOutput
if ($repairOutput -notmatch 'OpenBB venv: healthy') { throw 'Repair assertion failed: rebuilt OpenBB status was not healthy' }
& $python -c 'import openbb'
if ($LASTEXITCODE -ne 0) { throw 'Repair assertion failed: rebuilt interpreter could not import openbb' }
Write-Output 'PASS broken OpenBB interpreter rebuilt and reported healthy'
Write-Output 'test result: trading tools Windows end-to-end passed'
