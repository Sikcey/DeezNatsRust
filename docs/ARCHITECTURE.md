# Architecture

DeezNats Rust has a native Win32 interface and a Go audio engine exposed through a C ABI.

- `crates/app`: Windows application, navigation, buffered GDI+ rendering, themes,
  catalog, lyrics, downloads, session storage, Discord presence and Connect hosting.
- `crates/core`: shared playback and queue rules.
- `crates/app/src/engine_api.rs`: typed bindings to `libdeezercore.dll`.
- `crates/app/src/connect`: embedded Connect frontend and spectrum processing.
- `assets` and `themes`: icon, localization catalogs and appearance presets.
- `scripts` and `installer`: build, launch, engine compilation and NSIS packaging.

Sessions use current-user Windows DPAPI. Settings and library data are stored in
the user profile. Rendering uses bounded artwork and text caches; background work
handles network operations, downloads and image preparation. Playback and Connect
continue when the main window is inactive.
