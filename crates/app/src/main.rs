#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(all(windows, feature = "diagnostics"))]
#[path = "runtime/diagnostics.rs"]
mod diagnostics;
#[cfg(all(windows, feature = "memory-profile"))]
mod memory_profile {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        sync::{
            atomic::{AtomicU64, AtomicUsize, Ordering},
            mpsc, OnceLock,
        },
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    pub struct ProfilingAllocator;
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    static ALLOCATED: AtomicU64 = AtomicU64::new(0);
    static FREED: AtomicU64 = AtomicU64::new(0);
    #[cfg(feature = "diagnostics")]
    mod tagged {
        use std::{
            alloc::{GlobalAlloc, Layout, System},
            cell::Cell,
            sync::atomic::{AtomicUsize, Ordering},
        };
        const SLOTS: usize = 256;
        static LIVE: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
        thread_local! { static TAG: Cell<usize> = const { Cell::new(0) }; }
        pub struct Scope(usize);
        pub fn scope(tag: usize) -> Scope {
            Scope(TAG.with(|t| t.replace(tag.min(SLOTS - 1))))
        }
        impl Drop for Scope {
            fn drop(&mut self) {
                let _ = TAG.try_with(|t| t.set(self.0));
            }
        }
        pub fn live(tag: usize) -> usize {
            LIVE[tag.min(SLOTS - 1)].load(Ordering::Relaxed)
        }
        fn expanded(layout: Layout) -> Option<(Layout, usize)> {
            let header = std::mem::size_of::<usize>();
            let offset = header.checked_add(layout.align() - 1)? & !(layout.align() - 1);
            let size = offset.checked_add(layout.size())?;
            Some((
                Layout::from_size_align(size, layout.align().max(std::mem::align_of::<usize>()))
                    .ok()?,
                offset,
            ))
        }
        pub unsafe fn alloc(layout: Layout, zeroed: bool) -> *mut u8 {
            let Some((full, offset)) = expanded(layout) else {
                return std::ptr::null_mut();
            };
            let raw = if zeroed {
                System.alloc_zeroed(full)
            } else {
                System.alloc(full)
            };
            if raw.is_null() {
                return raw;
            }
            let tag = TAG.try_with(|t| t.get()).unwrap_or(0);
            let ptr = raw.add(offset);
            (ptr.sub(std::mem::size_of::<usize>()) as *mut usize).write(tag);
            if tag != 0 {
                LIVE[tag].fetch_add(layout.size(), Ordering::Relaxed);
            }
            ptr
        }
        pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
            let tag = (ptr.sub(std::mem::size_of::<usize>()) as *const usize).read();
            if tag != 0 {
                LIVE[tag].fetch_sub(layout.size(), Ordering::Relaxed);
            }
            let (full, offset) = expanded(layout).expect("allocation layout was valid");
            System.dealloc(ptr.sub(offset), full);
        }
        pub unsafe fn realloc(ptr: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
            let Ok(new_layout) = Layout::from_size_align(new_size, old.align()) else {
                return std::ptr::null_mut();
            };
            // Preserve the allocation's original tag, including when a different view resizes it.
            let tag = (ptr.sub(std::mem::size_of::<usize>()) as *const usize).read();
            let _scope = scope(tag);
            let next = alloc(new_layout, false);
            if !next.is_null() {
                std::ptr::copy_nonoverlapping(ptr, next, old.size().min(new_size));
                dealloc(ptr, old);
            }
            next
        }
        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn tagged_allocations_preserve_alignment_and_follow_realloc_and_free() {
                unsafe {
                    let baseline = live(63);
                    let layout = Layout::from_size_align(17, 64).unwrap();
                    let ptr = {
                        let _scope = scope(63);
                        alloc(layout, true)
                    };
                    assert!(!ptr.is_null());
                    assert_eq!(ptr as usize % 64, 0);
                    assert_eq!(std::slice::from_raw_parts(ptr, 17), &[0; 17]);
                    assert_eq!(live(63), baseline + 17);
                    let ptr = realloc(ptr, layout, 48);
                    assert!(!ptr.is_null());
                    assert_eq!(ptr as usize % 64, 0);
                    assert_eq!(live(63), baseline + 48);
                    dealloc(ptr, Layout::from_size_align(48, 64).unwrap());
                    assert_eq!(live(63), baseline);
                }
            }
        }
    }
    #[cfg(feature = "diagnostics")]
    pub use tagged::{live as page_live, scope as page_scope};
    #[cfg(feature = "diagnostics")]
    pub fn current_live() -> usize {
        LIVE.load(Ordering::Relaxed)
    }
    fn allocated(bytes: usize) {
        let live = LIVE
            .fetch_add(bytes, Ordering::Relaxed)
            .saturating_add(bytes);
        PEAK.fetch_max(live, Ordering::Relaxed);
        ALLOCATED.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    fn freed(bytes: usize) {
        LIVE.fetch_sub(bytes, Ordering::Relaxed);
        FREED.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    unsafe impl GlobalAlloc for ProfilingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            #[cfg(feature = "diagnostics")]
            let pointer = tagged::alloc(layout, false);
            #[cfg(not(feature = "diagnostics"))]
            let pointer = System.alloc(layout);
            if !pointer.is_null() {
                allocated(layout.size());
            }
            pointer
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            #[cfg(feature = "diagnostics")]
            let pointer = tagged::alloc(layout, true);
            #[cfg(not(feature = "diagnostics"))]
            let pointer = System.alloc_zeroed(layout);
            if !pointer.is_null() {
                allocated(layout.size());
            }
            pointer
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            freed(layout.size());
            #[cfg(feature = "diagnostics")]
            tagged::dealloc(pointer, layout);
            #[cfg(not(feature = "diagnostics"))]
            System.dealloc(pointer, layout);
        }
        unsafe fn realloc(&self, pointer: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
            #[cfg(feature = "diagnostics")]
            let new_pointer = tagged::realloc(pointer, old, new_size);
            #[cfg(not(feature = "diagnostics"))]
            let new_pointer = System.realloc(pointer, old, new_size);
            if !new_pointer.is_null() {
                freed(old.size());
                allocated(new_size);
            }
            new_pointer
        }
    }
    struct Sample {
        time_ms: u128,
        live: usize,
        peak: usize,
        allocated: u64,
        freed: u64,
    }
    fn sender() -> Option<&'static mpsc::SyncSender<Sample>> {
        static SENDER: OnceLock<Option<mpsc::SyncSender<Sample>>> = OnceLock::new();
        SENDER.get_or_init(|| {
            let path = std::env::var_os("DEEZNATS_RUST_ALLOC_LOG")?;
            let (tx, rx) = mpsc::sync_channel::<Sample>(32);
            std::thread::Builder::new().name("memory-profile-writer".into()).spawn(move || {
                use std::io::Write;
                let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
                while let Ok(sample) = rx.recv() {
                    let _ = writeln!(file, "{{\"time_unix_ms\":{},\"rust_live_bytes\":{},\"rust_peak_bytes\":{},\"rust_allocated_bytes\":{},\"rust_freed_bytes\":{}}}", sample.time_ms, sample.live, sample.peak, sample.allocated, sample.freed);
                }
            }).ok()?;
            Some(tx)
        }).as_ref()
    }
    pub fn sample() {
        let Some(sender) = sender() else { return };
        thread_local! { static LAST: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) }; }
        let now = Instant::now();
        if LAST.with(|last| {
            if last
                .get()
                .is_some_and(|previous| now.duration_since(previous) < Duration::from_secs(1))
            {
                true
            } else {
                last.set(Some(now));
                false
            }
        }) {
            return;
        }
        let _ = sender.try_send(Sample {
            time_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            live: LIVE.load(Ordering::Relaxed),
            peak: PEAK.load(Ordering::Relaxed),
            allocated: ALLOCATED.load(Ordering::Relaxed),
            freed: FREED.load(Ordering::Relaxed),
        });
    }
}
#[cfg(all(windows, feature = "memory-profile"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: memory_profile::ProfilingAllocator = memory_profile::ProfilingAllocator;
#[cfg(windows)]
pub(crate) use ui::appearance;
#[cfg(windows)]
#[path = "playback/audio.rs"]
mod audio;
#[cfg(windows)]
#[path = "library/catalog.rs"]
mod catalog;
#[cfg(windows)]
pub(crate) mod connect;
#[cfg(windows)]
#[path = "library/downloads.rs"]
mod downloads;
#[cfg(windows)]
pub(crate) use ui::drawing;
#[cfg(windows)]
#[path = "runtime/engine.rs"]
mod engine;
#[cfg(windows)]
#[path = "runtime/integration.rs"]
mod integration;
#[cfg(windows)]
#[path = "playback/media_controls.rs"]
mod media_controls;
#[cfg(windows)]
#[path = "ui/shortcuts.rs"]
mod shortcuts;
#[cfg(windows)]
#[path = "playback/style_mix.rs"]
mod style_mix;
#[cfg(windows)]
#[path = "runtime/updater.rs"]
mod updater;
#[cfg(windows)]
#[path = "providers/youtube.rs"]
mod youtube;
#[cfg(windows)]
pub(crate) use youtube::youtube_webview;

