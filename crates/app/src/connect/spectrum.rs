//! Frequency data from the engine's pre-volume PCM, never the Windows mix.
use std::time::{Duration, Instant};

const N: usize = 4096;
const RATE: f32 = 44100.0;
// Preserve headroom up to full-scale PCM. Ordinary musical peaks must not
// flatten against a ceiling intended for a much quieter signal.
pub(crate) fn dynamic_level(db: f32) -> f32 {
    ((db + 65.) / 65.).clamp(0., 1.).powf(2.4)
}

#[derive(Clone, Default)]
pub(crate) struct Snapshot {
    pub levels: [f32; 32],
    pub trap_levels: [f32; 32],
    pub updated: Option<Instant>,
}

impl Snapshot {
    pub fn fresh(&self) -> bool {
        self.updated
            .is_some_and(|time| time.elapsed() < Duration::from_millis(600))
    }
}

pub(crate) struct Analyzer {
    samples: [[f32; N]; 2],
    cursor: usize,
    filled: usize,
    last: Option<Instant>,
    scene: [f32; 32],
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            samples: [[0.0; N]; 2],
            cursor: 0,
            filled: 0,
            last: None,
            scene: [0.; 32],
        }
    }

    pub fn push(&mut self, pcm: &[u8]) -> Option<[f32; 32]> {
        for frame in pcm.chunks_exact(4) {
            self.samples[0][self.cursor] =
                i16::from_le_bytes([frame[0], frame[1]]) as f32 / 32768.0;
            self.samples[1][self.cursor] =
                i16::from_le_bytes([frame[2], frame[3]]) as f32 / 32768.0;
            self.cursor = (self.cursor + 1) % N;
            self.filled = (self.filled + 1).min(N);
        }
        if self.filled < N
            || self
                .last
                .is_some_and(|time| time.elapsed() < Duration::from_millis(20))
        {
            return None;
        }
        self.last = Some(Instant::now());
        let mut power = vec![0.0f32; N / 2];
        // Analyze both channels separately: anti-phase stereo must not cancel.
        for channel in &self.samples {
            let mut real = vec![0.0; N];
            let mut imag = vec![0.0; N];
            for (index, sample) in real.iter_mut().enumerate() {
                let window = 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / N as f32).cos();
                *sample = channel[(self.cursor + index) % N] * window;
            }
            fft(&mut real, &mut imag);
            for index in 0..power.len() {
                power[index] += (real[index] * real[index] + imag[index] * imag[index])
                    * (8.0 / (N * N) as f32);
            }
        }
        self.scene = bands_from_power(&power, true);
        Some(bands_from_power(&power, false))
    }
    pub(crate) fn scene_levels(&self) -> [f32; 32] {
        self.scene
    }
}
pub(crate) fn scene_frequency(position: f32) -> f32 {
    let logarithmic = 7.5 * (3840f32 / 7.5).powf(position);
    let linear = 7.5 + (3840. - 7.5) * position;
    logarithmic * 0.925 + linear * 0.075
}
fn bands_from_power(power: &[f32], scene: bool) -> [f32; 32] {
    let mut levels = [0.0; 32];
    for (index, level) in levels.iter_mut().enumerate() {
        let frequency = |position: f32| {
            if scene {
                scene_frequency(position)
            } else {
                40. * 300f32.powf(position)
            }
        };
        let low = frequency(index as f32 / 32.);
        let high = frequency((index + 1) as f32 / 32.);
        let first = ((low * N as f32 / RATE).floor() as usize).max(1);
        let last = ((high * N as f32 / RATE).ceil() as usize)
            .max(first + 1)
            .min(power.len());
        let band = &power[first..last];
        let peak = band.iter().copied().fold(0.0f32, f32::max);
        let energy = peak * 0.75 + band.iter().sum::<f32>() / band.len() as f32 * 0.25;
        let db = if energy > 0.0 {
            10.0 * energy.log10()
        } else {
            -120.0
        };
        *level = dynamic_level(db);
    }
    levels
}

