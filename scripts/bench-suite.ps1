# Ferrustry benchmark suite for Windows (Windows PowerShell 5.1+ and PowerShell 7+).
# NOTE: keep this file pure ASCII. Windows PowerShell 5.1 reads BOM-less files as ANSI,
# so any non-ASCII character breaks parsing. CI checks this.
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts\bench-suite.ps1 [-Quick]
# Output: bench-results\<date>-<commit>\ (suite.json, machine.txt)
param([switch]$Quick)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

# git may be missing or this may be a zip download without .git - that is fine.
$commit = "unknown"
try {
    $ErrorActionPreference = "Continue"
    $c = & git rev-parse --short HEAD 2>$null
    if ($LASTEXITCODE -eq 0 -and $c) { $commit = "$c".Trim() }
} catch { }
$ErrorActionPreference = "Stop"

$dir = Join-Path "bench-results" "$(Get-Date -Format yyyy-MM-dd_HHmm)-$commit"
New-Item -ItemType Directory -Force -Path $dir | Out-Null

Write-Host "Building release binary..."
& cargo build --release -p ferrustry-headless
if ($LASTEXITCODE -ne 0) { Write-Host "cargo build failed"; exit $LASTEXITCODE }

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$os = Get-CimInstance Win32_OperatingSystem
$rustc = & rustc --version
@(
    "commit: $commit",
    "rustc: $rustc",
    "os: $($os.Caption) $($os.Version)",
    "cpu: $($cpu.Name) cores=$($cpu.NumberOfCores) threads=$($cpu.NumberOfLogicalProcessors) maxMHz=$($cpu.MaxClockSpeed)",
    "L2KB: $($cpu.L2CacheSize) L3KB: $($cpu.L3CacheSize)",
    "ramGB: $([math]::Round($os.TotalVisibleMemorySize / 1MB, 1))"
) | Set-Content -Encoding ASCII (Join-Path $dir "machine.txt")

$env:FERRUSTRY_COMMIT = $commit
$suiteArgs = @("suite", "--runs", "3", "--out", (Join-Path $dir "suite.json"))
if ($Quick) { $suiteArgs += "--quick" }
& (Join-Path "target\release" "ferrustry-headless.exe") @suiteArgs
if ($LASTEXITCODE -ne 0) { Write-Host "benchmark suite failed"; exit $LASTEXITCODE }
Write-Host "Done: $dir"
