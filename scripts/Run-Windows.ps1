param(
    [string]$EnginePath = '',
    [switch]$Release,
    [switch]$BuildOnly,
    [string]$AppearanceProfile = ''
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path -LiteralPath $cargo)) { $cargo = (Get-Command cargo -ErrorAction Stop).Source }
$buildArgs = @('build', '--locked')
$profile = 'debug'
if ($Release) { $buildArgs += '--release'; $profile = 'release' }
& $cargo @buildArgs
if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
$output = Join-Path $PWD "target\$profile"
$outputEngine = Join-Path $output 'libdeezercore.dll'
if (!$EnginePath -and !(Test-Path -LiteralPath $outputEngine)) {
    $EnginePath = 'E:\DeezerNative\windows\DeezNats\native\libdeezercore.dll'
}
if ($EnginePath -and (Test-Path -LiteralPath $EnginePath)) {
    if ((Resolve-Path -LiteralPath $EnginePath).Path -ne $outputEngine) {
        Copy-Item -LiteralPath $EnginePath -Destination $outputEngine -Force
    }
} elseif (!(Test-Path -LiteralPath $outputEngine)) {
    Write-Warning 'Deezer engine DLL unavailable. Account playback requires the Deezer engine.'
}
Copy-Item -LiteralPath (Join-Path $PWD 'assets\DeezNats.ico') -Destination (Join-Path $output 'DeezNats.ico') -Force
if (-not $BuildOnly) {
    # This is the interactive window the user is launching.
    if($AppearanceProfile){
        Start-Process -FilePath (Join-Path $output 'deeznats-prototype.exe') -ArgumentList @('--appearance-profile',('"'+$AppearanceProfile+'"'))
    }else{Start-Process -FilePath (Join-Path $output 'deeznats-prototype.exe')}
}
