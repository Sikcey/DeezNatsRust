# Connect audio streaming

Browser listening requests `/api/stream?format=mp3&buffer_ms=1200` (the requested
startup buffer varies by device and recovery state). The host encodes its
44.1 kHz stereo PCM to 192 kbps constant-bitrate, joint-stereo MP3 using LAME.
The browser's native audio decoder plays the response. This uses approximately
86 MB/hour per listener before HTTP/network overhead, compared with 635 MB/hour
for uncompressed PCM. MP3 is lossy; the host's local playback remains unchanged.

Each listener has its own continuous encoder in its HTTP worker. No external
encoder executable or runtime DLL is required. A slow network or encoder cannot
block the capture thread or host playback. Pending PCM is bounded to three
seconds per listener; sustained backpressure disconnects that listener instead
of silently dropping samples. Encoding CPU cost increases with listener count.

The default `/api/stream` response remains 44.1 kHz stereo, signed 16-bit
little-endian WAV for existing clients. `format=mp3` returns `audio/mpeg` without
a WAV header. Both responses are live, non-seekable, close-delimited streams.
The browser's existing seek recovery opens a fresh connection and encoder.

Browser state polls use `/api/state?queue=0` to avoid resending the whole queue
every 500 ms. `/api/queue` is fetched independently when `queue_revision` changes;
queue downloads do not delay playback labels. The default `/api/state` response
still includes the queue for existing clients. JSON and page responses prohibit
caching. Queue artwork loads lazily rather than downloading every cover at once.

Confirmed track/provider and pause changes discard the browser's old audio buffer
and start a fresh stream. Seekable browser buffers more than five seconds behind
their received live edge are advanced to retain two seconds of headroom. Some
browsers cannot seek within a live MP3 stream; those still reset on track changes.

Compression lowers network bandwidth; it cannot repair missing source PCM.

YouTube uses an in-process decoder tap, normalized to the same wire format before
host volume and host-only EQ. Its visualizer has an independent analysis worker
and does not require an enabled Connect server. The render callback never waits
for listeners: a bounded 200 ms tap backlog drops oldest packets if the pump
stalls, and analysis is best effort. Provider changes invalidate old source
packets and disconnect listeners; seek and source replacement clear tap backlog.
YouTube remote catalog and controls remain unavailable. Routing tests use
synthetic decoded samples; they do not establish live account/browser playback.
The tap follows decoder consumption by the local renderer, so playback still
requires a working local output endpoint; this is not a separate headless player.

Spotify still requires capture from the host's supervised active Soloist player.
Spotify uses the same MP3 encoder, queue-independent state polling and browser
stream-reset behavior as Deezer. Unlike Deezer's pre-volume engine PCM tap,
Soloist process capture contains its rendered volume adjustment. Spotify host
volume and render-session mute therefore also affect Connect listeners. Current
Soloist Windows CLI/control interfaces expose no pre-volume PCM output. Full
volume independence requires a different source/output integration; boosting
captured samples cannot recover audio when the host has muted it.
Real-device browser playback and mobile-data stability need interactive testing;
encoder round-trip tests alone do not establish those results.
