# Spotify provider

The Spotify provider shares the native pages, rows, artwork/page disk cache,
lyrics and player controls with Deezer. Spotify Home, discovery, library,
recent history and entity pages use the authenticated Web Player session;
the optional public API client remains available through advanced setup.
Soloist owns playback separately from catalog sign-in. Downloads are hidden
because Soloist does not provide a supported offline-download interface.
No librespot dependency or playback engine is used.

## Sign in

Select Spotify and choose **Log in**. A private WebView2 window captures the
secure `sp_dc` session without requiring a developer Client ID. Credentials
are protected with current-user Windows DPAPI. Web Player token/client-token
refresh and operation discovery run in an isolated hidden WebView2 session;
API tokens and cookies are never returned to Connect guests.

CLI equivalents are `--spotify login-web` and the read-only
`--spotify web-doctor`, `--spotify devices-doctor` and `--spotify lyrics-doctor`.
Diagnostics print only operation status/counts and
does not start playback or modify the library. Closing sign-in cancels it.

**Advanced API setup** retains the optional OAuth flow below.

Create a Spotify Web API application and register this exact redirect URI:
`http://127.0.0.1:43821/spotify/callback`.
Set `DEEZNATS_SPOTIFY_CLIENT_ID` locally to its public client ID, then run:

```powershell
cargo run -p deeznats -- --spotify login
```

A private WebView2 window handles API authorization with PKCE and simultaneously
captures the secure Spotify `sp_dc` cookie. Both grants must complete before they
are saved together with current-user Windows DPAPI protection. The OAuth callback
is intercepted inside WebView2; its exact origin/path and random state are checked.
No client secret or local HTTP listener is used. Closing the window cancels sign-in.
The reusable `login_with` function also accepts a cancellation flag for a worker.

Sign-in requests profile, playback, saved-library, and private/collaborative playlist
read scopes. **Sign in again if your saved grant predates these library scopes.**
`--spotify refresh` renews an expiring token; catalog calls do this automatically.
A rejected catalog token is refreshed and retried once. Rotated refresh tokens are
preserved under a shared lock; protected session files are replaced atomically.
`--spotify logout` removes the API/web grant without touching Soloist pairing.
The captured cookie is retained separately from Soloist and is not passed to it.

## Provider behavior


Page cache keys include the provider and account; switching providers discards
stale replies, queues and view state. Spotify audience and stream counts appear
only when returned by the service. Genre/language mixes use matching real Spotify
mix playlists. Spotify Flow uses local mood and genre selections, rather than
editing Spotify's personalized mixes. In Customize Flow, choose a mood and any
genres, then Save and use.
The picker starts with 30 broad genres. See more expands to a curated list of
60 recognizable genres and subgenres; See less collapses it again. Search
covers the full bundled Every Noise index, including genres outside the curated
list. The menu displays `Search for more genres` above the search field. Selected genres remain visible when collapsed. An unlisted genre can be
selected with Use genre from the search field.
Saved custom genres remain available when reopening the picker.

