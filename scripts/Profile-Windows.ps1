param(
    [string]$EnginePath = '',
    [string]$OutputDirectory = '',
    [string]$TargetDirectory = '',
    [int]$IntervalSeconds = 1,
    [switch]$BuildOnly
)
$ErrorActionPreference = 'Stop'
if ($IntervalSeconds -lt 1) { throw 'IntervalSeconds must be at least 1.' }
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$target = if ($TargetDirectory) {
    if ([IO.Path]::IsPathRooted($TargetDirectory)) { [IO.Path]::GetFullPath($TargetDirectory) }
    else { [IO.Path]::GetFullPath((Join-Path $root $TargetDirectory)) }
} else { Join-Path $root 'target\diagnostics' }
& cargo build --locked --release --package deeznats --features diagnostics --target-dir $target
if ($LASTEXITCODE -ne 0) { throw 'Diagnostics build failed.' }
$bin = Join-Path $target 'release'
if (!$EnginePath) { $EnginePath = Join-Path $root 'target\release\libdeezercore.dll' }
if (Test-Path -LiteralPath $EnginePath) { Copy-Item -LiteralPath $EnginePath -Destination (Join-Path $bin 'libdeezercore.dll') -Force }
else { Write-Warning 'Deezer engine DLL unavailable; account playback requires it. Supply -EnginePath.' }
Copy-Item -LiteralPath (Join-Path $root 'assets\DeezNats.ico') -Destination $bin -Force
if ($BuildOnly) { Write-Host "Diagnostics build: $bin"; return }
if (!$OutputDirectory) {
    $OutputDirectory = Join-Path $env:LOCALAPPDATA ("DeezNatsRust\diagnostics\" + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-ffff'))
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$names = @('DEEZNATS_RUST_ALLOC_LOG', 'DEEZNATS_ARTWORK_METRICS', 'DEEZNATS_TIMING_LOG', 'DEEZNATS_GO_MEMORY_LOG')
$old = @{}
foreach ($name in $names) { $old[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $env:DEEZNATS_RUST_ALLOC_LOG = Join-Path $OutputDirectory 'rust-alloc.jsonl'
    $env:DEEZNATS_ARTWORK_METRICS = Join-Path $OutputDirectory 'render-memory.jsonl'
    $env:DEEZNATS_TIMING_LOG = Join-Path $OutputDirectory 'timings.jsonl'
    $env:DEEZNATS_GO_MEMORY_LOG = Join-Path $OutputDirectory 'go-memory.jsonl'
    $app = Start-Process -FilePath (Join-Path $bin 'deeznats.exe') -WorkingDirectory $root -PassThru
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $old[$name], 'Process') }
}
Write-Host "Profiling PID $($app.Id). Close the app to finish. Results: $OutputDirectory"
$startTicks = $app.StartTime.ToUniversalTime().Ticks
$previous = @{}
$out = Join-Path $OutputDirectory 'processes.jsonl'
while ($true) {
    $rootProcess = Get-Process -Id $app.Id -ErrorAction SilentlyContinue
    if (!$rootProcess -or $rootProcess.StartTime.ToUniversalTime().Ticks -ne $startTicks) { break }
    $tree = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name)
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($app.Id)
    do {
        $added = $false
        foreach ($item in $tree) {
            if ($ids.Contains([int]$item.ParentProcessId) -and $ids.Add([int]$item.ProcessId)) { $added = $true }
        }
    } while ($added)
    $samples = foreach ($id in $ids) {
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($p) {
            try {
                $key = "$id-$($p.StartTime.ToUniversalTime().Ticks)"
                $cpu = $p.TotalProcessorTime.TotalSeconds
                $delta = if ($previous.ContainsKey($key)) { $cpu - $previous[$key] } else { $null }
                $previous[$key] = $cpu
                @{ pid=$id; name=$p.ProcessName; private_bytes=$p.PrivateMemorySize64; working_set_bytes=$p.WorkingSet64; cpu_seconds_delta=$delta; threads=$p.Threads.Count; handles=$p.HandleCount }
            } catch { }
        }
    }
    $record = @{ time_unix_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds(); processes=@($samples) }
    [IO.File]::AppendAllText($out, (($record | ConvertTo-Json -Depth 5 -Compress) + "`n"), [Text.UTF8Encoding]::new($false))
    Start-Sleep -Seconds $IntervalSeconds
}
if (Get-Command python -ErrorAction SilentlyContinue) {
    & python (Join-Path $PSScriptRoot 'Analyze-Profile.py') $OutputDirectory
    if ($LASTEXITCODE -eq 0) { Write-Host "Report: $(Join-Path $OutputDirectory 'report.md')" }
    else { Write-Warning "Report generation failed. Raw session data: $OutputDirectory" }
} else {
    Write-Warning "Python is unavailable. Generate the report later: python scripts/Analyze-Profile.py `"$OutputDirectory`""
}
