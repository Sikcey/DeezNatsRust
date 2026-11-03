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

Flow searches YouTube playlists for each selected mood/genre combination (genre
radio is not implemented), interleaves playable songs, and removes duplicate IDs.
If playlist lookup fails or returns no playable songs, song search uses the same
mood/genre query. Each batch processes up to four genres and rotates through all
selected genres and deeper playlist results on continuation. Playback near the
queue's end retains the selection rather than switching to an unrelated station.
Continuation skips already queued tracks, tries up to three batches, and reports
an error if no new tracks are found. Requests have time limits; partial successful
genres remain playable when another genre fails. YouTube search ranking and
playlist metadata determine relevance; strict genre membership is not guaranteed.
An explicitly opened song radio continues to use YouTube automix.

Both mixes require available songs with artist and album metadata and a duration
between 30 seconds and 15 minutes. This excludes many unrelated videos and long
compilations, but can also omit legitimate songs with incomplete metadata.
Style Mix uses the selected region/language and genre to search playlists
and combine their songs, with song search as a fallback. This follows YouTube's
search ranking; it cannot guarantee the language of every result.

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
and queue display; remote catalog and controls remain unavailable.
The audio output follows Windows' default device
when the output stream is opened.

## Verification

Run `cargo check -p deeznats-prototype` and `cargo test --workspace`. The explicit
network check is:

```powershell
cargo test -p deeznats-prototype youtube::catalog::tests::live_authenticated_catalog_and_audio -- --ignored --exact
```

That check uses a saved signed-in session, loads Home/Explore/search/radio, opens an audio
stream and decodes a full song to PCM without playing it through speakers. It
does not verify Google login, account mutations, Premium-only streams, token
attestation on a flagged network, audible device output, or interactive UI.

Flow-specific checks:

```powershell
cargo test -p deeznats-prototype flow
cargo test -p deeznats-prototype youtube::catalog::tests::live_tuned_flow -- --ignored --exact
cargo build -p deeznats-prototype
```

The offline tests cover tuning options, invalid edits, mood/genre query generation,
interleaving and deduplication, provider isolation, persistence and the shared
picker UI. The optional live check verifies playlist-backed tuned batches and
selection-bearing continuation metadata. It does not prove musical relevance or
exercise interactive playback to the end of a queue.
