//! Decoded YouTube PCM, before Sink gain and host-only DSP. Never records an endpoint.
use rodio::{source::SeekError, Source};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

const SAMPLES: usize = 882; // 10 ms, 44.1 kHz stereo
static LEVELS: Mutex<Option<(u64, crate::connect::spectrum::Snapshot)>> = Mutex::new(None);
pub(crate) fn levels(trap: bool) -> Option<[f32; 32]> {
    let epoch = bus().lock().ok()?.epoch;
    let latest = LEVELS.lock().ok()?;
    let (generation, snapshot) = latest.as_ref()?;
    (*generation == epoch && snapshot.fresh()).then_some(if trap {
        snapshot.trap_levels
    } else {
        snapshot.levels
    })
}
fn analyzer() -> &'static std::sync::mpsc::SyncSender<(u64, Vec<u8>)> {
    static TX: OnceLock<std::sync::mpsc::SyncSender<(u64, Vec<u8>)>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<(u64, Vec<u8>)>(8);
        std::thread::spawn(move || {
            let mut analyzer = crate::connect::spectrum::Analyzer::new();
            let mut epoch = None;
            while let Ok((generation, pcm)) = rx.recv() {
                if epoch != Some(generation) {
                    analyzer = crate::connect::spectrum::Analyzer::new();
                    epoch = Some(generation);
                }
                if let Some(levels) = analyzer.push(&pcm) {
                    if let Ok(mut latest) = LEVELS.lock() {
                        *latest = Some((
                            generation,
                            crate::connect::spectrum::Snapshot {
                                levels,
                                trap_levels: analyzer.scene_levels(),
                                updated: Some(std::time::Instant::now()),
                            },
                        ));
                    }
                }
            }
        });
        tx
    })
}
#[derive(Default)]
struct Bus {
    epoch: u64,
    packets: VecDeque<Vec<u8>>,
}
fn bus() -> &'static Mutex<Bus> {
    static BUS: OnceLock<Mutex<Bus>> = OnceLock::new();
    BUS.get_or_init(Default::default)
}
pub(super) fn reset() {
    if let Ok(mut bus) = bus().lock() {
        bus.epoch = bus.epoch.wrapping_add(1);
        bus.packets.clear();
    }
}
pub(crate) fn drain() -> Vec<Vec<u8>> {
    bus()
        .lock()
        .map(|mut b| b.packets.drain(..).collect())
        .unwrap_or_default()
}

