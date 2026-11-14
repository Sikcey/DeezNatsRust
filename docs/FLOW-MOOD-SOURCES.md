# Flow mood-to-genre sources

Checked 2026-11-13. Last.fm mood discovery is now integrated for Spotify and YouTube.

## Current behavior

Spotify and YouTube already share the bundled Every Noise genre/radio index in
`assets/spotify-flow-genres.tsv`: 6,291 entries, retrieved 2026-11-11. This is a
genre catalog, not a mood-to-genre classification. Their pickers now retain the
full catalog for searching, order initially selected genres first, and show
20 entries initially or 40 when expanded. Other genres are not hidden merely
because they are outside the mood defaults. Deezer's behavior is unchanged.

When sufficient evidence is available, Last.fm genre recommendations
replace local/playlist-name defaults. Otherwise local defaults and provider playlist
evidence remain the fallback. Saved edits, including an empty selection, always win.

## Every Noise

Primary sources: https://everynoise.com/ and
https://everynoise.com/everynoise1d.html

The genre index supplies names and genre-radio links, which we already bundle.
The site describes its genre map and discovery experiments, not a documented
mood-to-genres API. Fetching the index alone would not replace classification.
Scraping also introduces layout and availability dependencies.

The homepage's 2024-01-05 note says much of the site became a static snapshot
after loss of internal Spotify access. Later notes describe separately maintained
research features. The 2026-08-07 API outage is not the latest status: the
2026-09-04 note says access was restored after an appeal. This does not establish
a guaranteed live mood API for the historical genre map.

## Last.fm integration

Primary documentation:

- https://www.last.fm/api/show/tag.getSimilar
- https://www.last.fm/api/show/tag.getTopArtists
- https://www.last.fm/api/show/artist.getTopTags
- https://www.last.fm/api/tos

These endpoints require an API key, but not an authenticated user session.
The API terms grant a no-charge license for non-commercial use, subject to their
conditions. Commercial use requires a separate agreement with Last.fm; do not
assume a free application is automatically a non-commercial use case.
Similar tags are documented as ranked from listening data. Top artists for a
tag are ordered by tag count; artist top tags are ordered by popularity.

Implemented pipeline:

1. Query two explicit mood tags, e.g. focus / study, with `tag.getTopArtists`.
2. Deduplicate artist names case-insensitively and fetch at most eight artists'
   `artist.getTopTags` responses (at most ten API requests per refresh).
3. Match the first 20 tags exactly against normalized known genres, with limited
   spelling aliases. Exclude explicit mood/activity labels such as focus and sleep,
   even if the bundled index contains them. Do not infer genres from substrings.
4. Score tags by reciprocal rank, counting each genre once per artist. Require
   two different artists and keep at most eight genre recommendations.

Requests are spaced at least 300 ms apart across refreshes. A refresh starts no
new requests after 12 seconds; individual requests use the existing bounded HTTP
transport. The catalog worker waits at most eight seconds before using fallback;
an already running fetch may finish and populate the cache for the next opening.
Retries for the same mood are throttled for one minute. Cache entries are shared
across Spotify/YouTube, versioned and fresh for 24 hours; stale entries are usable
for seven days on failure. No new crates or user-authentication flow are needed.

`tag.getSimilar` is not used: artist/tag co-occurrence supplies the evidence.
This is community-tag inference, not a scientifically verified mood classifier.
Sparse or biased tags can still yield imperfect suggestions. Review API terms,
especially for commercial use, before shipping. The app-owned Last.fm API key is
bundled for zero-setup use. It is extractable from the desktop binary and shares
the application's API quota. The Last.fm shared secret is not bundled.

### Zero-setup use and optional overrides

End users do not need a Last.fm account or API key: builds include the app-owned
key. Resolution order is environment override, saved DPAPI key, then bundled key.
An explicitly invalid override is rejected rather than silently replaced.

To override the bundled key, save another API key in a private temporary UTF-8 file
outside the repository, then run:

```powershell
.\target\debug\deeznats.exe --lastfm configure C:\private\lastfm-key.txt
.\target\debug\deeznats.exe --lastfm probe focus
```

Configuration validates the key format and saves it using current-user Windows
DPAPI in the active application data directory (`lastfm-lyrics.dat`, following
the existing provider credential naming convention). Delete the plaintext
temporary file after configuration. The shared secret is not used. The optional
`DEEZNATS_LASTFM_API_KEY` environment variable overrides protected storage; do not
put a private override in tracked files, release archives or command-line arguments. Probe output
contains genre scores and artist support counts, never credentials. A successful
probe also warms the mood cache. Restart an already-running app after rebuilding.

Offline ranking, deduplication, fallback and saved-selection tests run normally.
The ignored `lastfm::tests::live_lastfm_mood_discovery` test uses protected storage;
optionally `DEEZNATS_LASTFM_KEY_FILE` points to a private file to configure it first.
It verifies real mood evidence, the cache round trip and the shared tuner enrichment.

## MusicBrainz

Primary source: https://musicbrainz.org/doc/Genre

MusicBrainz treats an approved subset of community tags as genres, with user
votes on entities. It can validate taxonomy or enrich artist/recording metadata,
but its documented genre system is not a direct mood-to-genre API.

## Recommendation

Keep Every Noise for the catalog, Last.fm for mood evidence, and local/provider
defaults for failures or insufficient evidence. Do not scrape thousands of genres
or put unbounded artist requests on the Flow picker path. Deezer remains unchanged.
