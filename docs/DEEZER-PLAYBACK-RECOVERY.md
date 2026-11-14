# Deezer playback recovery

The native engine patch in `patches/deezer-playback-recovery.patch` addresses two
failure modes that can leave a track stuttering or stationary until a skip:

- After a PCM underrun, hold incoming audio until one second is buffered, or
  decoder EOF arrives. Resume the same source with an anti-click fade. Silence
  does not advance position or listening time. Pause/resume retains the reserve
  requirement; a replacement source does not inherit it. This leaves lifecycle
  state and the engine's queue-advance signal intact.
- Cancel an HTTP body read after five seconds without a read completing. The
  existing bounded, validated HTTP Range retry resumes the encrypted stream
  without resetting its decryptor or replaying previously decoded audio. The
  timeout applies only during network reads, excluding pause/preload waits,
  cached reads, pacing, and total track duration. It cancels one request, not the
  source context. Existing retry exhaustion and entity-validation errors remain
  errors rather than being retried indefinitely.

The manager logs PCM starvation outside the audio callback. These mechanisms
cover demonstrated engine failure modes; an intermittent real-device symptom
still requires a listening comparison. They do not restart a device whose audio
callbacks have stopped entirely.

`scripts/Build-JamEngine.ps1` applies the patch before building, skips it if
already applied, and fails if the engine source is incompatible. It accepts
`-EngineSourceDirectory` for a standalone OpenDeezer checkout and `-GoExecutable`
for an explicit Go toolchain, in addition to its existing options. Keep local
source snapshots, logs and DLL backups out of release archives.

From the engine source directory, using the matching Go and C toolchain:

```powershell
go test ./internal/audio -run 'TestStarvation|TestStreamIdle' -count=1 -timeout 60s
go test ./internal/audio ./corelib -count=1 -timeout 180s
go test -race ./internal/audio -run 'TestStarvation|TestStreamIdle' -count=1 -timeout 90s
```

Regression coverage includes trickling PCM, byte ordering and position after
recovery, short EOF tails, pause/resume, seek, stop/source replacement, request
cancellation, paused network reads, and a real HTTP server that stalls an
encrypted response and then serves a byte-exact Range continuation.
