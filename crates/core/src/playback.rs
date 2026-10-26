//! Queue advancement ported from DeezNats' PlaybackRules.cs.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Select the next queue index. `None` means playback should stop.
///
/// `current` is absent before playback starts. The caller supplies a random
/// picker returning an index in `0..upper_bound` so this core needs no RNG or
/// platform dependency. The picker is called only for shuffle with two or more
/// tracks; an out-of-range result is rejected.
pub fn next_index(
    current: Option<usize>,
    count: usize,
    repeat: RepeatMode,
    shuffle: bool,
    mut pick: impl FnMut(usize) -> usize,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let current = current.filter(|&index| index < count);
    if repeat == RepeatMode::One {
        return Some(current.unwrap_or(0));
    }
    let Some(current) = current else {
        return Some(0);
    };
    if shuffle && count > 1 {
        let candidate = pick(count - 1);
        return (candidate < count - 1).then(|| {
            if candidate >= current {
                candidate + 1
            } else {
                candidate
            }
        });
    }
    if current < count - 1 {
        Some(current + 1)
    } else if repeat == RepeatMode::All {
        Some(0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_random(_: usize) -> usize {
        panic!("randomness should not be needed");
    }

    #[test]
    fn original_playback_regressions() {
        assert_eq!(
            next_index(Some(2), 3, RepeatMode::Off, false, no_random),
            None
        );
        assert_eq!(
            next_index(Some(2), 3, RepeatMode::All, false, no_random),
            Some(0)
        );
        assert_eq!(
            next_index(Some(1), 3, RepeatMode::One, false, no_random),
            Some(1)
        );
        assert_eq!(
            next_index(Some(0), 1, RepeatMode::Off, true, no_random),
            None
        );
    }

    #[test]
    fn empty_start_and_stale_selection() {
        for repeat in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            assert_eq!(next_index(None, 0, repeat, true, no_random), None);
            assert_eq!(next_index(None, 3, repeat, false, no_random), Some(0));
            assert_eq!(
                next_index(Some(usize::MAX), 3, repeat, false, no_random),
                Some(0)
            );
        }
        assert_eq!(
            next_index(Some(0), 3, RepeatMode::Off, false, no_random),
            Some(1)
        );
        assert_eq!(
            next_index(Some(0), 1, RepeatMode::All, true, no_random),
            Some(0)
        );
        assert_eq!(
            next_index(Some(1), 3, RepeatMode::One, true, no_random),
            Some(1)
        );
    }

    #[test]
    fn shuffle_can_reach_every_other_track_without_repeating() {
        for count in 2..20 {
            for current in 0..count {
                let actual: Vec<_> = (0..count - 1)
                    .map(|choice| {
                        next_index(Some(current), count, RepeatMode::Off, true, |bound| {
                            assert_eq!(bound, count - 1);
                            choice
                        })
                        .unwrap()
                    })
                    .collect();
                let expected: Vec<_> = (0..count).filter(|&i| i != current).collect();
                assert_eq!(actual, expected);
            }
        }
        assert_eq!(next_index(Some(0), 3, RepeatMode::Off, true, |_| 2), None);
    }
}
