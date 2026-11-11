param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$AppDirectory = '',
    [string[]]$PreviousDirectories = @(),
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
if (!$AppDirectory) { $AppDirectory = Join-Path $project 'target/release' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $project 'artifacts/update' }
$AppDirectory = (Resolve-Path -LiteralPath $AppDirectory).Path
$binaryVersion = (Get-Item -LiteralPath (Join-Path $AppDirectory 'deeznats.exe')).VersionInfo.ProductVersion
if ($binaryVersion -ne $Version) { throw "Application version $binaryVersion does not match requested update version $Version." }
Copy-Item -LiteralPath (Join-Path $project 'assets/licenses/qbsdiff.txt') -Destination (Join-Path $AppDirectory 'qbsdiff.LICENSE') -Force
$arguments = @('run', '--locked', '--release', '-p', 'deeznats-updater', '--bin', 'deeznats-update-pack', '--', '--current', $AppDirectory, '--version', $Version, '--output', $OutputDirectory)
foreach ($directory in $PreviousDirectories) {
    $arguments += @('--previous', (Resolve-Path -LiteralPath $directory).Path)
}
Push-Location $project
try {
    # Cargo writes normal build progress to stderr. Windows PowerShell can turn
    # redirected stderr into error records; only Cargo's exit code means failure.
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & cargo @arguments
        $buildExit = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousErrorAction }
    if ($buildExit -ne 0) { throw 'Update package generation failed.' }
} finally { Pop-Location }
Write-Output "Publish the manifest and referenced assets from $OutputDirectory alongside the installer."
