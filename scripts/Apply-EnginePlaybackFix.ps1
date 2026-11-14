param(
    [Parameter(Mandatory = $true)]
    [string]$EngineSourceDirectory
)
$ErrorActionPreference = 'Stop'
$patch = Join-Path (Split-Path $PSScriptRoot -Parent) 'patches/deezer-playback-recovery.patch'
$source = (Resolve-Path -LiteralPath $EngineSourceDirectory).Path
if (!(Test-Path -LiteralPath (Join-Path $source 'internal/audio/player.go'))) {
    throw 'EngineSourceDirectory must contain the OpenDeezer go.mod and internal/audio sources.'
}
Push-Location $source
try {
    # Windows PowerShell treats native stderr as an ErrorRecord. A failed
    # reverse check is expected for an unpatched source, not a script error.
    $ErrorActionPreference = 'Continue'
    & git apply --reverse --check --unsafe-paths $patch 2>&1 | Out-Null
    $alreadyApplied = $LASTEXITCODE -eq 0
    $ErrorActionPreference = 'Stop'
    if ($alreadyApplied) {
        Write-Output 'Deezer playback recovery patch is already applied.'
        return
    }
    & git apply --check --unsafe-paths $patch
    if ($LASTEXITCODE -ne 0) {
        throw 'Playback recovery patch does not match this engine source. Reconcile the patch before building.'
    }
    & git apply --unsafe-paths $patch
    if ($LASTEXITCODE -ne 0) { throw 'Could not apply playback recovery patch.' }
    Write-Output 'Applied Deezer playback recovery patch.'
} finally {
    Pop-Location
}
