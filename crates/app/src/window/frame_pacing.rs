//! Active animation deadlines integrated with the UI queue. Native window
//! timers remain installed for Windows-owned menu and move/resize loops.
use std::{
    collections::{HashMap, VecDeque},
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::Threading::*, UI::WindowsAndMessaging::*,
};

struct Tick {
    hwnd: HWND,
    id: usize,
    period: Duration,
    deadline: Instant,
}
#[derive(Default)]
struct Pacer {
    timer: HANDLE,
    ticks: HashMap<(usize, usize), Tick>,
    messages_since_tick: usize,
    clock: Option<super::compositor_clock::CompositorClock>,
    serial: u64,
    pending: VecDeque<MSG>,
}
impl Drop for Pacer {
    fn drop(&mut self) {
        self.disable();
    }
}
impl Pacer {
    fn disable(&mut self) {
        self.clock = None;
        self.pending.clear();
        self.serial = 0;
        if !self.timer.is_null() {
            unsafe {
                CancelWaitableTimer(self.timer);
                CloseHandle(self.timer);
            }
            self.timer = null_mut();
        }
        self.ticks.clear();
    }
    fn take_due(&mut self, now: Instant) -> Option<MSG> {
        if let Some(message) = self.pending.pop_front() {
            return Some(message);
        }
        if let Some(clock) = &self.clock {
            let serial = clock.serial();
            if serial == 0 || serial == self.serial {
                return None;
            }
            self.serial = serial;
            for tick in self.ticks.values_mut() {
                let every_frame = tick.id != 4
                    && super::BACKGROUND_FRAME_INTERVAL_NS
                        .load(std::sync::atomic::Ordering::Relaxed)
                        == 0;
                if every_frame || tick.deadline <= now {
                    tick.deadline = following_deadline(tick.deadline, now, tick.period);
                    self.pending.push_back(MSG {
                        hwnd: tick.hwnd,
                        message: WM_TIMER,
                        wParam: tick.id,
                        ..unsafe { zeroed() }
                    });
                }
            }
            self.messages_since_tick = 0;
            return self.pending.pop_front();
        }
        let tick = self.ticks.values_mut().min_by_key(|tick| tick.deadline)?;
        if tick.deadline > now {
            return None;
        }
        tick.deadline = following_deadline(tick.deadline, now, tick.period);
        self.messages_since_tick = 0;
        Some(MSG {
            hwnd: tick.hwnd,
            message: WM_TIMER,
            wParam: tick.id,
            ..unsafe { zeroed() }
        })
    }
}

// Preserve cadence, but skip expired frames rather than issuing catch-up work.
fn following_deadline(previous: Instant, now: Instant, period: Duration) -> Instant {
    let elapsed = now.saturating_duration_since(previous).as_nanos();
    let remaining = period.as_nanos() - elapsed % period.as_nanos();
    now + Duration::from_nanos(remaining as u64)
}

thread_local! { static PACER: std::cell::RefCell<Pacer> = std::cell::RefCell::new(Pacer::default()); }

unsafe fn period(hwnd: HWND, id: usize) -> Duration {
    let mut monitor: MONITORINFOEXW = zeroed();
    monitor.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let mut mode: DEVMODEW = zeroed();
    mode.dmSize = size_of::<DEVMODEW>() as u16;
    let hz = if GetMonitorInfoW(
        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
        (&mut monitor as *mut MONITORINFOEXW).cast(),
    ) != 0
        && EnumDisplaySettingsW(monitor.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) != 0
        && (24..=1000).contains(&mode.dmDisplayFrequency)
    {
        mode.dmDisplayFrequency as u64
    } else {
        60
    };
    let background = super::BACKGROUND_FRAME_INTERVAL_NS.load(std::sync::atomic::Ordering::Relaxed);
    Duration::from_nanos((1_000_000_000 / hz).max(background).max(if id == 4 {
        33_333_333
    } else {
        1
    }))
}

