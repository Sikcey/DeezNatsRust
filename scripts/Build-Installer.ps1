param(
    [string]$AppDirectory = '',
    [string]$Compiler = '',
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$metadata = (& cargo metadata --manifest-path (Join-Path $project 'Cargo.toml') --no-deps --format-version 1 | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0) { throw 'Could not read application version.' }
$appVersion = ($metadata.packages | Where-Object { $_.name -eq 'deeznats' }).version
$productVersion = ([version]$appVersion).ToString(3) + '.0'
if (!$AppDirectory) { $AppDirectory = Join-Path $project 'target/release' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $project 'artifacts/installer' }
if (!$Compiler) { $Compiler = Join-Path $project '.local/installer-tools/nsis-3.11/makensis.exe' }
foreach ($required in @($Compiler, (Join-Path $AppDirectory 'deeznats.exe'), (Join-Path $AppDirectory 'libdeezercore.dll'))) {
    if (!(Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing build input: $required" }
}
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$AppDirectory = (Resolve-Path -LiteralPath $AppDirectory).Path
$engineHash = (Get-FileHash -LiteralPath (Join-Path $AppDirectory 'libdeezercore.dll') -Algorithm SHA256).Hash
if ($engineHash -ne 'C86910F53E259B6DE3D9C94DE548010289998E89B25B430F75CFEFAADEAB488C') {
    throw 'Engine differs from v0.1.6; update the source and license information before packaging.'
}
& $Compiler "/DPROJECT=$project" "/DPAYLOAD=$AppDirectory" "/DOUTPUT=$OutputDirectory" "/DAPPVERSION=$appVersion" "/DPRODUCTVERSION=$productVersion" (Join-Path $project 'installer/DeezNats.nsi')
if ($LASTEXITCODE -ne 0) { throw 'Installer build failed.' }
@((Join-Path $OutputDirectory 'DeezNats-Setup.exe'), (Join-Path $AppDirectory 'deeznats.exe'), (Join-Path $AppDirectory 'libdeezercore.dll')) |
    ForEach-Object { Get-FileHash -LiteralPath $_ -Algorithm SHA256 } | ForEach-Object { $_.Hash + '  ' + [IO.Path]::GetFileName($_.Path) } |
    Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS.txt') -Encoding ASCII
Get-Item (Join-Path $OutputDirectory 'DeezNats-Setup.exe') | Select-Object FullName, Length
