use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

/// Fetch independent shelves with bounded concurrency and publish each result as
/// soon as it is available. A superseded page stops starting additional work.
pub(crate) fn load<T: Send>(
    count: usize,
    fetch: impl Fn(usize) -> T + Sync,
    ready: impl FnMut(usize, T) -> Result<(), String>,
) -> Result<(), String> {
    load_with(3, count, fetch, ready)
}

pub(crate) fn load_with<T: Send>(
    workers: usize,
    count: usize,
    fetch: impl Fn(usize) -> T + Sync,
    mut ready: impl FnMut(usize, T) -> Result<(), String>,
) -> Result<(), String> {
    let workers = workers.max(1);
    let next = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let (tx, rx) = mpsc::sync_channel(workers);
    std::thread::scope(|scope| {
        for _ in 0..count.min(workers) {
            let tx = tx.clone();
            let fetch = &fetch;
            let next = &next;
            let cancelled = &cancelled;
            scope.spawn(move || loop {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= count {
                    break;
                }
                if tx.send((index, fetch(index))).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        for (index, value) in rx {
            if let Err(error) = ready(index, value) {
                cancelled.store(true, Ordering::Relaxed);
                return Err(error);
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Mutex, time::Duration};
    #[test]
    fn a_slow_first_shelf_does_not_delay_ready_shelves() {
        let (release, blocked) = mpsc::channel();
        let blocked = Mutex::new(blocked);
        let mut order = Vec::new();
        load(
            6,
            |index| {
                if index == 0 {
                    blocked
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap();
                }
                index
            },
            |index, value| {
                assert_eq!(index, value);
                if order.is_empty() {
                    assert_ne!(index, 0);
                    release.send(()).unwrap();
                }
                order.push(index);
                Ok(())
            },
        )
        .unwrap();
        order.sort();
        assert_eq!(order, [0, 1, 2, 3, 4, 5]);
    }
    #[test]
    fn superseded_loads_stop_scheduling_the_rest() {
        let calls = AtomicUsize::new(0);
        let result = load(
            1000,
            |_| {
                calls.fetch_add(1, Ordering::Relaxed);
            },
            |_, _| Err("superseded".into()),
        );
        assert_eq!(result.unwrap_err(), "superseded");
        assert!(calls.load(Ordering::Relaxed) < 1000);
    }
}
