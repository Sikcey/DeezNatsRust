# Changelog

## 0.1.7

- Improve Spotify catalog, playback recovery, diagnostics and interface behavior.
- Improve YouTube catalog handling and Azerbaijani and Turkish translations.

## 0.1.6

- Reorganize application source and assets, improve localization and update packaging.

## 0.1.5

- Add YouTube Music search, full discovery shelves, collection browsing, playback controls and queue editing to Connect.
- Connect YouTube Flow, genre tuning and Style Mix to the host's radio generators.
- Improve YouTube seeking and mood/genre radio recommendations.
- Fix early Apple TTML lyric timestamps, playlist menu scrolling and input isolation.
- Improve Style Mix search, catalog layout, hover rendering and playlist membership checks.

## 0.1.4

- Add staggered lyric scrolling, fading viewport edges and clear lyrics during manual scrolling.
- Keep lyric motion continuous across fast verses and prevent playback-clock freezes between sparse updates.
- Improve playlist editing and copying, track information, catalog updates and artwork handling.
- Improve YouTube Music session persistence and playback, Spotify integration and Connect handling.
- Include the modified YouTube Music dependency and playlist engine patch in source distributions.

## 0.1.3

- Fix the queue drag handle displaying a question mark instead of the grip icon.

## 0.1.2

- Add YouTube Music account, catalog and playback integration.
- Add MP3 streaming and more efficient state and queue updates for Connect listeners.
- Reuse the GPU device during window resizing and remove the debug paint FPS counter.
- Improve catalog, artwork, playback and localization handling.

## 0.1.1

- Render search and playlist text fields directly in the application canvas so text, selections and the caret remain visible under DirectComposition.
- Add Unicode-aware editing, clipboard commands, undo/redo, input method composition, keyboard navigation and multiline playlist descriptions.
- Preserve playlist drafts through cover editing and keep FlowTune filtering live as search text changes.
- Reduce allocations when resizing large playlist and album artwork.
- Include Spotify integration, GPU rendering, in-app updates and smoother visualization from the preceding application updates.

## 0.1.0 ? Rust edition

- Introduce a native Rust Windows application with Win32 controls and GDI+ rendering.
- Add catalog navigation, queue playback, lyrics, themes and session persistence.
- Integrate the existing OpenDeezer-based playback engine through its C ABI.
- Include Connect hosting and a standalone native Connect client.
