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
is `target/release/deeznats-prototype.exe`; the Connect client is
`target/release/DeezNatsConnect.exe`. The Connect client does not require the engine DLL.

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

Build the release application and stage the matching engine DLL first. Install
NSIS and pass its `makensis.exe` path:

```powershell
.\scripts\Build-Installer.ps1 -EngineRoot C:\src\DeezNats -Compiler C:\nsis\makensis.exe
```

Outputs are written to `artifacts/installer`. The installer includes a matching
source archive and applicable notices. It installs for the current Windows user
without administrator rights and preserves user data on uninstall. Engine source
must be available when packaging; do not distribute the DLL without corresponding
source and required third-party notices.
