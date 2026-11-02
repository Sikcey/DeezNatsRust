//! A compositor signal source. Never wait for DWM on the UI thread.
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Graphics::Dwm::DwmFlush,
    System::Threading::{CreateEventW, SetEvent},
};
struct Event(HANDLE);
// Kernel events support concurrent SetEvent/wait. Arc owns the final close.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub(super) struct CompositorClock {
    event: Arc<Event>,
    stop: Arc<AtomicBool>,
    serial: Arc<AtomicU64>,
    fallback_ns: Arc<AtomicU64>,
}
impl CompositorClock {
    pub(super) fn start(interval: Duration) -> Option<Self> {
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let event = Arc::new(Event(handle));
        let stop = Arc::new(AtomicBool::new(false));
        let serial = Arc::new(AtomicU64::new(0));
        let fallback_ns = Arc::new(AtomicU64::new(interval.as_nanos() as u64));
        let (signal, stopped, generation, period) = (
            event.clone(),
            stop.clone(),
            serial.clone(),
            fallback_ns.clone(),
        );
        std::thread::Builder::new()
            .name("compositor-clock".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    let began = Instant::now();
                    let status = unsafe { DwmFlush() };
                    // Disabled composition and headless/remote sessions can return
                    // immediately. Keep a bounded cadence instead of busy-spinning.
                    if status < 0 || began.elapsed() < Duration::from_millis(1) {
                        let interval =
                            Duration::from_nanos(period.load(Ordering::Relaxed).max(1_000_000));
                        if let Some(remaining) = interval.checked_sub(began.elapsed()) {
                            std::thread::sleep(remaining);
                        }
                    }
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    generation.fetch_add(1, Ordering::Release);
                    unsafe {
                        SetEvent(signal.0);
                    }
                }
            })
            .ok()?;
        Some(Self {
            event,
            stop,
            serial,
            fallback_ns,
        })
    }
    pub(super) fn handle(&self) -> HANDLE {
        self.event.0
    }
    pub(super) fn serial(&self) -> u64 {
        self.serial.load(Ordering::Acquire)
    }
    pub(super) fn refresh(&self, period: Duration) {
        self.fallback_ns
            .store(period.as_nanos() as u64, Ordering::Relaxed);
    }
}
impl Drop for CompositorClock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    #[test]
    fn compositor_signal_advances_and_drop_releases_owner() {
        let clock = CompositorClock::start(Duration::from_millis(10)).unwrap();
        assert_eq!(unsafe { WaitForSingleObject(clock.handle(), 2000) }, 0);
        let first = clock.serial();
        assert!(first > 0);
        clock.refresh(Duration::from_millis(12));
        assert_eq!(unsafe { WaitForSingleObject(clock.handle(), 2000) }, 0);
        assert!(clock.serial() > first);
        let stopped = clock.stop.clone();
        drop(clock);
        assert!(stopped.load(Ordering::Acquire));
    }
}
