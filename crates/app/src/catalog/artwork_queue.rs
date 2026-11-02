//! Bounded visible/prefetch lanes and cancellation before expensive decoding.
use super::ArtRequest;
use std::{
    collections::{HashSet, VecDeque},
    sync::{Condvar, Mutex},
};
#[derive(Default)]
struct State {
    visible: VecDeque<ArtRequest>,
    prefetch: VecDeque<ArtRequest>,
    wanted: Option<HashSet<String>>,
    closed: bool,
}
#[derive(Default)]
pub(super) struct ArtworkQueue {
    state: Mutex<State>,
    ready: Condvar,
}
fn wanted(state: &State, request: &ArtRequest) -> bool {
    request.background
        || request.max_edge > 256
        || state
            .wanted
            .as_ref()
            .is_none_or(|keys| keys.contains(&request.key))
}
impl ArtworkQueue {
    pub(super) fn push(&self, request: ArtRequest, visible: bool) -> Result<(), ArtRequest> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err(request);
        }
        let lane = if visible {
            &mut state.visible
        } else {
            &mut state.prefetch
        };
        if lane.len() >= if visible { 16 } else { 8 } {
            return Err(request);
        }
        lane.push_back(request);
        self.ready.notify_one();
        Ok(())
    }
    pub(super) fn pop(&self) -> Option<ArtRequest> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.closed {
                return None;
            }
            if let Some(request) = state
                .visible
                .pop_front()
                .or_else(|| state.prefetch.pop_front())
            {
                return Some(request);
            }
            state = self.ready.wait(state).unwrap();
        }
    }
    pub(super) fn wanted(&self, request: &ArtRequest) -> bool {
        wanted(&self.state.lock().unwrap(), request)
    }
    pub(super) fn update(&self, keys: HashSet<String>, visible: HashSet<String>) -> Vec<String> {
        let mut state = self.state.lock().unwrap();
        state.wanted = Some(keys);
        let mut cancelled = Vec::new();
        let old_visible = std::mem::take(&mut state.visible);
        let old_prefetch = std::mem::take(&mut state.prefetch);
        for request in old_visible.into_iter().chain(old_prefetch) {
            if !wanted(&state, &request) {
                cancelled.push(request.key);
                continue;
            }
            let foreground =
                visible.contains(&request.key) || request.background || request.max_edge > 256;
            if foreground && state.visible.len() < 16 {
                state.visible.push_back(request);
            } else if state.prefetch.len() < 8 {
                state.prefetch.push_back(request);
            } else {
                cancelled.push(request.key);
            }
        }
        cancelled
    }
    pub(super) fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn job(key: &str) -> ArtRequest {
        ArtRequest {
            #[cfg(feature = "diagnostics")]
            view_tag: 0,
            engine: None,
            url: key.into(),
            key: key.into(),
            background: false,
            max_edge: 192,
        }
    }
    #[test]
    fn visible_work_bypasses_prefetch_and_stale_jobs_are_cancelled() {
        let queue = ArtworkQueue::default();
        for i in 0..8 {
            assert!(queue.push(job(&format!("ahead-{i}")), false).is_ok());
        }
        assert!(queue.push(job("overflow"), false).is_err());
        assert!(queue.push(job("visible"), true).is_ok());
        assert_eq!(queue.pop().unwrap().key, "visible");
        let cancelled = queue.update(HashSet::from(["ahead-7".into()]), HashSet::new());
        assert_eq!(cancelled.len(), 7);
        assert!(!queue.wanted(&job("ahead-0")));
        assert_eq!(queue.pop().unwrap().key, "ahead-7");
        queue.close();
        assert!(queue.pop().is_none());
    }
    #[test]
    fn newly_visible_prefetch_is_promoted() {
        let queue = ArtworkQueue::default();
        let keys: HashSet<String> = (0..8).map(|i| format!("cover-{i}")).collect();
        for i in 0..8 {
            assert!(queue.push(job(&format!("cover-{i}")), false).is_ok());
        }
        assert!(queue
            .update(keys, HashSet::from(["cover-7".into()]))
            .is_empty());
        assert_eq!(queue.pop().unwrap().key, "cover-7");
        queue.close();
    }
}
