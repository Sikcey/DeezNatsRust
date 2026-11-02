param(
    [switch]$Apply
)

$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$cutoff = (Get-Date).Date
$candidates = @()

# Only recognizable Cargo target directories in .local are selected.
foreach ($directory in @((Join-Path $project '.local'))) {
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) { continue }
    $parent = Get-Item -LiteralPath $directory -Force
    if ($parent.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        Write-Warning "Skipping linked directory: $directory"
        continue
    }
    $candidates += @(Get-ChildItem -LiteralPath $directory -Directory -Force |
        Where-Object {
            -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -and
            $_.LastWriteTime -lt $cutoff -and
            (Test-Path -LiteralPath (Join-Path $_.FullName 'CACHEDIR.TAG') -PathType Leaf)
        })
}

if (-not $candidates.Count) { Write-Host 'No eligible build directories found.'; return }
foreach ($item in $candidates) {
    if ($Apply) {
        Write-Host "Removing $($item.FullName)"
        Remove-Item -LiteralPath $item.FullName -Recurse -Force -ErrorAction Continue
    } else {
        Write-Host "Would remove $($item.FullName)"
    }
}
if (-not $Apply) { Write-Host 'Preview only. Run with -Apply to remove these build directories last modified before today.' }