pub(super) unsafe fn start(hwnd: HWND, id: usize) {
    PACER.with(|state| {
        let mut state = state.borrow_mut();
        if state.timer.is_null() {
            state.timer = CreateWaitableTimerExW(
                null(),
                null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS,
            );
        }
        if !state.timer.is_null() {
            if state.clock.is_none() {
                state.clock = super::compositor_clock::CompositorClock::start(period(hwnd, 2));
                state.serial = 0;
            }
            let period = period(hwnd, id);
            state.ticks.insert(
                (hwnd as usize, id),
                Tick {
                    hwnd,
                    id,
                    period,
                    deadline: Instant::now() + period,
                },
            );
        }
    });
}

pub(super) fn stop(hwnd: HWND, id: usize) {
    PACER.with(|state| {
        let mut state = state.borrow_mut();
        state.ticks.remove(&(hwnd as usize, id));
        state.pending.retain(|m| m.hwnd != hwnd || m.wParam != id);
        if state.ticks.is_empty() {
            state.disable();
        }
    });
}

pub(super) unsafe fn refresh(hwnd: HWND) {
    PACER.with(|state| {
        let now = Instant::now();
        if let Some(clock) = &state.borrow().clock {
            clock.refresh(period(hwnd, 2));
        }
        for tick in state
            .borrow_mut()
            .ticks
            .values_mut()
            .filter(|tick| tick.hwnd == hwnd)
        {
            tick.period = period(hwnd, tick.id);
            tick.deadline = now + tick.period;
        }
    });
}

