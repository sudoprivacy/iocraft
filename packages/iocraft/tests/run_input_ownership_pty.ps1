param(
    [Parameter(Mandatory=$true)][string]$CodePath,
    [Parameter(Mandatory=$true)][string]$AppPath,
    [Parameter(Mandatory=$true)][string]$FixturePath,
    [Parameter(Mandatory=$true)][string]$LogDirectory
)
$ErrorActionPreference = 'Stop'
$product = Get-Content -LiteralPath (Join-Path $AppPath 'product.json') -Raw -Encoding utf8 | ConvertFrom-Json
if ($product.commit -ne '07f806f999227108933c2e30515b26eecc1fda74') { throw 'Unexpected terminal host' }
$CodePath = (Resolve-Path -LiteralPath $CodePath).Path
$FixturePath = (Resolve-Path -LiteralPath $FixturePath).Path
$env:ELECTRON_RUN_AS_NODE = '1'
$env:IOCRAFT_TERMINAL_MODULES = Join-Path $AppPath 'node_modules.asar'
New-Item -ItemType Directory -Path $LogDirectory -Force | Out-Null
$failed = @()
foreach ($backend in @('bundled', 'system')) {
    foreach ($scenario in @('clear', 'paste', 'middle-paste', 'clear-paste')) {
        $prefix = Join-Path $LogDirectory "$backend-$scenario"
        $env:IOCRAFT_WIRE_TRACE = "$prefix-wire.jsonl"
        [System.IO.File]::WriteAllText($env:IOCRAFT_WIRE_TRACE, '')
        $ErrorActionPreference = 'Continue'
        & $CodePath (Join-Path $PSScriptRoot 'input_ownership_pty.cjs') $FixturePath $backend $scenario 2> "$prefix.stderr.log" | Tee-Object -FilePath "$prefix.log"
        $result = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        Get-Content -LiteralPath "$prefix.stderr.log" -Encoding utf8
        if ($result -ne 0) { $failed += "$backend $scenario (exit $result)" }
    }
}
if ($failed.Count -gt 0) { throw "Input PTY failed: $($failed -join ', '). See $LogDirectory" }
