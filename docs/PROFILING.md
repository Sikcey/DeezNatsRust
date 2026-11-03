# Windows diagnostics build

Deezer and Spotify content uses Direct3D-backed Direct2D rasterization with
premultiplied DirectComposition presentation, including Acrylic windows.
Shapes, gradients, glyph outlines, artwork scaling, visualizers, lyric blur,
background cropping, transitions, tint and brightness
render on the GPU. Layout, cached glyph geometry and asset decoding remain CPU
work; native editors and the custom caption retain their Win32/GDI integration.
GPU image and geometry caches are bounded. Only the native search editor's
small backdrop is read back, on full repaints. Unavailable hardware uses the
existing software renderer.
Partial repaints retain the exact Windows update rectangles, including gaps
between independent marquee updates. The GPU also intersects each draw with
the exact native viewport clips; GDI+ region scan approximations are not used.
Solid brushes, bitmap brushes, stroke styles and transform storage are reused,
and simple rectangles and lines bypass GDI+ path conversion.
Text coverage is cached in bounded GPU masks; scrolling, hover colors and theme
colors reuse those masks. Font properties are read once per font per paint.
Active animations follow compositor signals from a dedicated DWM wait thread;
the UI thread never blocks in DwmFlush. Missed frames are skipped. Main content
and queue scrolling retain independent Direct2D command lists with overscan,
then replay them with fractional translations and exact viewport clips.
Main and queue scrollbar thumbs use the same fractional presentation offsets,
with subpixel coverage instead of integer layout coordinates.
Queue rows use an overscan buffer, rebuilding only after the buffer is exhausted.
Catalog buffer changes replace only catalog nodes. Track/card rows that remain
in range keep their text, media data and measured link geometry; only entering
rows are built and measured. Sidebar, player, native controls and document
extent stay intact. Changes to data, sorting, filters or window size still
perform a normal rebuild.
High-resolution wheel input preserves the animation clock across events.
Artwork uses separate bounded fetching and decoding queues: four to eight
fetchers and one or two background-priority decoders, sized for the available
CPU count. Decoding, fallback resizing and background preparation yield CPU
priority to the UI rather than saturating it with additional decode threads.
The decoded artwork cache owns shared allocations instead of weak references,
with a 64 MiB target and resident-image pins. UI offscreen artwork uses 16/24 MiB
low/high watermarks and a five-second recent-use grace. Routine trimming waits
for scrolling to settle; the 64 MiB resident ceiling still applies, except for
visible/pinned images that alone exceed it. Minimized cleanup can release unused
worker-cache images. Resource, decode edge and pixel content determine a stable
GPU identity, so identical decodes can reuse an existing GPU texture while changed
pixels get a new identity.
Visible artwork has priority over bounded prefetch work. Navigation cancels
stale queued thumbnails and skips stale decoding results. GPU cache misses
upload at most two images per frame with a 4 MiB budget (one larger image can
make progress); remaining images resume on a following animation frame.
Deferred uploads draw a static rounded placeholder and invalidate only the
main/queue layer that missed an upload, preserving the other cached layer.
Artwork completion invalidates only its owning paint layer and visible artwork
rectangles. Prefetched images outside the layout do not repaint the window.
Upload retries repaint artwork rectangles on a separate frame timer, avoiding
full-frame native search backdrop readbacks. Catalog thumbnails display fully
opaque, so retained lists never freeze an arrival fade and thumbnail arrivals
do not start unnecessary half-second animations. Playing/lyrics cover fades
remain independent. Thumbnail cropping/downscaling retains small sources without
CPU upscaling.
Set `DEEZNATS_SOFTWARE_RENDERING=1` before launch to compare with software
rendering. Compare identical workloads to measure scrolling performance.
Device failures discard the GPU surface, redraw through the software renderer,
and defer another creation attempt for five seconds.

Run `cargo test -p deeznats-prototype gpu_ -- --ignored --nocapture` for the
hardware pixel and provider-page tests. These require a Windows GPU session;
the normal test suite exercises the software renderer.

GPU rendering still requires CPU layout and command submission. For a debug
executable with symbols and assertions but less CPU overhead, use
`cargo rustc -p deeznats-prototype --bin deeznats-prototype -- -C opt-level=1`.
Instrumented render timings measure CPU submission and presentation calls,
not GPU execution time.

Run `powershell -ExecutionPolicy Bypass -File scripts/Profile-Windows.ps1` to build a release-optimized instrumented executable, start it, and sample its process tree. Use the application as usual and close it to finish the session. Pass `-EnginePath <path>` if the Deezer engine DLL is not in `target/release`, or `-BuildOnly` to build without running. Output goes to a timestamped directory under `%LOCALAPPDATA%\DeezNatsRust\diagnostics`; `-OutputDirectory <path>` selects a different location. Do not run this simultaneously with a normal app instance if the app enforces a single instance.

Use `-TargetDirectory <path>` to build a separate diagnostics executable without replacing another running build. Relative paths are resolved from the workspace root.

After the app closes, the launcher creates `report.md` in the session directory. Run `python scripts/Analyze-Profile.py <session-directory>` to regenerate it, including for older sessions. The report ranks operations, shows p95 and p99 timings, and breaks down process, Rust and cache memory by active view. Process and Rust memory are whole-process values correlated with recent view events, not memory owned by the view; idle periods without recent view events are excluded. Older sessions cannot provide measurements that were not recorded. Each JSONL file contains timestamped raw samples:

- `processes.jsonl`: per-process private bytes, working set, CPU time delta, thread and handle counts, including observed child processes such as WebView2.
- `rust-alloc.jsonl`: live and peak Rust allocator bytes and cumulative allocation/free traffic.
- `render-memory.jsonl`: artwork, backdrop, resized-image and lyric cache estimates, plus serialized page sizes and view information.
- `timings.jsonl`: scroll updates and rebuilds, scene construction, text fitting, native control updates, artwork work, painting, rendering, and individual render stages in microseconds, grouped by view. Paint and render scopes overlap; render stages are consecutive portions of rendering.
- `go-memory.jsonl`: when using the separately built instrumented experiment engine, Go heap/runtime measurements and current/preloaded stream buffer, PCM capacity, and callback/underrun counters. It samples every two seconds; locks held by playback are skipped to avoid waiting on the audio thread.

These measurements do not identify every allocation or operation: Windows/GDI, WebView2, GPU resources, native libraries and audio buffers are not fully attributed. Rust allocation totals exclude allocations made outside the Rust global allocator. Working set and private bytes are different measures and cache figures can overlap them; do not add them together. Process sampling is periodic, so short-lived children and spikes can be missed. Profiling itself adds overhead, and the bounded timing queue may drop records under load. Compare sessions using the same build, machine, window size and workload; use a native allocation/CPU profiler for deeper attribution. The instrumentation is enabled only in the dedicated `diagnostics` build.

The diagnostics allocator tags Rust allocations made during view rebuilds, scrolling and painting, and in catalog, lyrics, history enrichment, and artwork workers for requests carrying a view tag. It tracks their requested bytes until freed, even after leaving the view. The report shows sampled peaks and last-sample totals by view and subsystem, alongside untagged Rust bytes. Shared allocations are attributed to the view whose request created them. Other workers, untagged requests, existing allocations, native or GPU memory, and allocator overhead are excluded; this is partial attribution, not total page memory. Its per-allocation headers and realloc implementation increase profiling overhead and memory usage. Normal builds do not use it.
