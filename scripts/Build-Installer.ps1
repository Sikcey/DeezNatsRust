param(
    [string]$AppDirectory = '',
    [string]$EngineRoot = 'E:\DeezerNative',
    [string]$Compiler = '',
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$metadata = (& cargo metadata --manifest-path (Join-Path $project 'Cargo.toml') --no-deps --format-version 1 | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0) { throw 'Could not read the application version.' }
$appVersion = ($metadata.packages | Where-Object { $_.name -eq 'deeznats-prototype' }).version
$productVersion = ([version]$appVersion).ToString(3) + '.0'
if (!$AppDirectory) { $AppDirectory = Join-Path $project 'target/release' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $project 'artifacts/installer' }
if (!$Compiler) { $Compiler = Join-Path $project '.local/installer-tools/nsis-3.11/makensis.exe' }
foreach ($required in @($Compiler, (Join-Path $AppDirectory 'deeznats-prototype.exe'), (Join-Path $AppDirectory 'libdeezercore.dll'), (Join-Path $EngineRoot 'vendor/opendeezer/go.mod'))) {
    if (!(Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing build input: $required" }
}
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$AppDirectory = (Resolve-Path -LiteralPath $AppDirectory).Path
Add-Type -AssemblyName System.IO.Compression
$archivePath = Join-Path $OutputDirectory 'DeezNats-Source.zip'
$stream = [IO.File]::Open($archivePath, [IO.FileMode]::Create)
$archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create)
function Add-SourceFile([string]$File, [string]$Name) {
    $entry = $archive.CreateEntry($Name.Replace('\', '/'), [IO.Compression.CompressionLevel]::Optimal)
    $inputStream = [IO.File]::OpenRead($File)
    $outputStream = $entry.Open()
    try { $inputStream.CopyTo($outputStream) } finally { $inputStream.Dispose(); $outputStream.Dispose() }
}
try {
    foreach ($directory in @('crates', 'assets', 'themes', 'scripts', 'installer', 'docs', 'vendor', 'patches')) {
        Get-ChildItem -LiteralPath (Join-Path $project $directory) -Recurse -File |
            Where-Object { $_.FullName -notmatch '[\\/]__pycache__[\\/]' -and $_.Extension -notin @('.pyc', '.pyo') } | ForEach-Object {
            Add-SourceFile $_.FullName ('DeezNatsRust/' + $_.FullName.Substring($project.Length + 1))
        }
    }
    Get-ChildItem -LiteralPath $project -File | Where-Object { $_.Name -in @('README.md', 'NOTICE.md', 'SOURCE-AND-DEPENDENCIES.md', 'CHANGELOG.md', 'CONTRIBUTING.md', 'SECURITY.md', 'THEMES.md', 'Cargo.toml', 'Cargo.lock', 'LICENSE', '.gitignore') } | ForEach-Object {
        Add-SourceFile $_.FullName ('DeezNatsRust/' + $_.Name)
    }
    $engineSource = Join-Path $EngineRoot 'vendor/opendeezer'
    Get-ChildItem -LiteralPath $engineSource -Recurse -File | Where-Object { $_.Extension -notin @('.dll', '.exe', '.log') } | ForEach-Object {
        Add-SourceFile $_.FullName ('DeezerNative/vendor/opendeezer/' + $_.FullName.Substring($engineSource.Length + 1))
    }
    foreach ($relative in @('LICENSE', 'NOTICE.md', 'docs/BUILDING.md')) {
        Add-SourceFile (Join-Path $EngineRoot $relative) ('DeezerNative/' + $relative)
    }
} finally { $archive.Dispose(); $stream.Dispose() }
& $Compiler /V2 "/DPROJECT=$project" "/DPAYLOAD=$AppDirectory" "/DOUTPUT=$OutputDirectory" "/DAPPVERSION=$appVersion" "/DPRODUCTVERSION=$productVersion" (Join-Path $project 'installer/DeezNats.nsi')
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
@((Join-Path $OutputDirectory 'DeezNats-Setup.exe'), $archivePath, (Join-Path $AppDirectory 'deeznats-prototype.exe'), (Join-Path $AppDirectory 'libdeezercore.dll')) |
    ForEach-Object { Get-FileHash -LiteralPath $_ -Algorithm SHA256 } | ForEach-Object { $_.Hash + '  ' + [IO.Path]::GetFileName($_.Path) } |
    Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS.txt') -Encoding ASCII
Get-Item (Join-Path $OutputDirectory 'DeezNats-Setup.exe'), $archivePath | Select-Object FullName, Length
