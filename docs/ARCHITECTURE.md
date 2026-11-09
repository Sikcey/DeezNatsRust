# Architecture

DeezNats Rust has a native Win32 interface and a Go audio engine exposed through a C ABI.

- `crates/app`: Windows application, navigation, buffered GDI+ rendering, themes,
  catalog, lyrics, downloads, session storage, Discord presence and Connect hosting.
- `crates/core`: shared playback and queue rules.
- `crates/app/src/runtime/engine.rs` (`api` module): typed bindings to `libdeezercore.dll`.
- `crates/app/src/connect`: embedded Connect frontend and spectrum processing.
- `assets` and `themes`: icon, localization catalogs and appearance presets.
- `scripts` and `installer`: build, launch, engine compilation and NSIS packaging.

Sessions use current-user Windows DPAPI. Settings and library data are stored in
the user profile. Rendering uses bounded artwork and text caches; background work
handles network operations, downloads and image preparation. Playback and Connect
continue when the main window is inactive.

## Source layout

Application Rust code is grouped by responsibility under `crates/app/src/`. Related implementation modules are inline in their owning file, preserving module namespaces and visibility.

- `ui/mod.rs`: scene construction, scrolling, track information, device sidebar, drawing backends, provider logos, appearance, themes, and visualization.
- `ui/window.rs`: native window hosting, input, dialogs, popups, compositor timing, and frame pacing.
- `providers/youtube.rs`: authentication, catalog, playback, routing, proof-of-origin token handling, and browser adapter. The token JavaScript remains in `providers/youtube-potoken.js`.
- `providers/spotify.rs`: provider APIs, catalog, playback, queues, setup, web integration, and browser adapter. `providers/spotify_diagnostics.rs` remains separate because it also runs on non-Windows platforms.
- `playback/`: audio output, media controls, lyrics, and style mix.
- `library/`: shared catalog, downloads, and playlist handling.
- `storage/`: persistent settings and provider sessions.
- `runtime/`: engine bindings, updates, integration, process instance management, and diagnostics.
- `connect/`: Connect server in `mod.rs` plus its embedded HTML, CSS, JavaScript, and translations.

Use symbol search to navigate inline modules within these larger files.

Provider adapters live in `providers/` (YouTube, Spotify, Deezer, and Spotify diagnostics). UI scenes and the native window host live in `ui/`. Rust module paths stay stable through explicit source paths in `main.rs`.
