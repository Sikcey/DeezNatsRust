# Building and testing

Run commands from the repository root on Windows x64. Install Rust stable with
the MSVC target, Visual Studio C++ Build Tools and the Windows SDK. Cargo restores
dependencies from `Cargo.toml`; `Cargo.lock` is committed for reproducible builds.

## Application

```powershell
cargo build --release --locked --workspace
.\scripts\Run-Windows.ps1 -Release -EnginePath C:\path\to\libdeezercore.dll
```

Use `-BuildOnly` to build and stage without opening the window. The main binary
is `target/release/deeznats-prototype.exe`.

## Playback engine

Engine source is maintained in the [original repository](https://github.com/Sikcey/DeezNats),
under `vendor/opendeezer`. It is not vendored in this Rust repository.
Clone that project to a separate checkout. Install Go as required by its `go.mod`
and a Go-compatible x64 C compiler such as MinGW-w64 GCC or LLVM-MinGW.

```powershell
.\scripts\Build-JamEngine.ps1 -EngineRoot C:\src\DeezNats -Compiler C:\mingw64\bin\gcc.exe -OutputDirectory target\release
.\scripts\Run-Windows.ps1 -Release -EnginePath target\release\libdeezercore.dll
```

The build helper uses the checkout's local Go SDK if present, otherwise `go.exe`
on PATH. Its default engine location is `E:\DeezerNative`; pass `-EngineRoot`
and `-EnginePath` explicitly on other machines.

## Tests

```powershell
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Ordinary unit tests do not require a Deezer account. Account playback and Windows
rendering need separate interactive verification on Windows.

## Installer

In-app updates use verified binary delta patches. Bump the workspace version before
building a release. Keep the previous release's exact files (including the engine
DLL); rebuilding old source does not reproduce the bytes installed on users' PCs.

After building/staging the new release, generate its update assets:

```powershell
.\scripts\Build-Update.ps1 -Version 0.2.0 -AppDirectory target\release -PreviousDirectories C:\releases\0.1.0,C:\releases\0.1.1
```

Publish `deeznats-update.json` and every asset referenced by that manifest from
`artifacts/update` on the matching stable GitHub release in
`Sikcey/DeezNatsRust` (tag `v0.2.0` or `0.2.0`), alongside the installer and
corresponding source archive. Use a separate clean output directory per release.
The generator verifies each delta by reconstructing the target before publishing
its manifest. A patch is included only when smaller than the complete file.

The app checks once daily when enabled; checking never downloads application
binaries or interrupts playback. Download update stages only changed files,
using a BSDIFF40 patch for exact known source hashes and transparently falling
back to a complete changed file if needed. Restart to apply waits for the original
process to exit, verifies staged files, and replaces only those files. A failed
replacement restores earlier files from backups. Settings, sessions, downloads,
and unknown files are outside the update allowlist and are preserved. The engine
DLL is updated together with the executable when both change.

Existing releases without update assets cannot deliver in-app updates. The first
updater-enabled build must be installed normally. Updates do not create a listener
or enable Connect.

Build the release application and stage the matching engine DLL first. Install
NSIS and pass its `makensis.exe` path:

```powershell
.\scripts\Build-Installer.ps1 -EngineRoot C:\src\DeezNats -Compiler C:\nsis\makensis.exe
```

Outputs are written to `artifacts/installer`. The installer includes a matching
source archive and applicable notices. It installs for the current Windows user
without administrator rights and removes current DeezNats user data on uninstall. Engine source
must be available when packaging; do not distribute the DLL without corresponding
source and required third-party notices.

Artwork cache diagnostics can be enabled for a single launch in PowerShell:

```powershell
$env:DEEZNATS_ARTWORK_METRICS = Join-Path $env:TEMP 'deeznats-artwork-metrics.jsonl'
.\target\release\deeznats-prototype.exe
Remove-Item Env:DEEZNATS_ARTWORK_METRICS
```

The JSON Lines file records cache counts and estimated pixel bytes before and
after eviction. It contains no artwork URLs or account details. `unique_bytes`
deduplicates UI bitmap aliases, while `accounted_bytes` and `offscreen_bytes`
use the conservative per-entry accounting used for eviction. Resized bitmap
bytes are estimates and are separate from decoded UI pixels. The figures do not
cover all process memory.