Matching Spotify editorial playlists supply 10 tracks initially; See more
appends the next batch of up to 10 unique playable
tracks. Sources rotate between playlist pages, and pagination ends when they
are exhausted. Mood-only selections are supported. This requires Spotify web
sign-in. Genre selections with All moods also use the corresponding Every Noise
genre radio playlists; no matching readable sources produce an explicit error.
The bundled genre names and radio links come from the
[Every Noise genre index](https://everynoise.com/everynoise1d.html), retrieved
2026-11-11, in `assets/spotify-flow-genres.tsv`. This is a snapshot; playlist
availability depends on Spotify and the listener's region.

Style Mix (Tarz Karması) follows the same country/language-first selector as
Deezer. Each button list starts with three rows; See more adds three rows per
click, and See less collapses the fully expanded list. Genre choices appear immediately. Spotify searches for matching playlists after
a genre is selected; availability depends on the account and region. For language
choices, tracks must also match Spotify's lyrics language metadata; playlist
titles alone do not establish a track's language. Tracks with missing or different
lyrics language are skipped, so mixes may be shorter or unavailable. Language
lookups are cached, with at most 60 candidate tracks checked per mix and no new
lookups started after 20 seconds. Country choices with multiple languages retain
playlist-based matching.

The native lyrics source follows the selected provider: Spotify uses its
authenticated timed-lyrics service, while the existing optional lyric sources
and timing-quality selection remain shared.

Spotify catalog, Connect controls, and queue edits use native HTTP requests.
WebView2 runs only for login or an explicit credential refresh. Local playback
is sent to the paired Soloist device; queue edits preserve the current track
and reject stale device or queue state.

The player **Devices** menu lists Spotify Connect devices and offers the
DeezNats Connect toggle and settings. DeezNats Connect is session-only: saved
enablement is ignored, and the server stays off until explicitly enabled.
Native Deezer DSP/output controls are hidden for Soloist playback.

When Soloist is the active local device, Connect can stream its audio through
Windows process-isolated loopback capture (Windows build 20348 or newer).
Capture follows the supervised Soloist process tree and starts only for an
explicit Connect audio subscription. It never falls back to recording system
audio. Device/provider changes stop the old stream. A quiet synthetic-process
integration test verifies that unrelated audio is excluded.

The player **Devices** button opens Spotify Connect in the same docked right-hand
panel as the queue. The page resizes beside it; it stays open during navigation
and uses the queue's scrollbar, wheel scrolling, and resize rules. Opening the
queue replaces devices, and opening devices replaces the queue. Its pinned
playback card identifies the active device and whether playback is paused.
Choose another device below it; the sidebar has no Open Spotify or Play on this
computer shortcut. Devices and Jam members scroll independently of the page.
Jam controls are directly in this sidebar (also in **Settings > Integrations**):
start or end a Jam, copy an invite link, refresh the participant list, remove
guests as the host, or leave as a participant. These controls use direct HTTPS requests
and renew authorization from your saved, DPAPI-protected Spotify sign-in;
they do not launch the official Spotify app or WebView2. Playback remains on your
active Spotify device, including an already paired Soloist instance.

Jam uses Spotify's private social-connect protocol. A Spotify protocol change
may require a DeezNats update. Failed or uncertain mutations are not retried
automatically; refresh the Jam before trying again. Existing sign-in and Soloist
setup are separate from Jam management.

`--spotify jam-doctor` performs a read-only Jam check and reports only status,
ownership and participant count. `--spotify jam-lifecycle-check` explicitly
creates a real Jam, generates an invite without sharing it, then ends the Jam.
It refuses to modify an existing active Jam. This opt-in check does not start
playback or invite participants.

## Catalog commands

Prefix each command below with `cargo run -p deeznats -- --spotify`:

- `profile`
- `search "search words" track` (also album, artist, playlist, or episode)
- `playlists`, `saved-tracks`, `saved-albums`
- `playlist PLAYLIST_ID`, `album ALBUM_ID`
- `item track TRACK_ID` (also album, artist, playlist, or episode)

Commands return JSON. Library/playlist/album commands fetch one page and accept
`next` on stdin for another page; any other input or EOF ends paging. The reusable
catalog client also supports search pagination. IDs must be 22-character Spotify
IDs. Metadata preserves Spotify URIs rather than mixing them with Deezer IDs.
Removed/null/local items without a usable identity are skipped and counted.
Pagination URLs must remain on the HTTPS Spotify API origin before any credential
is loaded or sent. Responses and requests are bounded. HTTP 429 honors numeric
`Retry-After` with an in-memory cooldown on the client instead of sleeping/retrying.
Forbidden, unavailable, expired-auth, transport, and malformed-response failures
remain distinct. Raw service error bodies and tokens are never printed.

Spotify controls which endpoints an application can access. The current playlist
items endpoint restricts reads to owned/collaborative playlists; this backend
reports denied access rather than trying to bypass it. See the
[official endpoint documentation](https://developer.spotify.com/documentation/web-api/reference/get-playlists-items).

## Setup from the app

Open **Settings > Providers**, then select **Set up Spotify**. Sign in to Spotify
in the isolated WebView2 window and accept any terms Spotify presents. Setup opens
the Soloist key page, generates a personal key if needed, and captures the key and
secure `sp_dc` cookie. It downloads and installs the official Windows x64 build
over HTTP in the background, saves the configuration using current-user DPAPI,
and finishes automatically once the account and installation are ready.

If automatic key capture is unavailable, the setup window retains the personal-key
field and **Finish setup** button as a fallback. **Download Soloist** and **Use
local ZIP / EXE** also remain available for manual recovery. Keys and cookie
contents are not displayed in status messages or logs.

After setup, the app starts Soloist, displays a pairing hint, and opens Spotify
when first pairing is needed. Select DeezNats in Spotify's device picker once;
the provider card changes to paired/ready. Subsequent starts restore the saved
pairing. An existing API grant is preserved; API logout removes the API grant and
captured web session without unpairing Soloist.

The hidden WebView2 session loads the catalog and Spotify Connect control service;
it is not the app's audio output and is excluded from playback targets. Other
browser devices may still appear as remote Connect targets. An **Active device**
label identifies Spotify's current target, not necessarily this computer.
Signing in to that browser session does not pair Soloist. If local audio is silent
and setup says to pair DeezNats, use Spotify's desktop or phone app on the same
network to select **DeezNats** once.

Playback in DeezNats defaults to local Soloist. A remote device is controlled only
after you explicitly select it in the Connect picker; the app does not follow an
unrelated active browser automatically. **Play on this computer** returns to the
local route. A paired, inactive Soloist is activated before a new local Play
request is dispatched. An unpaired Soloist shows pairing instructions instead of
silently sending that request to another device.

Local track and collection playback uses a DeezNats-managed queue over Soloist's
single-URI controls. Collection order, duplicate occurrences, selection, queue
edits, shuffle and repeat stay in DeezNats; editing upcoming items preserves the
current track's position and paused state. Natural completion starts the next
managed item after Soloist reports completion. This queue is local to DeezNats
and is not mirrored into Spotify's native queue. To replace or reorder a queue
started in another Spotify client, first start a track or collection in DeezNats.
No background WebView2 player is started for queue control or sender registration.

## Soloist setup

Download Windows x64 Soloist yourself from [Spotify's official downloads](https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates).
We do not bundle, redistribute, or automatically download it. Generate your own
Soloist API key in Spotify Developers; never share or commit it.
You can import and activate the official ZIP using the commands below, or set
`DEEZNATS_SOLOIST_PATH` to an existing executable's absolute path.
Set `DEEZNATS_SOLOIST_API_KEY` locally to your personal key. Do not paste secrets into chat.

The environment key overrides the saved setup key; update or remove it if setup
keeps using an old key. Empty values are ignored and surrounding whitespace is
trimmed. Use a **Soloist personal API key**, not a Spotify Web API client ID or
client secret. Startup errors identify recognized key rejection, expired builds,
directory access, network, and local API failures, with an exit code when available.
Raw Soloist diagnostic output is kept in bounded memory and is not displayed or
written to application logs.
The documented API-key argument may be visible to local process inspection; raw
daemon logs and command lines are suppressed. The key is removed from child
process environments.

Optionally run `--spotify configure` to save the resolved executable and your key
with DPAPI protection. The executable is resolved from an explicit environment
path, the activated managed build, a saved existing path, then discovered PATH/app
candidates. An environment key overrides a saved key independently, so a managed
installation only needs the key variable for initial configuration. You can remove
the environment variables afterward. An explicitly empty key is rejected.
`--spotify forget-soloist-config` removes this saved configuration only.

```powershell
cargo run -p deeznats -- --spotify run
```

The app starts Soloist hidden, with the WebSocket interface bound to `127.0.0.1`
on an automatically chosen port. It waits for matching process-ID discovery files
and a working local API. Another DeezNats session cannot own the same Soloist data
directory simultaneously. Child processes belong to Windows jobs so app exit or
crash closes the owned processes without killing unrelated Spotify instances.

In Spotify on the same network, choose **DeezNats** in Spotify Connect to pair it.
Premium is required for initial setup. API login and cookie capture do not replace
this independent pairing. Soloist can be tested independently of API sign-in.

Playback commands on stdin:

- `play spotify:track:TRACK_ID` (also episode, album, or playlist URI), `play`, `pause`
- `next`, `prev`, `seek 30000`, `volume 50`
- `shuffle on`, `shuffle off`, `repeat off`, `repeat context`, `repeat track`
- `add-to-queue spotify:track:TRACK_ID`, `activate`, `deactivate`
- `status`, `now`, `queue 20` (maximum 80), `snapshot`
- `quit` or EOF stops the owned daemon and event reader.

Controls are validated for both text and typed callers. Each control subprocess
has a three-second timeout and a 2 MB output limit. A successful control means
accepted/dispatched; playback state changes only from Soloist's events.
The event reader uses documented `soloist ctl trace` output. `snapshot` returns
cached authentication, current item/context, progress, duration, volume, options,
actions, queue entries, connection state, error, and revision. Position advances
using monotonic time while playing and clamps to duration. Logout clears stale
track/queue state; disconnect freezes progress. Snapshot polling reconnects a
failed event stream without restarting audio. Broadcast queue events contain only
the engine's limited window; use `queue` for an explicit larger query.

## Pairing automation

Spotify documents initial login through its app's Spotify Connect device picker.
There is no documented unattended first-pairing endpoint. Web API playback transfer
is not used as a substitute, and the captured `sp_dc` cookie is never injected into
Soloist. The app automates startup, gives an existing session time to restore,
opens Spotify when manual selection is needed, and observes Soloist's `auth_state`
until pairing completes. Normal startup preserves the session; it does not run
`--pair`, which would clear it. Authentication readiness is separate from active
playback, so connecting does not forcibly take playback from another device.

On later app launches, select Start Spotify on the provider card. The saved
Soloist session restores without another manual pairing. The app does not start
Spotify merely because it launches. Closing DeezNats stops its owned Soloist
process; reopening setup stops the supervisor before changing the installation.
Startup and reconnect failures appear on the provider card, with setup available
again. A failed event connection is bounded rather than leaving setup busy forever.

## Local installation and update automation

Prefix these commands with `cargo run -p deeznats -- --spotify`:

1. `soloist-open-downloads` opens Spotify's official download instructions in your
   default browser. Download the Windows x64 ZIP yourself.
2. `soloist-import "C:\Users\You\Downloads\soloist_release_windows_x86_64.zip"`
   validates and stages that local archive. It returns an installation `id`.
3. `soloist-activate ID` selects that installation. Set your personal key locally,
   run `configure` once, then use `run` normally.
4. `soloist-open-key` opens Spotify's personal Soloist key dashboard. Each user
   must generate their own key; initial setup requires Premium.

Additional commands:

- `soloist-discover` lists environment, managed, saved, PATH, and app-directory
  executable candidates. Discovery reads PE headers and never runs candidates.
- `soloist-check [EXE]` executes `--version` on the selected/resolved executable,
  reporting its hash, output, recognized build date, and estimated expiry. A key
  is not needed to check a build.
- `soloist-doctor` returns a JSON setup report, saved-key presence (never its value),
  discovered candidates, managed builds, active build, errors, and setup links.
- `soloist-rollback` reactivates the previous usable managed build.

On the first app launch each UTC day, a background HTTP check downloads the current official Windows build for managed installations. A newer dated build is imported and activated when playback is idle; if Soloist is running, activation is retried on the next launch. The commands above also support manual updates.
Activation uses an atomic manifest and refuses to change builds while a managed
playback session holds the data-directory lock. Old builds stay available for
rollback; expired builds cannot be activated. No account/session/cache files are
replaced. An explicit `DEEZNATS_SOLOIST_PATH` overrides the managed selection;
remove that override to use activated builds.
An executable explicitly selected through **Use local ZIP / EXE** is also reused
ahead of managed builds. Select a managed ZIP in setup to switch back.

Managed packages live in `spotify-soloist-install/builds/ARCHIVE_SHA256`, separate
from `spotify-soloist`, which holds pairing and cache state. All package companion
files and license notices are retained. Imports are bounded to a 512 MB ZIP,
2,048 entries, 256 MB per file, 768 MB expanded total, and a 30-second extraction
window. Unsupported ZIP64, multi-volume archives, encrypted entries, non-ASCII
or unsafe Windows paths, symlinks, filename collisions, and CRC failures are
rejected. Failed staging is cleaned up; the active installation remains selected.

Import and check run the user-selected local executable with `--version`. Use
only a trusted official Spotify download. The importer checks Windows x64 PE
format and subsequent executable integrity with SHA-256; these checks do **not**
authenticate Spotify's publisher signature. Expiry is estimated from an explicit,
recognized build timestamp in version output. Unknown formats stay `unknown`,
and Soloist's own expired-build exit code remains authoritative.

Distribution follows Spotify's [download guidance](https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates):
the app does not host, bundle, redistribute, or patch Soloist binaries. On a fresh setup, an HTTP request downloads the official Windows build and imports
it locally. The download button can retry the download or obtain a newer build.
The command-line helper opens that page in the default browser.
Personal keys are never shared, generated by the app, or included in releases.
Initial pairing still requires choosing DeezNats in Spotify Connect.

## Backend integration points

- `spotify::login_with(client_id, cancel)` and `spotify::logout()`
- `spotify::api::Client::saved()` for profile, search, libraries, items, and pages
- `spotify::soloist::Config::{load, save}` and `Soloist::start(config)`
- `Soloist::control(&Control)` and `Soloist::snapshot()`
- `spotify::service::Service::{start, status}` for nonblocking pairing supervision
- `spotify::setup::{discover, import_archive, activate, rollback, check, doctor}`

These are blocking worker-thread APIs with no dependency on UI types. Nothing
starts, signs in, captures cookies, or loads credentials on ordinary app startup.
No UI files need to change to test these backends.

## Limitations and verification

- Soloist builds expire after 90 days. Exit code 10 reports an expired build;
  install a new official build manually.
- Soloist renders through Windows WASAPI, outside the Deezer audio pipeline.
  DeezNats restarts Soloist when the Windows default output changes and waits
  for an output to return if the device disconnects. It attempts to restore
  active local playback after reconnecting.
  **Reconnect audio output** remains an explicit fallback for a stuck output.
  It restarts Soloist and restores active local playback after pairing returns,
  preserving the confirmed track and queue on screen during recovery.
  Manual recovery briefly interrupts playback. Windows per-app output overrides
  still apply.
  Spotify Playback settings offer a **Windows Volume mixer** shortcut: while
  local playback is active, find Soloist and choose its output there. Remote
  Spotify Connect devices manage their own output.
- Soloist exposes no EQ, normalization, crossfade, or offline-download control.
  These capabilities remain disabled. Its volatile playback cache is not an
  offline library. See the [Soloist CLI](https://developer.spotify.com/documentation/soloist/reference/command-line)
  and [WebSocket API](https://developer.spotify.com/documentation/soloist/reference/websocket-api).
- Soloist owns its paired session under `spotify-soloist` in app data, with a
  separate playback cache capped at 512 MB. Its state is outside the app's DPAPI
  API/web credential file. API logout and forgetting configuration do not unpair it.
- Offline tests use mock HTTP responses and a compiled local Soloist stand-in.
  They cover auth retry, rate limiting, safe pagination, token rotation, event
  reduction, controls, process limits, readiness, locking, expiry, reconnection,
  process-job cleanup, ZIP rejection/cleanup, staged updates, rollback, receipt
  integrity, architecture checks, expiry boundaries, and preservation of paired
  state. The stand-in never contacts Spotify or produces audio.
- Live API access, combined browser sign-in, actual Soloist event compatibility,
  pairing, and audible playback still need your registration/account/key/build.

Run backend checks with `cargo test --locked -p deeznats spotify`.
The lifecycle fixture requires the Rust toolchain already used to build the app.
See [Soloist CLI](https://developer.spotify.com/documentation/soloist/reference/command-line),
[control commands](https://developer.spotify.com/documentation/soloist/reference/soloist-ctl),
and [events](https://developer.spotify.com/documentation/soloist/reference/websocket-api).
