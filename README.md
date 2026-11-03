# DeezNats Rust

A native Windows music client for Deezer, Spotify and YouTube Music, built with
Rust and Win32. Deezer playback uses a modified OpenDeezer audio engine. This is
an unofficial project and is not affiliated with those services.

## Features

- Personalized home shelves, discovery, search, artist pages, albums and playlists.
- Favorites, liked albums, podcasts, Flow and a playback queue. Upcoming queue
  items can be reordered by dragging their grip icons.
- Flow's **Style Mix** button starts with languages, checks Deezer for playable matching playlists, then shows the available genres. Pick a genre to build a mix such as Turkish Pop or Japanese Rock. Playlist labels guide the selection; individual tracks are not language-verified.
- Synchronized lyrics, equalizer, ReplayGain, gapless and crossfade controls.
- Background downloads with persistent pending jobs, automatic retries, cancellation and verified offline library entries.
  Music is stored as encrypted files in `mediacache` inside the active data folder, with playback metadata in `mediacache/index.json`; it is not exported as MP3/FLAC files. Podcast episodes use the engine download folder (`Podcasts/<id>.mp3`). Unfinished and failed tracks resume after Deezer connects on the next launch.
- Acrylic, artwork backgrounds, custom images, gradients and importable [themes](THEMES.md).
- English, Turkish and Azerbaijani interface, plus Discord presence.
- Windows media controls with track details, artwork and seeking; standard keyboard
  media keys work automatically. Optional global shortcuts can be assigned in
  Settings > Keyboard shortcuts and reset individually to the media-key defaults.
- In-app updates with binary delta downloads and restart-to-apply controls in
  Settings > General. Release update assets must be published to enable delivery.
- Connect hosting with a browser client and MP3 audio streaming. See
  [Connect audio](docs/CONNECT-AUDIO.md) for provider-specific behavior.

See [Spotify](docs/SPOTIFY.md) and [YouTube Music](docs/YOUTUBE-MUSIC.md) for
account setup, supported features and provider limitations.

Full playback and available quality depend on your account's entitlements.
Saved sessions are encrypted for the current Windows user with DPAPI.
Some features are still in development; download completion and full feature
parity with the original application are not yet verified.

## Requirements

- Windows x64 and the Rust stable MSVC toolchain.
- Visual Studio Build Tools with the C++ workload and Windows SDK.
- A matching `libdeezercore.dll` for account and playback features.
- Microsoft Edge WebView2 Runtime for embedded browser functionality.

## Build

```powershell
cargo build --release --locked --workspace
.\scripts\Run-Windows.ps1 -Release -EnginePath C:\path\to\libdeezercore.dll
```

The Rust source builds without a bundled engine. Playback requires the external
DLL, which is staged beside the executable by the launcher. The engine derives
from the [original DeezNats project](https://github.com/Sikcey/DeezNats).
See [building and testing](docs/BUILDING.md) for engine and installer instructions.

## Data and compatibility

Settings and session data live in `%LOCALAPPDATA%\DeezNatsRust`.
After moving data in Settings, this AppData folder retains `storage-location.json`
and its `.bak` recovery copy. Installed and standalone builds use this shared
pointer automatically; upgrades preserve it and your data. Uninstalling removes
the current DeezNats data folder, including its managed Soloist installation.
If the destination drive is disconnected, reconnect it before starting the app.
The application can read a current-user legacy DeezNats session or a profile
selected with `DEEZNATS_PROFILE`. Profiles and downloaded media are not included
in this repository. Importing the official Deezer application's encrypted offline
files is not supported.

## License and credits

AGPL-3.0; see [LICENSE](LICENSE) and [third-party notices](NOTICE.md).

The audio engine derives from [OpenDeezer](https://github.com/Cycl0o0/OpenDeezer).
Visual inspiration comes from [GlassLiquify](https://github.com/satomasahiro2005/GlassLiquify/tree/fork)
by satomasahiro2005 and [Spicy Lyrics](https://github.com/Spikerko/spicy-lyrics) by Spikerko.

Contributions: [CONTRIBUTING.md](CONTRIBUTING.md). Security: [SECURITY.md](SECURITY.md).