pub(super) unsafe fn next_message(message: &mut MSG) -> i32 {
    loop {
        let active = PACER.with(|state| !state.borrow().timer.is_null());
        if !active {
            return GetMessageW(message, null_mut(), 0, 0);
        }
        if let Some(pending) = PACER.with(|s| s.borrow_mut().pending.pop_front()) {
            *message = pending;
            return 1;
        }
        // Drain real input and pending paints first. Do not post synthetic frame
        // messages: Windows' nested modal loops must remain able to yield.
        if PeekMessageW(message, null_mut(), 0, 0, PM_NOREMOVE) != 0 {
            // Bound input batches so a stream of posted worker/input messages
            // cannot indefinitely starve an expired animation deadline. Always
            // complete pending paints before advancing another animation frame.
            if !matches!(message.message, WM_PAINT | WM_NCPAINT | WM_QUIT) {
                let due = PACER.with(|state| {
                    let mut state = state.borrow_mut();
                    (state.messages_since_tick >= 8)
                        .then(|| state.take_due(Instant::now()))
                        .flatten()
                });
                if let Some(due) = due {
                    *message = due;
                    return 1;
                }
            }
            if PeekMessageW(message, null_mut(), 0, 0, PM_REMOVE) == 0 {
                continue;
            }
            if message.message == WM_QUIT {
                return 0;
            }
            if message.message == WM_TIMER
                && message.lParam == 0
                && PACER.with(|state| {
                    state
                        .borrow()
                        .ticks
                        .contains_key(&(message.hwnd as usize, message.wParam))
                })
            {
                continue; // Main loop uses the deadline; modal loops use this fallback.
            }
            PACER.with(|state| {
                let mut state = state.borrow_mut();
                state.messages_since_tick = state.messages_since_tick.saturating_add(1);
            });
            return 1;
        }
        let due = PACER.with(|state| state.borrow_mut().take_due(Instant::now()));
        if let Some(due) = due {
            *message = due;
            return 1;
        }
        let wait = PACER.with(|state| {
            let state = state.borrow();
            state
                .ticks
                .values()
                .map(|tick| tick.deadline)
                .min()
                .map(|deadline| {
                    (
                        state
                            .clock
                            .as_ref()
                            .map_or(state.timer, |clock| clock.handle()),
                        deadline,
                        state.clock.is_none(),
                    )
                })
        });
        let Some((timer, deadline, use_deadline)) = wait else {
            return GetMessageW(message, null_mut(), 0, 0);
        };
        let due_100ns = -((deadline
            .saturating_duration_since(Instant::now())
            .as_nanos()
            / 100)
            .max(1) as i64);
        if use_deadline && SetWaitableTimerEx(timer, &due_100ns, 0, None, null(), null(), 0) == 0 {
            PACER.with(|state| state.borrow_mut().disable());
            continue;
        }
        if MsgWaitForMultipleObjectsEx(1, &timer, INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
            == WAIT_FAILED
        {
            PACER.with(|state| state.borrow_mut().disable());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn posted_message_backlog_cannot_starve_animation() {
        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            );
            assert!(!hwnd.is_null());
            super::super::start_animation_timer(hwnd, 2);
            let available = PACER.with(|state| {
                let mut state = state.borrow_mut();
                state.clock = None; // Exercise the deadline fallback deterministically.
                if let Some(tick) = state.ticks.get_mut(&(hwnd as usize, 2)) {
                    tick.deadline = Instant::now() - Duration::from_millis(1);
                    state.messages_since_tick = 0;
                    true
                } else {
                    false
                }
            });
            if available {
                for _ in 0..16 {
                    assert_ne!(PostMessageW(hwnd, WM_APP, 0, 0), 0);
                }
                let mut message: MSG = zeroed();
                for _ in 0..8 {
                    assert_eq!(next_message(&mut message), 1);
                    assert_eq!(message.message, WM_APP);
                }
                assert_eq!(next_message(&mut message), 1);
                assert_eq!(message.message, WM_TIMER);
                assert_ne!(
                    PeekMessageW(&mut message, hwnd, WM_APP, WM_APP, PM_NOREMOVE),
                    0
                );
                while PeekMessageW(&mut message, hwnd, WM_APP, WM_APP, PM_REMOVE) != 0 {}
            }
            super::super::stop_animation_timer(hwnd, 2);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn message_loop_delivers_input_then_animation_and_releases_idle_timer() {
        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            );
            assert!(!hwnd.is_null());
            super::super::start_animation_timer(hwnd, 2);
            assert_ne!(PostMessageW(hwnd, WM_KEYDOWN, 40, 0), 0);
            let mut message: MSG = zeroed();
            assert_eq!(next_message(&mut message), 1);
            assert_eq!(message.message, WM_KEYDOWN);
            assert_eq!(next_message(&mut message), 1);
            assert_eq!(message.message, WM_TIMER);
            assert_eq!(message.wParam, 2);
            super::super::stop_animation_timer(hwnd, 2);
            PACER.with(|state| {
                let state = state.borrow();
                assert!(state.ticks.is_empty());
                assert!(state.timer.is_null());
            });
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn missed_frames_skip_forward_without_catch_up_or_phase_drift() {
        let origin = Instant::now();
        let period = Duration::from_millis(10);
        assert_eq!(following_deadline(origin, origin, period), origin + period);
        assert_eq!(
            following_deadline(origin, origin + Duration::from_millis(37), period),
            origin + Duration::from_millis(40)
        );
        assert_eq!(
            following_deadline(origin, origin + Duration::from_millis(40), period),
            origin + Duration::from_millis(50)
        );
    }
    #[test]
    fn overdue_tick_produces_one_frame_and_waits_for_next_deadline() {
        let now = Instant::now();
        let mut state = Pacer::default();
        state.ticks.insert(
            (0, 2),
            Tick {
                hwnd: null_mut(),
                id: 2,
                period: Duration::from_millis(10),
                deadline: now - Duration::from_millis(37),
            },
        );
        assert_eq!(state.take_due(now).unwrap().wParam, 2);
        assert!(state.take_due(now).is_none());
        assert!(state.take_due(now + Duration::from_millis(2)).is_none());
        assert!(state.take_due(now + Duration::from_millis(3)).is_some());
    }
}
