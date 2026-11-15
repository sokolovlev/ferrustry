# Полный набор бенчмарков Ferrustry для Windows (PowerShell).
# Использование: powershell -ExecutionPolicy Bypass -File scripts\bench-suite.ps1 [-Quick]
param([switch]$Quick)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
$commit = (git rev-parse --short HEAD) 2>$null
if (-not $commit) { $commit = "unknown" }
$dir = "bench-results\$(Get-Date -Format yyyy-MM-dd_HHmm)-$commit"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
cargo build --release -p ferrustry-headless
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$os = Get-CimInstance Win32_OperatingSystem
@(
  "commit: $commit",
  "rustc: $(rustc --version)",
  "os: $($os.Caption) $($os.Version)",
  "cpu: $($cpu.Name) cores=$($cpu.NumberOfCores) threads=$($cpu.NumberOfLogicalProcessors) maxMHz=$($cpu.MaxClockSpeed)",
  "L2KB: $($cpu.L2CacheSize) L3KB: $($cpu.L3CacheSize)",
  "ramGB: $([math]::Round($os.TotalVisibleMemorySize / 1MB, 1))"
) | Set-Content "$dir\machine.txt"
$env:FERRUSTRY_COMMIT = $commit
$extra = @(); if ($Quick) { $extra += "--quick" }
& target\release\ferrustry-headless.exe suite --runs 3 --out "$dir\suite.json" @extra
Write-Host "Готово: $dir"
