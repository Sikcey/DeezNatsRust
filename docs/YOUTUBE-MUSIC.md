# YouTube Music

Select **YouTube Music** in onboarding or **Settings > Accounts**. Switching
providers stops the previous player's audio and clears the visible queue and
provider-specific catalog state. Sign in to YouTube Music to browse and play.
The YouTube account page also provides access to saved playlists, liked songs,
saved albums and personalized Home recommendations.

Sign-in uses a private WebView2 browser. If Google refuses embedded sign-in,
open music.youtube.com in your normal browser and paste the request's Cookie
header into the YouTube account page. Cookies are credentials: keep them private.
The app validates the account before saving the header through the existing
Windows DPAPI provider-session store. Session cookies rotate in memory every ten
minutes while the signed-in client is alive. This integration currently uses Google's
default account (`authuser=0`); account/brand-channel selection is not provided.
Sign out removes the saved provider session. The encrypted session stays under
`%LOCALAPPDATA%\DeezNatsRust` even when other data is relocated, and the
uninstaller removes it along with the app's browser profiles.

## Catalog and listening

Home and Explore use YouTube Music's own feeds. Explore also lists mood and genre
categories. Search returns songs, artists, albums and playlists. Album, artist and playlist
pages open in the native catalog UI. Feed and playlist continuation tokens load
additional results. Favorites lists liked songs; the library lists playlists and
saved albums. Recent lists tracks played in this application session.

Flow uses the same **Customize Flow** mood and genre picker as Spotify, including
search, additional genres and custom genre names. Apply saves a YouTube-specific
selection that survives provider changes and app restarts. Spotify and Deezer
selections remain separate. Effects are not provided.

Flow searches YouTube playlists for each selected mood/genre combination and
uses a matching song to start YouTube Music radio. Song search supplies a seed
when no playlist matches; matching search results are used if radio fails. Each
batch processes up to four genres, interleaves tracks, removes duplicate IDs, and
rotates genres and seeds on continuation. Playback retains the Flow selection,
skips queued tracks, and tries up to three batches for new tracks. Requests have
time limits. Search ranking and YouTube's song-based recommendations affect
relevance; strict genre membership is not guaranteed. YouTube does not expose a
separate genre radio through this integration. An explicitly opened song radio
also uses YouTube automix.
Both mixes require songs with artist and album metadata and a duration between
30 seconds and 15 minutes. This can omit legitimate songs with incomplete metadata.
Style Mix uses the selected region/language and genre to find a seed for YouTube
radio, with the same search fallback. Language is not guaranteed.
Playback supports pause/resume, seek, volume, next/previous, repeat, shuffle,
explicit queue insertion/reordering, Windows media controls and shared lyrics
lookup. The resolver uses the pinned ytmusic-rs client, including signature
deciphering, guest stream fallbacks and a private browser proof-of-origin minter.
Songs with a known size are buffered in memory before decoding (128 MiB limit, 60-second load
deadline). This adds startup latency compared with progressive playback.

Liked songs and playlist save/unsave use the authenticated API. Playlist creation,
rename, add/remove and delete are wired; custom covers and descriptions are not.
Offline downloads and crossfade are unavailable. Decoded audio is converted to
44.1 kHz stereo and tapped before local volume and the host-only ten-band EQ.
Connect WAV/MP3 listening and visualizers consume that pre-volume PCM even at
zero host volume. Visualizers also run with Connect disabled. No Windows endpoint
or unrelated application is recorded. YouTube Connect supports listening, state
and queue display, host-account search, the full Explore discovery feed,
album/playlist/artist and category browsing, playback controls and queue editing.
Flow, its genre tuner, and Style Mix use the host's YouTube radio generators.
Lyrics use the desktop player.
The audio output follows Windows' default device
when the output stream is opened.

## Verification

Run `cargo check -p deeznats` and `cargo test --workspace`. The explicit
network check is:

```powershell
cargo test -p deeznats youtube::catalog::tests::live_authenticated_catalog_and_audio -- --ignored --exact
```

That check uses a saved signed-in session, loads Home/Explore/search/radio, opens an audio
stream and decodes a full song to PCM without playing it through speakers. It
does not verify Google login, account mutations, Premium-only streams, token
attestation on a flagged network, audible device output, or interactive UI.

Flow-specific checks:

```powershell
cargo test -p deeznats flow
cargo test -p deeznats youtube::catalog::tests::live_tuned_flow -- --ignored --exact
cargo build -p deeznats
```

The offline tests cover tuning options, invalid edits, mood/genre query generation,
interleaving and deduplication, provider isolation, persistence and the shared
picker UI. The optional live check verifies playlist-backed tuned batches and
selection-bearing continuation metadata. It does not prove musical relevance or
exercise interactive playback to the end of a queue.
