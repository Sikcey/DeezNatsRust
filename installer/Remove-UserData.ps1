$ErrorActionPreference = 'Stop'
$defaultData = Join-Path $env:LOCALAPPDATA 'DeezNatsRust'
$manifest = Join-Path $defaultData 'storage-location.json'
$backup = "$manifest.bak"
$location = $null
foreach ($file in @($manifest, $backup)) {
    if (Test-Path -LiteralPath $file -PathType Leaf) {
        try {
            $location = Get-Content -LiteralPath $file -Raw | ConvertFrom-Json -ErrorAction Stop
            break
        } catch {
            if ($file -eq $backup -or !(Test-Path -LiteralPath $backup -PathType Leaf)) { throw }
        }
    }
}
foreach ($path in @($location.active, $location.pending)) {
    if (!$path) { continue }
    if (![IO.Path]::IsPathRooted($path) -or
        [IO.Path]::GetFileName($path.TrimEnd('\', '/')) -ine 'DeezNatsRust') {
        throw "Invalid DeezNats data location: $path"
    }
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    } elseif (!(Test-Path -LiteralPath ([IO.Path]::GetPathRoot($path)))) {
        throw "Data drive is unavailable: $path"
    }
}
foreach ($path in @($defaultData, (Join-Path $env:TEMP 'DeezNatsRustUpdates'))) {
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
}