#[cfg(windows)]
#[path = "providers/deezer.rs"]
mod deezer_webview;
#[cfg(windows)]
#[path = "playback/lyrics.rs"]
mod lyrics;
#[cfg(windows)]
#[path = "storage/session.rs"]
mod session;
#[cfg(windows)]
#[path = "providers/spotify.rs"]
mod spotify;
#[path = "providers/spotify_diagnostics.rs"]
mod spotify_diagnostics;
#[cfg(windows)]
pub(crate) use spotify::spotify_webview;
#[cfg(windows)]
mod storage;

#[cfg(windows)]
#[path = "runtime/instance.rs"]
mod instance;
#[cfg(windows)]
#[path = "library/playlist.rs"]
mod playlist;
#[cfg(windows)]
pub(crate) use ui::theme;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
pub(crate) use ui::visualizer;
#[cfg(windows)]
#[path = "ui/window.rs"]
mod window;

fn main() {
    crate::spotify_diagnostics::record("app.start", "connect_browser_adapter_v1");
    #[cfg(windows)]
    {
        let args: Vec<_> = std::env::args_os().collect();
        let result = if let Some(index) = args.iter().position(|arg| arg == "--apply-update") {
            args.get(index + 1)
                .ok_or_else(|| "Missing update staging directory.".to_owned())
                .and_then(|path| updater::apply_update(std::path::Path::new(path)))
        } else if std::env::args().any(|a| a == "--inspect-soloist-dashboard") {
            spotify::wizard::run(
                &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                std::sync::Arc::new(std::collections::HashMap::new()),
                [
                    0x202020, 0xffffff, 0x303030, 0xffffff, 0x404040, 0xffffff, 0x505050, 0xffffff,
                    0x606060,
                ],
            )
        } else if std::env::args().any(|a| a == "--spotify") {
            unsafe {
                use windows_sys::Win32::System::Console::*;
                if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                    AllocConsole();
                }
            }
            spotify::run_cli()
        } else if std::env::args().any(|a| a == "--integration-test") {
            integration::run()
        } else if std::env::args().any(|a| a == "--deezer-playback-test") {
            audio::deezer_playback_test()
        } else if std::env::args().any(|a| a == "--engine-smoke-test") {
            audio::engine_smoke_test()
        } else {
            window::run()
        };
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    println!("DeezNats requires Windows.");
}