#[derive(Clone)]
pub(super) struct Eq {
    pub enabled: bool,
    pub gains: [f32; 11],
}
impl Default for Eq {
    fn default() -> Self {
        Self {
            enabled: false,
            gains: [0.; 11],
        }
    }
}
impl Eq {
    pub fn update(&mut self, value: &serde_json::Value) {
        if value["reset"] == true {
            self.gains = [0.; 11];
        }
        if let Some(preset) = value["preset"].as_str() {
            if let Some(gains) = crate::ui::preset_gains(&preset.replace('-', " ")) {
                for (out, gain) in self.gains.iter_mut().zip(gains) {
                    *out = gain as f32;
                }
            }
        }
        if let Some(enabled) = value["enabled"].as_bool() {
            self.enabled = enabled;
        }
        if let Some(gains) = value["gainsDb"].as_array() {
            for (out, gain) in self.gains[..10].iter_mut().zip(gains) {
                if let Some(gain) = gain.as_f64() {
                    *out = (gain as f32).clamp(-12., 12.);
                }
            }
        }
        if let Some(gain) = value["preampDb"].as_f64() {
            self.gains[10] = (gain as f32).clamp(-12., 12.);
        }
        if let (Some(index), Some(gain)) = (
            value["band"]["index"].as_u64(),
            value["band"]["gainDb"].as_f64(),
        ) {
            if index < 10 {
                self.gains[index as usize] = (gain as f32).clamp(-12., 12.);
            }
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Filter {
    c: [f32; 5],
    z: [[f32; 2]; 2],
}
impl Filter {
    fn configure(&mut self, hz: f32, db: f32) {
        let a = 10f32.powf(db / 40.);
        let w = 2. * std::f32::consts::PI * hz / 44100.;
        let alpha = w.sin() / 2. / 1.4;
        let a0 = 1. + alpha / a;
        self.c = [
            (1. + alpha * a) / a0,
            -2. * w.cos() / a0,
            (1. - alpha * a) / a0,
            -2. * w.cos() / a0,
            (1. - alpha / a) / a0,
        ];
    }
    fn sample(&mut self, x: f32, channel: usize) -> f32 {
        let [b0, b1, b2, a1, a2] = self.c;
        let z = &mut self.z[channel];
        let y = b0 * x + z[0];
        z[0] = b1 * x - a1 * y + z[1];
        z[1] = b2 * x - a2 * y;
        y
    }
}
pub(super) struct Routed<S> {
    source: S,
    epoch: u64,
    pcm: Vec<u8>,
    channel: usize,
    eq: Arc<Mutex<Eq>>,
    settings: Eq,
    filters: [Filter; 10],
}
impl<S: Source> Routed<S> {
    pub fn new(source: S, eq: Arc<Mutex<Eq>>) -> Self {
        debug_assert_eq!((source.channels(), source.sample_rate()), (2, 44100));
        let _ = analyzer();
        let epoch = bus().lock().map(|b| b.epoch).unwrap_or_default();
        Self {
            source,
            epoch,
            pcm: Vec::with_capacity(SAMPLES * 2),
            channel: 0,
            eq,
            settings: Eq::default(),
            filters: [Filter::default(); 10],
        }
    }
}
impl<S: Source> Iterator for Routed<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let Some(x) = self.source.next() else {
            if !self.pcm.is_empty() {
                self.publish();
            }
            return None;
        };
        if self.pcm.is_empty() {
            if let Ok(eq) = self.eq.try_lock() {
                self.settings = eq.clone();
            }
            for ((filter, hz), gain) in self
                .filters
                .iter_mut()
                .zip([
                    31., 62., 125., 250., 500., 1000., 2000., 4000., 8000., 16000.,
                ])
                .zip(self.settings.gains)
            {
                filter.configure(hz, gain);
            }
        }
        self.pcm
            .extend_from_slice(&((x.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
        if self.pcm.len() == SAMPLES * 2 {
            self.publish();
        }
        let mut y = x;
        if self.settings.enabled {
            for filter in &mut self.filters {
                y = filter.sample(y, self.channel);
            }
            y *= 10f32.powf(self.settings.gains[10] / 20.);
        }
        self.channel ^= 1;
        Some(y.clamp(-1., 1.))
    }
}
impl<S> Routed<S> {
    fn publish(&mut self) {
        let _ = analyzer().try_send((self.epoch, self.pcm.clone()));
        // Audio callback never waits for Connect or analysis. Bound backlog to 200 ms.
        if let Ok(mut bus) = bus().try_lock() {
            if bus.epoch == self.epoch {
                if bus.packets.len() == 20 {
                    bus.packets.pop_front();
                }
                bus.packets.push_back(self.pcm.clone());
            }
        }
        self.pcm.clear();
    }
}
impl<S: Source> Source for Routed<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.source.current_span_len()
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        44100
    }
    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.source.try_seek(pos)?;
        reset();
        self.epoch = bus().lock().map(|b| b.epoch).unwrap_or_default();
        self.pcm.clear();
        self.channel = 0;
        for filter in &mut self.filters {
            filter.z = [[0.; 2]; 2];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pre_volume_pcm_survives_host_mute_eq_and_resets() {
        reset();
        let eq = Arc::new(Mutex::new(Eq::default()));
        eq.lock().unwrap().update(
            &serde_json::json!({"enabled":true,"preampDb":-12,"band":{"index":5,"gainDb":-12}}),
        );
        let samples: Vec<f32> = (0..SAMPLES * 20)
            .map(|i| ((i / 2) as f32 * std::f32::consts::TAU * 1000. / 44100.).sin() * 0.5)
            .collect();
        let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples.clone());
        let routed = Routed::new(source, eq);
        // Same ordering used in production: Sink's gain is outside Routed.
        let local: Vec<_> = routed.amplify(0.).collect();
        assert!(local.iter().all(|x| *x == 0.));
        let pcm: Vec<u8> = drain().into_iter().flatten().collect();
        assert_eq!(pcm.len(), samples.len() * 2);
        for (raw, expected) in pcm.chunks_exact(2).zip(&samples) {
            assert_eq!(
                i16::from_le_bytes([raw[0], raw[1]]),
                (expected * 32767.).round() as i16
            );
        }
        let eq = Arc::new(Mutex::new(Eq::default()));
        eq.lock()
            .unwrap()
            .update(&serde_json::json!({"enabled": true, "preampDb": -12}));
        let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples.clone());
        let local: Vec<_> = Routed::new(source, eq).collect();
        assert!(local.iter().all(|x| x.abs() < 0.13));
        assert!(drain()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .chunks_exact(2)
            .any(|x| i16::from_le_bytes([x[0], x[1]]).abs() > 15000));
        let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples.clone());
        let mut routed = Routed::new(source, Arc::new(Mutex::new(Eq::default())));
        // Epoch invalidation prevents a stopped/replaced decoder publishing old packets.
        reset();
        for _ in 0..SAMPLES {
            routed.next();
        }
        assert!(drain().is_empty());
        let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples);
        let mut routed = Routed::new(source, Arc::new(Mutex::new(Eq::default())));
        for _ in 0..SAMPLES {
            routed.next();
        }
        assert!(!drain().is_empty());
        // Independent analysis works without constructing a ConnectHub or endpoint.
        routed.try_seek(Duration::ZERO).unwrap();
        assert!(drain().is_empty());
        for _ in 0..SAMPLES {
            routed.next();
        }
        assert!(!drain().is_empty());
        let source = rodio::buffer::SamplesBuffer::new(
            2,
            44100,
            (0..SAMPLES * 40)
                .map(|i| ((i / 2) as f32 * std::f32::consts::TAU * 1000. / 44100.).sin() * 0.5)
                .collect::<Vec<_>>(),
        );
        let mut routed = Routed::new(source, Arc::new(Mutex::new(Eq::default())));
        for _ in 0..40 {
            for _ in 0..SAMPLES {
                routed.next();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(levels(false).unwrap().iter().any(|level| *level > 0.));
        assert!(bus().lock().unwrap().packets.len() <= 20);
        reset();
        assert!(levels(false).is_none());
        // Exercise the actual loopback HTTP/WAV path with a muted synthetic host.
        use std::io::{Read, Write};
        let (tx, commands) = std::sync::mpsc::sync_channel(4);
        let hub = crate::connect::ConnectHub::start(0, tx).unwrap();
        hub.update(&crate::audio::Snapshot {
            provider: crate::catalog::Provider::YouTube,
            playback_connected: true,
            state: "Playing".into(),
            ..Default::default()
        });
        let port = hub.port();
        let client = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            stream
                .write_all(b"GET /api/stream?buffer_ms=250 HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
                assert!(head.len() < 4096);
            }
            assert!(head.starts_with(b"HTTP/1.1 200"));
            let mut wav = [0; 44];
            stream.read_exact(&mut wav).unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            let mut pcm = [0; 4096];
            stream.read_exact(&mut pcm).unwrap();
            assert!(pcm
                .chunks_exact(2)
                .any(|x| i16::from_le_bytes([x[0], x[1]]).abs() > 15000));
        });
        let source = rodio::buffer::SamplesBuffer::new(
            2,
            44100,
            (0..SAMPLES * 100)
                .map(|i| ((i / 2) as f32 * std::f32::consts::TAU * 1000. / 44100.).sin() * 0.5)
                .collect::<Vec<_>>(),
        );
        let mut muted = Routed::new(source, Arc::new(Mutex::new(Eq::default()))).amplify(0.);
        for _ in 0..100 {
            for _ in 0..SAMPLES {
                assert_eq!(muted.next(), Some(0.));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        client.join().unwrap();
        for (path, expected) in [
            ("/api/state", "\"provider\":\"youtube\""),
            ("/api/queue", "\"provider\":\"youtube\""),
            (
                "/api/search?q=test",
                "remote controls and catalog are unavailable",
            ),
        ] {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.contains(expected), "{response}");
        }
        assert!(commands.try_recv().is_err());
    }
}