fn fft(real: &mut [f32], imag: &mut [f32]) {
    let mut reversed = 0;
    for index in 1..N {
        let mut bit = N >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if index < reversed {
            real.swap(index, reversed);
            imag.swap(index, reversed);
        }
    }
    let mut length = 2;
    while length <= N {
        let angle = -std::f32::consts::TAU / length as f32;
        let (step_imag, step_real) = angle.sin_cos();
        for start in (0..N).step_by(length) {
            let (mut wr, mut wi) = (1.0, 0.0);
            for offset in 0..length / 2 {
                let a = start + offset;
                let b = a + length / 2;
                let vr = real[b] * wr - imag[b] * wi;
                let vi = real[b] * wi + imag[b] * wr;
                real[b] = real[a] - vr;
                imag[b] = imag[a] - vi;
                real[a] += vr;
                imag[a] += vi;
                (wr, wi) = (
                    wr * step_real - wi * step_imag,
                    wr * step_imag + wi * step_real,
                );
            }
        }
        length *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_range_does_not_remove_treble_from_the_main_spectrum() {
        let pcm: Vec<u8> = (0..N)
            .flat_map(|index| {
                let value = ((std::f32::consts::TAU * 6000. * index as f32 / RATE).sin()
                    * 0.3
                    * 32767.) as i16;
                let bytes = value.to_le_bytes();
                [bytes[0], bytes[1], bytes[0], bytes[1]]
            })
            .collect();
        let mut analyzer = Analyzer::new();
        let full = analyzer.push(&pcm).unwrap();
        assert!(full.iter().copied().fold(0., f32::max) > 0.45);
        assert!(analyzer.scene_levels().iter().all(|value| *value < 0.02));
        assert!((scene_frequency(0.) - 7.5).abs() < 0.01);
        assert!((scene_frequency(1.) - 3840.).abs() < 0.1);
    }
    fn tone(hz: f32, amplitude: f32, inverted: bool) -> [f32; 32] {
        let mut pcm = Vec::new();
        for index in 0..N {
            let sample = ((std::f32::consts::TAU * hz * index as f32 / RATE).sin()
                * amplitude
                * 32767.0) as i16;
            pcm.extend_from_slice(&sample.to_le_bytes());
            pcm.extend_from_slice(&(if inverted { -sample } else { sample }).to_le_bytes());
        }
        Analyzer::new().push(&pcm).unwrap()
    }
    #[test]
    fn real_pcm_keeps_bass_and_treble_in_their_frequency_regions() {
        for (hz, lower, upper) in [(80.0, 0, 8), (6000.0, 20, 31)] {
            let levels = tone(hz, 0.3, false);
            let peak = levels
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert!((lower..=upper).contains(&peak), "{hz}: {peak}");
            assert!(levels[peak] > 0.45 && levels[peak] < 0.85);
        }
    }
    #[test]
    fn silence_quiet_signals_and_antiphase_stereo_are_handled() {
        assert!(tone(80.0, 0.0, false).iter().all(|level| *level == 0.0));
        let quiet = tone(6000.0, 0.002, false)
            .into_iter()
            .fold(0.0f32, f32::max);
        assert!(quiet > 0.0 && quiet < 0.2);
        assert!(tone(80.0, 0.3, true).iter().copied().fold(0.0f32, f32::max) > 0.45);
        assert!(!Snapshot::default().fresh());
    }
    #[test]
    fn quiet_phrases_and_hits_retain_their_dynamic_range() {
        for hz in [80., 6000.] {
            let peaks: Vec<_> = [0.003, 0.03, 0.1, 0.3, 0.9]
                .into_iter()
                .map(|amplitude| tone(hz, amplitude, false).into_iter().fold(0., f32::max))
                .collect();
            assert!(peaks.windows(2).all(|v| v[1] > v[0]), "{hz}: {peaks:?}");
            assert!(
                peaks[1] < 0.25 && peaks[3] < 0.85 && peaks[4] > 0.85,
                "{hz}: {peaks:?}"
            );
        }
    }
}
