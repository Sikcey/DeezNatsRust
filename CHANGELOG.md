# Changelog

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

Earlier Windows Forms releases are documented in the
[original project changelog](https://github.com/Sikcey/DeezNats/blob/main/CHANGELOG.md).
