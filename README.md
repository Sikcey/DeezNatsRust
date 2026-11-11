# DeezNats Rust

A native Windows music client for Deezer, Spotify and YouTube Music, built with
Rust and Win32. Deezer playback uses a modified OpenDeezer audio engine. This is
an unofficial project and is not affiliated with those services.

## Features

- Use provider-specific Flow and Style Mix selections. Spotify and YouTube Music offer a searchable genre picker; Spotify includes the bundled Every Noise index.
- View synchronized lyrics and configure optional global shortcuts.
- Customize the interface with acrylic, artwork backgrounds, images, gradients and importable [themes](THEMES.md). The interface supports English, Turkish and Azerbaijani, with optional Discord presence.
- Host a browser client with [Connect audio](docs/CONNECT-AUDIO.md), browse and control supported providers, and stream audio where supported.

Provider support differs:

- **Deezer:** Playback uses a modified OpenDeezer engine. Downloads support persistent jobs, retries and offline playback; downloaded music is encrypted in the active data folder and is not exported as MP3 or FLAC. Podcast episodes use the engine download folder. Equalizer, ReplayGain, gapless playback and crossfade are available through the Deezer engine.
- **Spotify:** Play through a paired Soloist device using Spotify's web integration. Local Flow uses Spotify playlists and the bundled Every Noise genre radio links. Offline downloads and Deezer's audio effects are unavailable for Soloist playback.
- **YouTube Music:** Flow and Style Mix use YouTube Music recommendations. Offline downloads and crossfade are unavailable.

See [Spotify](docs/SPOTIFY.md) and [YouTube Music](docs/YOUTUBE-MUSIC.md) for setup, account requirements and provider limitations. Full playback and available quality depend on your account's entitlements. Saved sessions are encrypted for the current Windows user with DPAPI.

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
DLL, which is staged beside the executable by the launcher. See [building and testing](docs/BUILDING.md) for engine and installer instructions.

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
