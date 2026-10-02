param(
    [Parameter(Mandatory = $true)][string]$CodePath,
    [Parameter(Mandatory = $true)][string]$AppPath,
    [Parameter(Mandatory = $true)][string]$FixturePath,
    [Parameter(Mandatory = $true)][string]$LogDirectory,
    [ValidateSet('transactions', 'resize', 'rapid', 'tiny', 'oversized-history')][string]$Scenario = 'transactions'
)

$ErrorActionPreference = 'Stop'
$expectedCommit = '07f806f999227108933c2e30515b26eecc1fda74'
$product = Get-Content -LiteralPath (Join-Path $AppPath 'product.json') -Raw -Encoding utf8 | ConvertFrom-Json
if ($product.commit -ne $expectedCommit) {
    throw "Expected VS Code commit $expectedCommit; got $($product.commit)"
}
$CodePath = (Resolve-Path -LiteralPath $CodePath).Path
$FixturePath = (Resolve-Path -LiteralPath $FixturePath).Path
$modulePath = Join-Path $AppPath 'node_modules.asar'
if (!(Test-Path -LiteralPath $modulePath)) { throw "Missing VS Code terminal modules: $modulePath" }
New-Item -ItemType Directory -Path $LogDirectory -Force | Out-Null
$env:ELECTRON_RUN_AS_NODE = '1'
$env:IOCRAFT_TERMINAL_MODULES = $modulePath
$failed = @()
foreach ($backend in @('bundled', 'system')) {
    $log = Join-Path $LogDirectory "$backend-$Scenario.log"
    $env:IOCRAFT_WIRE_TRACE = Join-Path $LogDirectory "$backend-$Scenario-wire.jsonl"
    # A rerun has its own transcript, without merging stale terminal bytes.
    [System.IO.File]::WriteAllText($env:IOCRAFT_WIRE_TRACE, '')
    $arguments = @((Join-Path $PSScriptRoot 'inline_history_pty.cjs'), $FixturePath, $backend)
    if ($Scenario -ne 'transactions') { $arguments += $Scenario }
    # Piping waits for the GUI-subsystem executable even in Windows PowerShell.
    # stderr is captured through its own pipe so a JS assertion's real exit code
    # remains available instead of becoming a PowerShell terminating error.
    $stderrLog = Join-Path $LogDirectory "$backend-$Scenario.stderr.log"
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & $CodePath @arguments 2> $stderrLog | Tee-Object -FilePath $log
    $result = $LASTEXITCODE
    $ErrorActionPreference = $previousPreference
    Get-Content -LiteralPath $stderrLog -Encoding utf8
    if ($result -ne 0) { $failed += "$backend (exit $result)" }
}
if ($failed.Count -gt 0) { throw "PTY $Scenario failed: $($failed -join ', '). See $LogDirectory" }
