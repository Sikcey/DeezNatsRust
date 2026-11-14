param(
    [string]$EngineRoot = 'E:\DeezerNative',
    [string]$OutputDirectory = '',
    [string]$Compiler = '',
    [string]$EngineSourceDirectory = '',
    [string]$GoExecutable = ''
)
$ErrorActionPreference = 'Stop'
if (!$OutputDirectory) { $OutputDirectory = Join-Path (Split-Path $PSScriptRoot -Parent) 'target/debug' }
$go = $GoExecutable
if (!$go) { $go = Join-Path $EngineRoot '.tools/go-sdk/go/bin/go.exe' }
if (!(Test-Path -LiteralPath $go)) { $go = (Get-Command go.exe -ErrorAction Stop).Source }
if (!$Compiler) {
    $Compiler = Get-ChildItem "$EngineRoot/.tools/compiler-fast/*/bin/x86_64-w64-mingw32-clang.exe" | Select-Object -First 1 -ExpandProperty FullName
}
if (!$Compiler) { throw 'Pass -Compiler with a Go-compatible C compiler.' }
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$output = Join-Path (Resolve-Path -LiteralPath $OutputDirectory).Path 'libdeezercore.dll'
$previousCgo = $env:CGO_ENABLED
$previousCompiler = $env:CC
if (!$EngineSourceDirectory) { $EngineSourceDirectory = Join-Path $EngineRoot 'vendor/opendeezer' }
& (Join-Path $PSScriptRoot 'Apply-EnginePlaybackFix.ps1') -EngineSourceDirectory $EngineSourceDirectory
Push-Location $EngineSourceDirectory
try {
    $env:CGO_ENABLED = '1'
    $env:CC = $Compiler
    & $go build -buildmode=c-shared -o $output ./corelib
    if ($LASTEXITCODE -ne 0) { throw 'Jam engine build failed.' }
} finally {
    $env:CGO_ENABLED = $previousCgo
    $env:CC = $previousCompiler
    Pop-Location
}
