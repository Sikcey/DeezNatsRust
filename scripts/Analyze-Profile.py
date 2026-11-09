"""Produce a repeatable, evidence-based report for a diagnostics session."""
import json
import math
import statistics
import sys
from bisect import bisect_right
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

MIB = 1048576


def records(path, problems):
    if not path.exists():
        problems.append(f"Missing {path.name}")
        return []
    output = []
    with path.open(encoding="utf-8-sig") as source:
        for number, line in enumerate(source, 1):
            try:
                row = json.loads(line)
                if not isinstance(row, dict):
                    raise ValueError("record is not an object")
                output.append(row)
            except (ValueError, json.JSONDecodeError):
                problems.append(f"Invalid {path.name} line {number}")
    return output


def percentile(values, fraction):
    values = sorted(values)
    return values[max(0, math.ceil(len(values) * fraction) - 1)]


def timestamp(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc).isoformat(timespec="seconds")


def report(directory):
    problems = []
    timing = records(directory / "timings.jsonl", problems)
    process = records(directory / "processes.jsonl", problems)
    alloc = records(directory / "rust-alloc.jsonl", problems)
    memory = records(directory / "render-memory.jsonl", problems)
    go_path = directory / "go-memory.jsonl"
    go = records(go_path, problems) if go_path.exists() else []
    lines = ["# Diagnostics report", "", f"Session: `{directory}`", ""]
    groups = defaultdict(list)
    views = defaultdict(list)
    for row in timing:
        if isinstance(row.get("duration_us"), (int, float)) and isinstance(row.get("name"), str):
            groups[row["name"]].append(row["duration_us"] / 1000)
            views[(row["name"], row.get("view", "unattributed"))].append(row["duration_us"] / 1000)
        else:
            problems.append("Timing record missing name or duration")
    samples = [row for row in process if isinstance(row.get("time_unix_ms"), (int, float))]
    if samples:
        samples.sort(key=lambda row: row["time_unix_ms"])
        lines += [f"Recorded: {timestamp(samples[0]['time_unix_ms'])} to {timestamp(samples[-1]['time_unix_ms'])} ({(samples[-1]['time_unix_ms']-samples[0]['time_unix_ms'])/1000:.1f} seconds).", ""]
    dropped = max((r.get("dropped_total", 0) for r in timing), default=0)
    has_views = any("view" in row for row in timing)
    has_composite = "render.composite" in groups
    lines += ["## What to investigate", ""]
    paint = groups.get("ui.paint", [])
    if paint:
        for budget in (16.67, 33.33):
            count = sum(value > budget for value in paint)
            lines.append(f"- Paints over {budget:.2f} ms: {count}/{len(paint)} ({100*count/len(paint):.1f}%). Paint duration is not a measurement of end-to-end frame latency or dropped frames.")
    scroll = groups.get("scroll.update", [])
    if scroll:
        lines.append(f"- Scroll updates over 16.67 ms: {sum(x > 16.67 for x in scroll)}/{len(scroll)}; inspect `scroll.rebuild` and `rebuild.*` in the same view to locate the work.")
    stage_names = [name for name in groups if name.startswith(("render.", "rebuild.")) and name != "render.present"]
    if stage_names:
        worst = max(stage_names, key=lambda name: sum(groups[name]))
        lines.append(f"- Largest measured stage by accumulated time: `{worst}` ({sum(groups[worst])/1000:.2f} s across {len(groups[worst])} samples). Compare p95 and per-view rows below before deciding what to change.")
        targets = {
            "rebuild.scene": "Inspect `ui::build` and visible-row construction in `crates/app/src/ui/mod.rs`.",
            "rebuild.text": "Inspect `expand_media_text` and `fit_text_links` in `crates/app/src/ui/window.rs`.",
            "rebuild.controls": "Inspect native button and edit-control synchronization in `rebuild` in `crates/app/src/ui/window.rs`.",
            "rebuild.artwork": "Inspect `request_art` and `trim_artwork` in `crates/app/src/ui/window.rs`.",
            "render.labels": "Inspect the label drawing section of `render` in `crates/app/src/ui/window.rs`.",
            "render.composite": "Inspect the pixel compositing loop in `render` in `crates/app/src/ui/window.rs`.",
            "render.pictures": "Inspect image and artwork rendering in `render` in `crates/app/src/ui/window.rs`.",
            "render.panels": "Inspect panel drawing in `render` in `crates/app/src/ui/window.rs`.",
        }
        if worst in targets:
            if worst == "render.labels" and not has_composite:
                lines.append("- Inspect label drawing and the pixel compositing loop in `render` in `crates/app/src/ui/window.rs`; this older timing combines both.")
            else:
                lines.append("- Suggested starting point: " + targets[worst])
    if not has_composite and groups.get("render.labels"):
        lines.append("- This older recording combines text and pixel compositing in `render.labels`; collect a new session to separate them.")
    if not has_views:
        lines.append("- This older recording has no view labels; collect a new session to identify the affected screen.")
    if not groups:
        lines.append("- No timing samples; no performance conclusion is available.")
    paint_views = [(view, values) for (name, view), values in views.items() if name == "ui.paint"]
    if paint_views:
        view, values = max(paint_views, key=lambda pair: sum(pair[1]))
        lines.append(f"- Most accumulated paint time: `{view}` ({sum(values)/1000:.2f} s across {len(values)} paints, p95 {percentile(values,.95):.2f} ms). This also depends on how long that view was active; compare p95 for per-paint cost.")
    lines += ["", "## Timing by operation", "", "| Operation | Samples | Median ms | p95 ms | p99 ms | Max ms | Total s |", "|---|---:|---:|---:|---:|---:|---:|"]
    for name, values in sorted(groups.items()):
        lines.append(f"| {name} | {len(values)} | {statistics.median(values):.3f} | {percentile(values,.95):.3f} | {percentile(values,.99):.3f} | {max(values):.3f} | {sum(values)/1000:.2f} |")
    lines += ["", "## Timing by view", "", "| Operation | View | Samples | Median ms | p95 ms | Max ms | Total s |", "|---|---|---:|---:|---:|---:|---:|"]
    for (name, view), values in sorted(views.items(), key=lambda item: sum(item[1]), reverse=True):
        lines.append(f"| {name} | {view} | {len(values)} | {statistics.median(values):.3f} | {percentile(values,.95):.3f} | {max(values):.3f} | {sum(values)/1000:.2f} |")
    lines += ["", "Timing scopes overlap: `ui.paint` contains `ui.render`, and `scroll.rebuild` contains `ui.rebuild`. Render stages are consecutive portions; rebuild stages cover selected portions only. Totals must not be added across scopes.", "", "## Processes", "", "| PID | Name | Samples | Peak private MiB | Peak working set MiB | CPU seconds observed | Peak threads | Peak handles |", "|---:|---|---:|---:|---:|---:|---:|---:|"]
    by_process = defaultdict(list)
    for sample in samples:
        for row in sample.get("processes", []):
            if all(key in row for key in ("pid", "name", "private_bytes", "working_set_bytes")):
                by_process[(row["pid"], row["name"])].append(row)
    for (pid, name), rows in sorted(by_process.items()):
        cpu = sum(row.get("cpu_seconds_delta") or 0 for row in rows)
        lines.append(f"| {pid} | {name} | {len(rows)} | {max(r['private_bytes'] for r in rows)/MIB:.1f} | {max(r['working_set_bytes'] for r in rows)/MIB:.1f} | {cpu:.1f} | {max(r.get('threads', 0) for r in rows)} | {max(r.get('handles', 0) for r in rows)} |")
    if samples and by_process:
        totals = [(sum(p.get("private_bytes", 0) for p in row.get("processes", [])), row["time_unix_ms"]) for row in samples]
        peak, when = max(totals)
        lines += ["", f"Peak simultaneous observed process-tree private memory: {peak/MIB:.1f} MiB at {timestamp(when)}. Child processes that were missed or not descendants are excluded."]
    # Timing records carry the active view. Restrict correlation to recent UI
    # activity; a stale view must not silently label later process samples.
    timeline = sorted((r['time_unix_ms'], r['view']) for r in timing
                      if isinstance(r.get('time_unix_ms'), (int, float)) and isinstance(r.get('view'), str))
    times = [item[0] for item in timeline]
    def active_view(when):
        index = bisect_right(times, when) - 1
        if index < 0 or when - times[index] > 2000:
            return None
        return timeline[index][1]
    memory_by_view = defaultdict(lambda: defaultdict(list))
    unassigned = 0
    for sample in samples:
        view = active_view(sample['time_unix_ms'])
        if view is None:
            unassigned += 1
            continue
        rows = sample.get('processes', [])
        memory_by_view[view]['process_private'].append(sum(r.get('private_bytes', 0) for r in rows))
        memory_by_view[view]['process_working'].append(sum(r.get('working_set_bytes', 0) for r in rows))
    for row in alloc:
        if isinstance(row.get('time_unix_ms'), (int, float)) and isinstance(row.get('rust_live_bytes'), (int, float)):
            view = active_view(row['time_unix_ms'])
            if view:
                memory_by_view[view]['rust_live'].append(row['rust_live_bytes'])
    def memory_cell(values):
        return f"{statistics.median(values)/MIB:.1f} / {max(values)/MIB:.1f} ({len(values)})" if values else "—"
    lines += ["", "## Memory while each view was active", "", "| View | Process tree private MiB median / peak (samples) | Process tree working set MiB median / peak (samples) | Rust live MiB median / peak (samples) |", "|---|---:|---:|---:|"]
    for view, metrics in sorted(memory_by_view.items(), key=lambda item: max(item[1]['process_private'], default=0), reverse=True):
        lines.append(f"| {view} | {memory_cell(metrics['process_private'])} | {memory_cell(metrics['process_working'])} | {memory_cell(metrics['rust_live'])} |")
    lines += ["", f"{unassigned} process samples had no view event in the preceding 2 seconds and are excluded. A view label is inferred from the most recent timed UI event, so changes between samples can be misclassified. Figures are whole-process memory while the view was active, **not memory owned by that view**: playback, previously visited pages, caches, native libraries and other background work remain included. Peaks on different views are not additive. Idle views may be underrepresented."]
    valid_alloc = sorted((r for r in alloc if all(k in r for k in ('time_unix_ms', 'rust_live_bytes', 'rust_peak_bytes', 'rust_allocated_bytes', 'rust_freed_bytes'))), key=lambda r:r['time_unix_ms'])
    lines += ["", "## Rust allocator", ""]
    if valid_alloc:
        first, last = valid_alloc[0], valid_alloc[-1]
        peak = max(valid_alloc, key=lambda r:r['rust_live_bytes'])
        lines += [f"- Live: {first['rust_live_bytes']/MIB:.1f} MiB first sample, {last['rust_live_bytes']/MIB:.1f} MiB last; maximum sampled live {peak['rust_live_bytes']/MIB:.1f} MiB at {timestamp(peak['time_unix_ms'])}.", f"- Allocator-recorded peak: {max(r['rust_peak_bytes'] for r in valid_alloc)/MIB:.1f} MiB. Allocated traffic between samples: {(last['rust_allocated_bytes']-first['rust_allocated_bytes'])/MIB:.1f} MiB; freed traffic: {(last['rust_freed_bytes']-first['rust_freed_bytes'])/MIB:.1f} MiB. Traffic is not retained memory."]
        rates = [( (b['rust_allocated_bytes']-a['rust_allocated_bytes'])/MIB / ((b['time_unix_ms']-a['time_unix_ms'])/1000), b['time_unix_ms']) for a,b in zip(valid_alloc, valid_alloc[1:]) if b['time_unix_ms'] > a['time_unix_ms'] and b['rust_allocated_bytes'] >= a['rust_allocated_bytes']]
        if rates:
            rate, when = max(rates)
            lines.append(f"- Highest observed allocation rate: {rate:.1f} MiB/s at {timestamp(when)}; median interval rate {statistics.median(r for r,_ in rates):.1f} MiB/s. These are process-wide, not allocation call sites.")
    else:
        lines.append("No allocator samples.")
    lines += ["", "## Go playback engine", ""]
    if go:
        for key, label in (("heap_live_bytes", "Go live heap"), ("heap_inuse_bytes", "Go heap in-use pages"), ("heap_reserved_bytes", "Go heap reserved address space"), ("heap_released_bytes", "Go heap released"), ("go_sys_bytes", "Go runtime system bytes")):
            values = [r[key] for r in go if isinstance(r.get(key), (int, float))]
            if values:
                lines.append(f"- {label}: median {statistics.median(values)/MIB:.1f} MiB; peak {max(values)/MIB:.1f} MiB; last {values[-1]/MIB:.1f} MiB.")
        players = [r['player'] for r in go if isinstance(r.get('player'), dict)]
        for name in ('current', 'next'):
            sources = [p[name] for p in players if isinstance(p.get(name), dict) and p[name].get('present') and not p[name].get('busy')]
            if sources:
                lines.append(f"- {name.title()} source: peak encoded buffer capacity {max(s['encoded_capacity'] for s in sources)/MIB:.2f} MiB, read-cache capacity {max(s['read_cache_capacity'] for s in sources)/MIB:.2f} MiB, PCM capacity {max(s['pcm_capacity'] for s in sources)/MIB:.2f} MiB; {sum(bool(s.get('disk_spill')) for s in sources)}/{len(sources)} samples were disk-backed.")
        if players:
            lines.append(f"- Audio callbacks: first {players[0].get('callbacks',0)}, last {players[-1].get('callbacks',0)}; cumulative underruns: first {players[0].get('underruns',0)}, last {players[-1].get('underruns',0)}. Counters may reset if the player is recreated; underruns include all observed playback, not only this experiment's changes.")
        lines.append("- Go live heap includes stream and PCM buffers; do not add those figures together. Heap reservation is not equivalent to process private bytes. Busy source snapshots are excluded; reporting runs every two seconds and can miss short spikes.")
    else:
        lines.append("No Go samples: this requires the experimental instrumented engine DLL and a new session.")
    cache_rows = [r for r in memory if r.get('kind') == 'render_memory']
    tracked = [r for r in cache_rows if isinstance(r.get('page_scoped_all_live_bytes'), list)]
    lines += ["", "## Retained Rust allocations by page and subsystem", ""]
    if tracked:
        tracked.sort(key=lambda r: r.get('time_unix_ms', 0))
        tag_names = {r['page_tag']: r['view'] for r in tracked if isinstance(r.get('page_tag'), int) and isinstance(r.get('view'), str)}
        lines += ["| View that created allocations | UI layout / scroll peak / last MiB | Catalog / lyrics workers peak / last MiB | Artwork workers peak / last MiB | Painting peak / last MiB |", "|---|---:|---:|---:|---:|"]
        for tag, name in sorted(tag_names.items()):
            columns = []
            for offset in (0, 64, 128, 192):
                values = [r['page_scoped_all_live_bytes'][tag + offset] for r in tracked if tag + offset < len(r['page_scoped_all_live_bytes'])]
                columns.append(f"{max(values)/MIB:.2f} / {values[-1]/MIB:.2f}" if values else "not recorded")
            lines.append(f"| {name} | {' | '.join(columns)} |")
        last = tracked[-1]
        counters = last['page_scoped_all_live_bytes']
        rust = last.get('measured_rust_live_bytes')
        if isinstance(rust, (int, float)):
            tagged = sum(counters)
            lines += ["", f"Last sample: tagged Rust allocations {tagged/MIB:.1f} MiB; total Rust live allocations {rust/MIB:.1f} MiB; approximately {max(0, rust-tagged)/MIB:.1f} MiB remains untagged. Counters are read sequentially while other threads run, so the difference is approximate."]
        lines += ["", "Live requested Rust bytes are tagged while the UI rebuild, scrolling, painting or associated catalog, lyrics, history or artwork worker executes. Tags follow allocations until freed, even after navigation; reallocations keep their original tag. Shared caches and responses remain attributed to the view that triggered the work. Existing allocations, other workers, untagged requests, native/GDI/GPU memory and allocator overhead are excluded. Some Spotify requests do not carry a page tag. Peaks are sampled and may miss short spikes. Profiling adds overhead."]
    else:
        lines.append("Not recorded in this session; run a new diagnostics build to measure rebuild-scoped Rust allocations.")
    cache_names = ("artwork_bytes", "backdrop_bytes", "resized_bytes", "lyric_shadow_bytes", "lyric_raster_bytes", "lyric_blurred_bytes", "page_json_bytes")
    cache_by_view = defaultdict(list)
    for row in cache_rows:
        if row.get('view'):
            cache_by_view[row['view']].append(row)
    lines += ["", "## Cache estimates by view", "", "| View | Samples | Artwork peak MiB | Resized peak MiB | Backdrop peak MiB | Lyric rasters peak MiB | Serialized pages peak MiB |", "|---|---:|---:|---:|---:|---:|---:|"]
    for view, rows in sorted(cache_by_view.items()):
        peak = lambda key: max(r.get(key, 0) for r in rows)/MIB
        lyric_peak = max(sum(r.get(k, 0) for k in ('lyric_shadow_bytes','lyric_raster_bytes','lyric_blurred_bytes')) for r in rows)/MIB
        lines.append(f"| {view} | {len(rows)} | {peak('artwork_bytes'):.1f} | {peak('resized_bytes'):.1f} | {peak('backdrop_bytes'):.1f} | {lyric_peak:.1f} | {peak('page_json_bytes'):.1f} |")
    lines += ["", "## Tracked render caches", "", "| Estimate | Peak MiB | View at peak | Time UTC |", "|---|---:|---|---|"]
    for key in cache_names:
        if cache_rows:
            row = max(cache_rows, key=lambda r:r.get(key, 0))
            lines.append(f"| {key} | {row.get(key,0)/MIB:.1f} | {row.get('view','unknown')} | {timestamp(row.get('time_unix_ms',0))} |")
    lines += ["", "Cache estimates overlap allocator/process memory; page JSON size is a serialized-size estimate, not heap usage. Separate peak values may occur at different times.", "", "## Data quality and limits", "", f"- Timing records: {len(timing)}; last observed cumulative queue drops: {dropped if any('dropped_total' in r for r in timing) else 'not recorded'}. A final drop count may be missing if no later event was written.", "- Samples do not cover every operation, call site, native/GDI allocation, GPU allocation, audio buffer, or WebView2 subprocess. Periodic process samples miss short spikes and processes.", "- Instrumentation, cache accounting, and logging add overhead. Compare identical workloads and builds. Timing stage totals overlap parent scopes."]
    if problems:
        lines += ["- Input problems: " + "; ".join(problems[:20]) + (f"; and {len(problems)-20} more" if len(problems) > 20 else "")]
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python scripts/Analyze-Profile.py SESSION_DIRECTORY")
    directory = Path(sys.argv[1])
    if not directory.is_dir():
        raise SystemExit(f"Session directory not found: {directory}")
    result = report(directory)
    (directory / "report.md").write_text(result, encoding="utf-8")
    print(result)
