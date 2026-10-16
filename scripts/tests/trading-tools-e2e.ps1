$ErrorActionPreference = 'Stop'
$root = (Resolve-Path '.').Path
$target = Join-Path $env:RUNNER_TEMP 'trading tools with spaces'
New-Item -ItemType Directory -Force -Path $target | Out-Null
function Get-OpenBBEnvironmentDiagnostics {
    $venvPath = Join-Path $target '.archon/tools/openbb-venv'
    $pythonPath = Join-Path $venvPath 'Scripts/python.exe'
    $lines = @("OpenBB paths: target=$target (length $($target.Length)); venv=$venvPath (length $($venvPath.Length)); interpreter=$pythonPath (length $($pythonPath.Length)); USERPROFILE=$env:USERPROFILE; HOME=$env:HOME; TEMP=$env:TEMP; TMP=$env:TMP; RUNNER_TEMP=$env:RUNNER_TEMP")
    foreach ($name in @('USERPROFILE', 'HOME', 'TEMP', 'TMP', 'RUNNER_TEMP')) {
        $path = [Environment]::GetEnvironmentVariable($name)
        if (-not $path) { continue }
        $exists = Test-Path -LiteralPath $path -PathType Container
        $writable = $false
        if ($exists) {
            $probe = Join-Path $path ('.archon-write-probe-' + [Guid]::NewGuid().ToString('N'))
            try { [IO.File]::WriteAllText($probe, 'probe'); $writable = $true }
            catch { $writable = $false }
            finally { Remove-Item -LiteralPath $probe -Force -ErrorAction SilentlyContinue }
        }
        $lines += "  $name exists=$exists writable=$writable pathLength=$($path.Length)"
    }
    return $lines -join [Environment]::NewLine
}
$output = & "$root/scripts/setup-trading-tools.ps1" -Target $target 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    Write-Output $output
    Write-Output (Get-OpenBBEnvironmentDiagnostics)
    throw 'Installer assertion failed: TradingView-enabled setup did not complete'
}
Write-Output $output
if (-not (Test-Path -LiteralPath (Join-Path $target '.archon/tools/tradingview-mcp/src/server.js') -PathType Leaf)) {
    throw 'TradingView assertion failed: installer completed without the server entry point'
}
if ($output -notmatch 'OpenBB venv: healthy') { throw 'Installer assertion failed: final OpenBB status was not healthy' }
$venv = Join-Path $target '.archon/tools/openbb-venv'
$python = Join-Path $venv 'Scripts/python.exe'
$openbbImportOutput = @(& $python -c 'import openbb' 2>&1 | ForEach-Object { $_.ToString() })
$openbbImportExit = $LASTEXITCODE
if ($openbbImportExit -ne 0) {
    Write-Output "OpenBB import failed with exit $openbbImportExit; last output lines:"
    $openbbImportOutput | Select-Object -Last 20 | ForEach-Object { Write-Output $_ }
    Write-Output (Get-OpenBBEnvironmentDiagnostics)
    throw 'Import assertion failed: venv Python could not import openbb'
}

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
    [Console]::Error.WriteLine("OpenBB import probe exit: $openbbImportExit")
    $openbbImportOutput | Select-Object -Last 20 | ForEach-Object { [Console]::Error.WriteLine($_) }
    [Console]::Error.WriteLine('OpenBB API server log:')
    if (Test-Path -LiteralPath $logPath) { Get-Content -LiteralPath $logPath | ForEach-Object { [Console]::Error.WriteLine($_) } }
    if (Test-Path -LiteralPath $errorLogPath) { Get-Content -LiteralPath $errorLogPath | ForEach-Object { [Console]::Error.WriteLine($_) } }
    throw
} finally {
    # The generated launcher invokes openbb-api.exe, which can in turn start
    # python.exe. Stop the entire tree rooted at the launcher PowerShell.
    $taskkillOutput = & taskkill.exe /PID $process.Id /T /F 2>&1
    if ($taskkillOutput) { $taskkillOutput | ForEach-Object { Write-Output "API teardown: $_" } }
    if (-not $process.HasExited) { $process.WaitForExit() }

    # taskkill's tree walk normally catches every descendant. Sweep any
    # remaining venv processes too, then wait for Windows to release the files.
    $venvPrefix = [IO.Path]::GetFullPath($venv).TrimEnd('\') + '\'
    $drainDeadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $venvProcesses = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
            $_.Path -and $_.Path.StartsWith($venvPrefix, [StringComparison]::OrdinalIgnoreCase)
        })
        foreach ($venvProcess in $venvProcesses) {
            Write-Output "API teardown: stopping remaining venv process $($venvProcess.ProcessName) (PID $($venvProcess.Id), $($venvProcess.Path))"
            & taskkill.exe /PID $venvProcess.Id /T /F 2>&1 | ForEach-Object { Write-Output "API teardown: $_" }
        }
        if ($venvProcesses.Count -gt 0) { Start-Sleep -Milliseconds 500 }
    } while ($venvProcesses.Count -gt 0 -and [DateTime]::UtcNow -lt $drainDeadline)
    $remainingVenvProcesses = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.StartsWith($venvPrefix, [StringComparison]::OrdinalIgnoreCase)
    })
    if ($remainingVenvProcesses.Count -gt 0) {
        $details = $remainingVenvProcesses | ForEach-Object { "$($_.ProcessName) (PID $($_.Id), $($_.Path))" }
        throw "Timed out waiting for OpenBB venv processes to stop: $($details -join '; ')"
    }
    Write-Output 'API teardown: no processes remain from the OpenBB venv'
}

Remove-Item -LiteralPath $python -Force
$brokenStatus = & "$root/scripts/setup-trading-tools.ps1" -Target $target -Check 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "Repair assertion failed: status check exited $LASTEXITCODE`n$brokenStatus" }
if ($brokenStatus -notmatch 'OpenBB venv: broken') { throw "Repair assertion failed: installer did not report the missing interpreter as broken`n$brokenStatus" }
Write-Output 'PASS installer reported broken OpenBB venv after interpreter deletion'
try {
    $repairOutput = & "$root/scripts/setup-trading-tools.ps1" -Target $target 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Installer exited $LASTEXITCODE`n$repairOutput" }
} catch { throw "Repair assertion failed: installer did not complete`n$repairOutput`n$_" }
Write-Output $repairOutput
if ($repairOutput -notmatch 'OpenBB venv: healthy') { throw 'Repair assertion failed: rebuilt OpenBB status was not healthy' }
$openbbImportOutput = @(& $python -c 'import openbb' 2>&1 | ForEach-Object { $_.ToString() })
$openbbImportExit = $LASTEXITCODE
if ($openbbImportExit -ne 0) {
    Write-Output "Repair OpenBB import failed with exit $openbbImportExit; last output lines:"
    $openbbImportOutput | Select-Object -Last 20 | ForEach-Object { Write-Output $_ }
    throw 'Repair assertion failed: rebuilt interpreter could not import openbb'
}
Write-Output 'PASS broken OpenBB interpreter rebuilt and reported healthy'
Write-Output 'test result: trading tools Windows end-to-end passed'
exit 0
