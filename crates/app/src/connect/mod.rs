//! DeezNats Connect: Embedded HTTP server supporting the full application interface in Blood Moon theme.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::SyncSender,
    },
    thread,
    time::{Duration, Instant},
};

mod mp3 {
    //! Per-listener MP3 encoding. Capture and host playback never wait for this work.
    use mp3lame_encoder::{Bitrate, Builder, InterleavedPcm, Mode, Quality};

    pub(super) struct Encoder {
        inner: mp3lame_encoder::Encoder,
        samples: Vec<i16>,
        output: Vec<u8>,
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Cursor;
        use symphonia::core::{
            audio::SampleBuffer, errors::Error, io::MediaSourceStream, probe::Hint,
        };

        #[test]
        fn live_mp3_decodes_incremental_stereo_pcm_at_the_expected_rate() {
            let mut encoder = Encoder::new().unwrap();
            let frames = 44100 * 5;
            let pcm: Vec<u8> = (0..frames)
                .flat_map(|n| {
                    [440., 1200.].into_iter().flat_map(move |hz| {
                        let sample = ((n as f64 * hz * std::f64::consts::TAU / 44100.).sin()
                            * 12000.) as i16;
                        sample.to_le_bytes()
                    })
                })
                .collect();
            let mut encoded = Vec::new();
            // Vary packet sizes, including single stereo frames. Never flush:
            // a browser must decode the ongoing response before its connection ends.
            let mut offset = 0;
            for chunk_frames in [1, 137, 441, 882, 4096].into_iter().cycle() {
                let end = (offset + chunk_frames * 4).min(pcm.len());
                encoded.extend_from_slice(encoder.encode(&pcm[offset..end]).unwrap());
                offset = end;
                if offset == pcm.len() {
                    break;
                }
            }
            assert!(
                (110_000..130_000).contains(&encoded.len()),
                "unexpected MP3 bandwidth: {}",
                encoded.len()
            );
            let source = MediaSourceStream::new(Box::new(Cursor::new(encoded)), Default::default());
            let mut hint = Hint::new();
            hint.with_extension("mp3");
            let mut format = symphonia::default::get_probe()
                .format(&hint, source, &Default::default(), &Default::default())
                .unwrap()
                .format;
            let track = format.default_track().unwrap();
            assert_eq!(track.codec_params.sample_rate, Some(44100));
            assert_eq!(track.codec_params.channels.unwrap().count(), 2);
            let mut decoder = symphonia::default::get_codecs()
                .make(&track.codec_params, &Default::default())
                .unwrap();
            let mut samples = Vec::new();
            loop {
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                        break;
                    }
                    Err(e) => panic!("invalid MP3 response: {e}"),
                };
                let decoded = decoder.decode(&packet).unwrap();
                let mut buffer =
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                buffer.copy_interleaved_ref(decoded);
                samples.extend_from_slice(buffer.samples());
            }
            assert!(
                (44100 * 48 / 10..44100 * 51 / 10).contains(&(samples.len() / 2)),
                "stream lost audio duration"
            );
            let power = |channel: usize, hz: f64| {
                let (mut re, mut im) = (0., 0.);
                for (n, frame) in samples.chunks_exact(2).enumerate().skip(4410) {
                    let phase = n as f64 * hz * std::f64::consts::TAU / 44100.;
                    re += frame[channel] as f64 * phase.cos();
                    im += frame[channel] as f64 * phase.sin();
                }
                re * re + im * im
            };
            assert!(
                power(0, 440.) > 100. * power(0, 1200.),
                "left channel corrupted"
            );
            assert!(
                power(1, 1200.) > 100. * power(1, 440.),
                "right channel corrupted"
            );
        }

        #[test]
        fn malformed_pcm_is_rejected_before_encoding() {
            let mut encoder = Encoder::new().unwrap();
            assert!(encoder.encode(&[0; 3]).is_err());
            assert!(encoder.encode(&[0; 4096]).is_ok());
        }
    }

    impl Encoder {
        pub(super) fn new() -> Result<Self, String> {
            let mut builder = Builder::new().ok_or("Could not allocate Connect MP3 encoder")?;
            let configured = (|| {
                builder.set_num_channels(2)?;
                builder.set_sample_rate(44100)?;
                builder.set_brate(Bitrate::Kbps192)?;
                builder.set_mode(Mode::JointStereo)?;
                builder.set_quality(Quality::NearBest)?;
                // Live responses cannot seek back to rewrite a duration/Xing tag.
                builder.set_to_write_vbr_tag(false)?;
                builder.build()
            })();
            let inner = configured
                .map_err(|e| format!("Could not initialize Connect MP3 encoder: {e:?}"))?;
            Ok(Self {
                inner,
                samples: Vec::new(),
                output: Vec::new(),
            })
        }

        pub(super) fn encode(&mut self, pcm: &[u8]) -> Result<&[u8], String> {
            if pcm.len() % 4 != 0 {
                return Err("Connect PCM is not aligned to stereo frames".into());
            }
            self.samples.clear();
            self.samples.extend(
                pcm.chunks_exact(2)
                    .map(|s| i16::from_le_bytes([s[0], s[1]])),
            );
            self.output.clear();
            self.output
                .reserve(mp3lame_encoder::max_required_buffer_size(
                    self.samples.len() / 2,
                ));
            self.inner
                .encode_to_vec(InterleavedPcm(&self.samples), &mut self.output)
                .map_err(|e| format!("Connect MP3 encoding failed: {e:?}"))?;
            // Do not flush between callbacks: preserve the continuous codec timeline.
            Ok(&self.output)
        }
    }
}
pub(crate) mod spectrum {
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
                    let window =
                        0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / N as f32).cos();
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
}
mod spotify_audio {
    //! Process-isolated Soloist capture. No endpoint enumeration or system-loopback fallback.
    //! Matches Microsoft's ApplicationLoopback sample: WASAPI converts/resamples into
    //! the existing Connect wire format (44,100 Hz, stereo, signed 16-bit little endian).
    use crate::spotify::service::AudioTarget;
    use std::{
        mem::ManuallyDrop,
        sync::{Arc, mpsc},
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
            Media::Audio::*,
            System::{
                Com::{
                    BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
                    StructuredStorage::{
                        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                    },
                },
                Threading::{CreateEventW, WaitForSingleObject},
                Variant::VT_BLOB,
            },
        },
        core::{HRESULT, Interface, Ref, implement},
    };

    pub(super) const RATE: u32 = 44_100;
    pub(super) const CHANNELS: u16 = 2;
    const FRAME_BYTES: usize = 4;
    const MAX_PACKET_FRAMES: u32 = RATE * 2;

    fn wire_format() -> WAVEFORMATEX {
        WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: CHANNELS,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * FRAME_BYTES as u32,
            nBlockAlign: FRAME_BYTES as u16,
            wBitsPerSample: 16,
            cbSize: 0,
        }
    }

    fn activation_params(pid: u32) -> AUDIOCLIENT_ACTIVATION_PARAMS {
        AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        }
    }

    fn unavailable(stage: &str, error: impl std::fmt::Display) -> String {
        format!(
            "Spotify process-isolated audio unavailable ({stage}: {error}). Requires Windows application-loopback support (build 20348 or newer). No system audio fallback is used."
        )
    }

    struct Apartment;
    impl Apartment {
        fn new() -> Result<Self, String> {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
                .map_err(|e| unavailable("COM initialization", e))?;
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    // The callback retains both the blob and PROPVARIANT through late completion,
    // even if the owner cancels/drops a pending activation. Neither is mutated after
    // publication. The blob pointer addresses a stable Box, never stack storage.
    struct ActivationStorage {
        variant: ManuallyDrop<PROPVARIANT>,
        _params: Box<AUDIOCLIENT_ACTIVATION_PARAMS>,
    }
    unsafe impl Send for ActivationStorage {}
    unsafe impl Sync for ActivationStorage {}
    impl ActivationStorage {
        fn new(pid: u32) -> Self {
            let mut params = Box::new(activation_params(pid));
            let variant = PROPVARIANT {
                Anonymous: PROPVARIANT_0 {
                    Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                        vt: VT_BLOB,
                        Anonymous: PROPVARIANT_0_0_0 {
                            blob: BLOB {
                                cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                                pBlobData: (&mut *params as *mut AUDIOCLIENT_ACTIVATION_PARAMS)
                                    .cast(),
                            },
                        },
                        ..Default::default()
                    }),
                },
            };
            // Do not PropVariantClear this borrowed blob; Box owns the allocation.
            Self {
                variant: ManuallyDrop::new(variant),
                _params: params,
            }
        }
    }

    // windows::implement supplies IAgileObject and a free-threaded marshaler by
    // default. Windows invokes this callback on an MTA worker; no UI thread is used.
    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Completion {
        ready: mpsc::SyncSender<()>,
        _storage: Arc<ActivationStorage>,
    }
    impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
        fn ActivateCompleted(
            &self,
            _: Ref<IActivateAudioInterfaceAsyncOperation>,
        ) -> windows::core::Result<()> {
            let _ = self.ready.try_send(());
            Ok(())
        }
    }

    struct Event(HANDLE);
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    struct Running {
        // Release interfaces before the event. COM apartment outlives all of these.
        capture: IAudioCaptureClient,
        client: IAudioClient,
        event: Event,
        max_frames: u32,
    }
    impl Running {
        fn new(client: IAudioClient) -> windows::core::Result<Self> {
            unsafe {
                let event = Event(CreateEventW(None, false, false, None)?);
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    0,
                    0,
                    &wire_format(),
                    None,
                )?;
                // The process-loopback resampler can return aligned packets beyond
                // the nominal period (observed 448 frames for a 441-frame buffer).
                // Keep a bounded conversion margin as well as the hard packet cap.
                let max_frames = client
                    .GetBufferSize()?
                    .saturating_add(128)
                    .min(MAX_PACKET_FRAMES);
                let capture = client.GetService::<IAudioCaptureClient>()?;
                client.SetEventHandle(event.0)?;
                // Install the stop guard before Start, including partial failures.
                let running = Self {
                    capture,
                    client,
                    event,
                    max_frames,
                };
                running.client.Start()?;
                Ok(running)
            }
        }

        fn read(&self, output: &mut Vec<u8>) -> Result<(), String> {
            unsafe {
                let event = WaitForSingleObject(self.event.0, 0);
                if event == WAIT_FAILED {
                    return Err(unavailable("capture event", "wait failed"));
                }
                if event != WAIT_OBJECT_0 {
                    return Ok(());
                }
                // Bounded draining: let the hub observe stop/provider/PID changes.
                for _ in 0..32 {
                    let next = self
                        .capture
                        .GetNextPacketSize()
                        .map_err(|e| unavailable("packet size", e))?;
                    if next == 0 {
                        break;
                    }
                    let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
                    self.capture
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                        .map_err(|e| unavailable("capture buffer", e))?;
                    // Always release a successful GetBuffer, including invalid packets.
                    let packet = append_packet(output, data, frames, flags, self.max_frames);
                    let released = self
                        .capture
                        .ReleaseBuffer(frames)
                        .map_err(|e| unavailable("release buffer", e));
                    packet?;
                    released?;
                    if output.len() >= RATE as usize * FRAME_BYTES / 10 {
                        break;
                    }
                }
            }
            Ok(())
        }
    }
    impl Drop for Running {
        fn drop(&mut self) {
            unsafe {
                let _ = self.client.Stop();
            }
        }
    }

    /// Only trusts the exact format passed to Initialize. Windows performs all
    /// channel/sample conversion; no GetMixFormat guesswork or float reinterpretation.
    unsafe fn append_packet(
        output: &mut Vec<u8>,
        data: *const u8,
        frames: u32,
        flags: u32,
        max_frames: u32,
    ) -> Result<(), String> {
        if frames > max_frames || frames > MAX_PACKET_FRAMES {
            return Err(unavailable(
                "packet",
                format!("invalid frame count {frames}; capture buffer {max_frames}"),
            ));
        }
        let bytes = frames as usize * FRAME_BYTES;
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            // SILENT packets may have a null data pointer. Preserve their duration.
            output.resize(output.len() + bytes, 0);
        } else if bytes != 0 {
            if data.is_null() {
                return Err(unavailable("packet", "missing PCM data"));
            }
            output.extend_from_slice(std::slice::from_raw_parts(data, bytes));
        }
        // DATA_DISCONTINUITY: forward valid frames, never invent a gap from a wall
        // clock. TIMESTAMP_ERROR: ignore unreliable timestamps, preserve sample count.
        Ok(())
    }

    enum Phase {
        Pending {
            operation: IActivateAudioInterfaceAsyncOperation,
            _handler: IActivateAudioInterfaceCompletionHandler,
            ready: mpsc::Receiver<()>,
            started: Instant,
        },
        Running(Running),
    }

    pub(super) struct Capture {
        phase: Phase,
        pub target: Arc<AudioTarget>,
        _apartment: Apartment,
    }
    impl Capture {
        pub fn start(target: Arc<AudioTarget>) -> Result<Self, String> {
            let apartment = Apartment::new()?;
            if !target.is_alive() || target.pid == 0 {
                return Err(
                    "Spotify audio unavailable: the supervised Soloist process stopped".into(),
                );
            }
            let storage = Arc::new(ActivationStorage::new(target.pid));
            let (tx, ready) = mpsc::sync_channel(1);
            let handler: IActivateAudioInterfaceCompletionHandler = Completion {
                ready: tx,
                _storage: storage.clone(),
            }
            .into();
            let operation = unsafe {
                ActivateAudioInterfaceAsync(
                    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                    &IAudioClient::IID,
                    Some(&*storage.variant),
                    &handler,
                )
            }
            .map_err(|e| unavailable("process activation", e))?;
            Ok(Self {
                phase: Phase::Pending {
                    operation,
                    _handler: handler,
                    ready,
                    started: Instant::now(),
                },
                target,
                _apartment: apartment,
            })
        }

        pub fn read(&mut self, output: &mut Vec<u8>) -> Result<(), String> {
            if !self.target.is_alive() {
                return Err("Spotify audio unavailable: Soloist exited".into());
            }
            if let Phase::Pending {
                operation,
                ready,
                started,
                ..
            } = &self.phase
            {
                match ready.try_recv() {
                    Ok(()) => {
                        let mut result = HRESULT(0);
                        let mut activated = None;
                        let running = unsafe {
                            operation
                                .GetActivateResult(&mut result, &mut activated)
                                .and_then(|_| result.ok())
                                .and_then(|_| {
                                    activated.ok_or_else(|| {
                                        windows::core::Error::from_hresult(HRESULT(
                                            0x80004003u32 as i32,
                                        ))
                                    })
                                })
                                .and_then(|unknown| unknown.cast::<IAudioClient>())
                                .and_then(Running::new)
                        }
                        .map_err(|e| unavailable("process capture initialization", e))?;
                        self.phase = Phase::Running(running);
                    }
                    Err(mpsc::TryRecvError::Empty)
                        if started.elapsed() < Duration::from_secs(5) =>
                    {
                        return Ok(());
                    }
                    _ => return Err(unavailable("process activation", "completion timed out")),
                }
            }
            if let Phase::Running(running) = &self.phase {
                running.read(output)?;
            }
            Ok(())
        }
    }

    /// Capture ownership is tied to a service generation, never merely a PID.
    pub(super) fn same_target(
        current: Option<&Arc<AudioTarget>>,
        next: Option<&Arc<AudioTarget>>,
    ) -> bool {
        match (current, next) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn protocol_format_and_activation_are_exact_and_process_isolated() {
            let format = wire_format();
            // WAVEFORMATEX is packed; copy fields before assertions.
            let fields = (
                format.wFormatTag,
                format.nChannels,
                format.nSamplesPerSec,
                format.nAvgBytesPerSec,
                format.nBlockAlign,
                format.wBitsPerSample,
                format.cbSize,
            );
            assert_eq!(fields, (1, 2, 44100, 176400, 4, 16, 0));
            let params = activation_params(321);
            assert_eq!(
                params.ActivationType,
                AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK
            );
            let process = unsafe { params.Anonymous.ProcessLoopbackParams };
            assert_eq!(process.TargetProcessId, 321);
            assert_eq!(
                process.ProcessLoopbackMode,
                PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
            );
            let storage = ActivationStorage::new(321);
            let variant = unsafe { &storage.variant.Anonymous.Anonymous };
            assert_eq!(variant.vt, VT_BLOB);
            let blob = unsafe { variant.Anonymous.blob };
            assert_eq!(
                blob.cbSize as usize,
                std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()
            );
            assert_eq!(
                blob.pBlobData.cast::<AUDIOCLIENT_ACTIVATION_PARAMS>(),
                &*storage._params as *const _ as *mut _
            );
        }

        #[test]
        fn packets_preserve_stereo_endianness_flags_and_silent_duration() {
            let samples = [-32768i16, 32767, -123, 456];
            let pcm: Vec<u8> = samples.iter().flat_map(|n| n.to_le_bytes()).collect();
            let mut output = Vec::new();
            unsafe {
                append_packet(
                    &mut output,
                    pcm.as_ptr(),
                    2,
                    (AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0
                        | AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0) as u32,
                    10,
                )
                .unwrap();
                append_packet(
                    &mut output,
                    std::ptr::null(),
                    3,
                    AUDCLNT_BUFFERFLAGS_SILENT.0 as u32,
                    10,
                )
                .unwrap();
            }
            assert_eq!(&output[..8], &pcm);
            assert_eq!(&output[8..], &[0; 12]);
            assert_eq!(output.len(), 5 * FRAME_BYTES);
        }

        #[test]
        fn malformed_packets_are_rejected_without_mutating_the_stream() {
            let mut output = vec![1, 2, 3, 4];
            unsafe {
                assert!(append_packet(&mut output, std::ptr::null(), 1, 0, 10).is_err());
                assert!(append_packet(&mut output, std::ptr::null(), 11, 2, 10).is_err());
                assert!(
                    append_packet(&mut output, std::ptr::null(), u32::MAX, 2, u32::MAX).is_err()
                );
                append_packet(&mut output, std::ptr::null(), 0, 0, 10).unwrap();
            }
            assert_eq!(output, [1, 2, 3, 4]);
        }

        #[test]
        fn completion_is_agile_and_survives_owner_cancellation() {
            let (tx, rx) = mpsc::sync_channel(1);
            let storage = Arc::new(ActivationStorage::new(321));
            let weak = Arc::downgrade(&storage);
            let handler: IActivateAudioInterfaceCompletionHandler = Completion {
                ready: tx,
                _storage: storage,
            }
            .into();
            assert!(
                handler
                    .cast::<windows::Win32::System::Com::IAgileObject>()
                    .is_ok()
            );
            let os_reference = handler.clone();
            drop(handler);
            assert!(weak.upgrade().is_some());
            unsafe {
                os_reference.ActivateCompleted(None).unwrap();
            }
            rx.recv_timeout(Duration::from_millis(50)).unwrap();
            drop(os_reference);
            assert!(weak.upgrade().is_none());
        }

        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        #[test]
        fn process_exit_and_service_generation_invalidate_capture_identity() {
            use std::os::windows::process::CommandExt;
            let mut child = Child(
                std::process::Command::new("powershell.exe")
                    .args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "Start-Sleep -Seconds 30",
                    ])
                    .creation_flags(0x08000000)
                    .spawn()
                    .unwrap(),
            );
            let first = Arc::new(AudioTarget::from_child(&child.0).unwrap());
            let replacement = Arc::new(AudioTarget::from_child(&child.0).unwrap());
            assert!(first.is_alive());
            assert!(same_target(Some(&first), Some(&first.clone())));
            // Same PID with a new service publication is still a new generation.
            assert!(!same_target(Some(&first), Some(&replacement)));
            assert!(!same_target(Some(&first), None));
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            assert!(!first.is_alive());
            assert!(!replacement.is_alive());
        }

        // This child fixture is inert in ordinary test runs. Only the explicitly
        // ignored integration test below sets its private tone-rendering argument.
        #[test]
        fn synthetic_renderer_child() {
            let Some(hz) = std::env::var("DEEZNATS_CAPTURE_FIXTURE_HZ")
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
            else {
                return;
            };
            let _apartment = Apartment::new().unwrap();
            unsafe {
                use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
                let devices: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
                let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).unwrap();
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();
                client
                    .Initialize(
                        AUDCLNT_SHAREMODE_SHARED,
                        AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                        1_000_000,
                        0,
                        &wire_format(),
                        None,
                    )
                    .unwrap();
                let frames = client.GetBufferSize().unwrap();
                let renderer: IAudioRenderClient = client.GetService().unwrap();
                let volume: ISimpleAudioVolume = client.GetService().unwrap();
                // Set each fixture explicitly: Windows can retain session volume
                // from an earlier fixture process using the same executable.
                let level = if std::env::var_os("DEEZNATS_CAPTURE_FIXTURE_MUTED").is_some() {
                    0.
                } else {
                    1.
                };
                volume.SetMasterVolume(level, std::ptr::null()).unwrap();
                client.Start().unwrap();
                let start = Instant::now();
                let mut cursor = 0u64;
                while start.elapsed() < Duration::from_secs(15) {
                    let count = frames - client.GetCurrentPadding().unwrap();
                    if count > 0 {
                        let ptr = renderer.GetBuffer(count).unwrap();
                        let samples =
                            std::slice::from_raw_parts_mut(ptr.cast::<i16>(), count as usize * 2);
                        for pair in samples.chunks_exact_mut(2) {
                            let value = ((std::f64::consts::TAU * hz * cursor as f64 / RATE as f64)
                                .sin()
                                * 1300.) as i16;
                            pair.fill(value);
                            cursor += 1;
                        }
                        renderer.ReleaseBuffer(count, 0).unwrap();
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                client.Stop().unwrap();
            }
        }

        #[test]
        #[ignore = "opt-in: silent process capture test; requires a Windows audio endpoint"]
        fn muted_render_session_produces_silent_process_capture() {
            use std::os::windows::process::CommandExt;
            let child = Child(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "connect::spotify_audio::tests::synthetic_renderer_child",
                        "--nocapture",
                    ])
                    .env("DEEZNATS_CAPTURE_FIXTURE_HZ", "440")
                    .env("DEEZNATS_CAPTURE_FIXTURE_MUTED", "1")
                    .creation_flags(0x08000000)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let target = Arc::new(AudioTarget::from_child(&child.0).unwrap());
            let mut capture = Capture::start(target).unwrap();
            let mut pcm = Vec::new();
            let mut peak = 0i32;
            let mut captured_bytes = 0;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(4) {
                pcm.clear();
                capture.read(&mut pcm).unwrap();
                captured_bytes += pcm.len();
                for sample in pcm.chunks_exact(2) {
                    peak = peak.max(i16::from_le_bytes([sample[0], sample[1]]) as i32);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            // This documents the integration limit, not volume independence:
            // process loopback cannot recover samples muted before its capture tap.
            assert!(
                captured_bytes > RATE as usize * FRAME_BYTES,
                "capture did not run"
            );
            assert!(
                peak < 10,
                "muted process capture unexpectedly retained signal: peak={peak}"
            );
        }

        #[test]
        #[ignore = "opt-in: renders quiet tones in two child processes; requires a Windows audio endpoint"]
        fn synthetic_process_capture_excludes_unrelated_audio() {
            use std::os::windows::process::CommandExt;
            let render = |hz: u32| {
                Child(
                    std::process::Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "connect::spotify_audio::tests::synthetic_renderer_child",
                            "--nocapture",
                        ])
                        .env("DEEZNATS_CAPTURE_FIXTURE_HZ", hz.to_string())
                        .creation_flags(0x08000000)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .unwrap(),
                )
            };
            let included = render(523);
            let excluded = render(2701);
            let mut capture =
                Capture::start(Arc::new(AudioTarget::from_child(&included.0).unwrap())).unwrap();
            let start = Instant::now();
            let mut pcm = Vec::new();
            while start.elapsed() < Duration::from_secs(4) {
                capture.read(&mut pcm).unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
            let signal: Vec<f64> = pcm
                .chunks_exact(4)
                .map(|f| i16::from_le_bytes([f[0], f[1]]) as f64)
                .collect();
            assert!(signal.len() > RATE as usize, "no captured sample timeline");
            let magnitude = |hz: f64| {
                let mut real = 0.;
                let mut imag = 0.;
                for (n, sample) in signal.iter().enumerate() {
                    let phase = std::f64::consts::TAU * hz * n as f64 / RATE as f64;
                    real += sample * phase.cos();
                    imag += sample * phase.sin();
                }
                real.hypot(imag) / signal.len() as f64
            };
            let wanted = magnitude(523.);
            let other = magnitude(2701.);
            assert!(wanted > 100., "target tone missing: {wanted}");
            assert!(
                other < wanted * 0.02,
                "unrelated process leaked: {other} / {wanted}"
            );
            drop(capture);
            drop((included, excluded));
        }
    }
}
pub(crate) use spectrum::{dynamic_level, scene_frequency};

use crate::audio::{Command, Snapshot};
use crate::catalog::{Entry, Provider};
use crate::engine::Engine as Deezer;
use std::sync::{Mutex, OnceLock};

static VISUALIZER_SPECTRUM: OnceLock<RwLock<spectrum::Snapshot>> = OnceLock::new();
pub(crate) fn visualizer_spectrum() -> Option<[f32; 32]> {
    if Provider::active() == Provider::YouTube {
        return crate::youtube::routing::levels(false);
    }
    let value = VISUALIZER_SPECTRUM.get()?.read().ok()?;
    value.fresh().then_some(value.levels)
}
pub(crate) fn visualizer_trap_spectrum() -> Option<[f32; 32]> {
    if Provider::active() == Provider::YouTube {
        return crate::youtube::routing::levels(true);
    }
    let value = VISUALIZER_SPECTRUM.get()?.read().ok()?;
    value.fresh().then_some(value.trap_levels)
}
pub struct AudioBroadcaster {
    clients: Mutex<Vec<AudioClient>>,
}
struct AudioClient {
    sender: std::sync::mpsc::Sender<Arc<Vec<u8>>>,
    queued: Arc<AtomicUsize>,
    listener: std::sync::Weak<()>,
}
pub struct AudioSubscription {
    receiver: std::sync::mpsc::Receiver<Arc<Vec<u8>>>,
    _listener: Arc<()>,
    queued: Arc<AtomicUsize>,
}
// Three seconds of wire PCM, independent of capture callback size.
const MAX_QUEUED_PCM: usize = 44100 * 2 * 2 * 3;
impl AudioSubscription {
    fn received(&self, pcm: Arc<Vec<u8>>) -> Arc<Vec<u8>> {
        self.queued.fetch_sub(pcm.len(), Ordering::AcqRel);
        pcm
    }
    fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Arc<Vec<u8>>, std::sync::mpsc::RecvTimeoutError> {
        self.receiver
            .recv_timeout(timeout)
            .map(|pcm| self.received(pcm))
    }
    #[cfg(test)]
    fn try_recv(&self) -> Result<Arc<Vec<u8>>, std::sync::mpsc::TryRecvError> {
        self.receiver.try_recv().map(|pcm| self.received(pcm))
    }
}

static BROADCASTER: OnceLock<AudioBroadcaster> = OnceLock::new();

pub fn broadcaster() -> &'static AudioBroadcaster {
    BROADCASTER.get_or_init(|| AudioBroadcaster {
        clients: Mutex::new(Vec::new()),
    })
}

impl AudioBroadcaster {
    pub fn subscribe(&self) -> AudioSubscription {
        let (tx, rx) = std::sync::mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let listener = Arc::new(());
        if let Ok(mut clients) = self.clients.lock() {
            clients.push(AudioClient {
                sender: tx,
                queued: queued.clone(),
                listener: Arc::downgrade(&listener),
            });
        }
        AudioSubscription {
            receiver: rx,
            _listener: listener,
            queued,
        }
    }
    fn has_listeners(&self) -> bool {
        self.clients
            .lock()
            .map(|mut clients| {
                clients.retain(|c| c.listener.strong_count() > 0);
                !clients.is_empty()
            })
            .unwrap_or(false)
    }
    fn publish(&self, pcm: Arc<Vec<u8>>) {
        if pcm.is_empty() {
            return;
        }
        if let Ok(mut clients) = self.clients.lock() {
            // Disconnect slow clients rather than dropping timeline samples.
            clients.retain(|c| {
                if c.listener.strong_count() == 0 {
                    return false;
                }
                let size = pcm.len();
                if c.queued
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                        queued
                            .checked_add(size)
                            .filter(|total| *total <= MAX_QUEUED_PCM)
                    })
                    .is_err()
                {
                    return false;
                }
                if c.sender.send(pcm.clone()).is_err() {
                    c.queued.fetch_sub(size, Ordering::AcqRel);
                    return false;
                }
                true
            });
        }
    }
}

fn publish_pcm(
    state: &RwLock<HubState>,
    active: &AtomicBool,
    spectrum_tx: &std::sync::mpsc::SyncSender<(Provider, u64, Arc<Vec<u8>>)>,
    generation: u64,
    pcm: &[u8],
    provider: Provider,
) {
    if let Ok(st) = state.read() {
        if !active.load(Ordering::Relaxed) || st.snapshot.provider != provider {
            return;
        }
        let pcm = Arc::new(pcm.to_vec());
        broadcaster().publish(pcm.clone());
        // Visualisation is best effort and must never block the audio stream.
        let _ = spectrum_tx.try_send((provider, generation, pcm));
    }
}

fn wav_header(sample_rate: u32, channels: u16) -> [u8; 44] {
    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&channels.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * (channels as u32) * 2;
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    let block_align = channels * 2;
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    header
}

pub const DEFAULT_PORT: u16 = 3131;

fn attribute_entry(mut entry: Entry, payload: &serde_json::Value) -> Entry {
    if let Some(key) = payload[crate::style_mix::MARKER]
        .as_str()
        .filter(|key| crate::style_mix::Selection::parse(key).is_ok())
    {
        entry.raw[crate::style_mix::MARKER] = serde_json::json!(key);
    }
    let nickname: String = payload["added_by"]
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(64)
        .collect();
    entry.raw["added_by"] = serde_json::json!(nickname);
    entry
}

#[derive(Clone, Default)]
pub struct ConnectTrack {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    #[allow(dead_code)]
    pub album: String,
    pub artwork: String,
    pub duration: u64,
}

impl ConnectTrack {
    #[allow(dead_code)]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id, "kind": self.kind,
            "title": self.title,
            "artist": self.artist,
            "album": self.album,
            "artwork": self.artwork,
            "duration": self.duration / 1000, "duration_ms": self.duration,
        })
    }

    pub fn from_value(val: &serde_json::Value) -> Option<Self> {
        let id = val.get("id").and_then(|v| {
            v.as_str()
                .map(ToString::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })?;
        let title = val
            .get("title")
            .and_then(serde_json::Value::as_str)?
            .to_string();
        let artist = val
            .get("artist")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let album = val
            .get("album")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let artwork = val
            .get("artwork")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let duration = val
            .get("duration_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| {
                val.get("duration")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    .saturating_mul(1000)
            });

        let kind = match val.get("kind").and_then(serde_json::Value::as_str) {
            Some("episode") => "episode",
            _ => "track",
        }
        .to_string();
        Some(Self {
            kind,
            id,
            title,
            artist,
            album,
            artwork,
            duration,
        })
    }

    pub fn to_entry(&self) -> Entry {
        Entry {
            id: self.id.clone(),
            title: self.title.clone(),
            subtitle: self.artist.clone(),
            artwork: self.artwork.clone(),
            kind: self.kind.clone(),
            duration: self.duration,
            album: self.album.clone(),
            ..Default::default()
        }
    }
}

pub struct HubState {
    presence: std::collections::HashMap<String, (String, Instant)>,
    queue_revision: u64,
    spotify_audio_error: Option<String>,
    pub spectrum: spectrum::Snapshot,
    pub snapshot: Snapshot,
    pub engine: Option<Arc<Deezer>>,
}

pub struct ConnectHub {
    audio_thread: Option<thread::JoinHandle<()>>,
    server_thread: Option<thread::JoinHandle<()>>,
    state: Arc<RwLock<HubState>>,
    #[allow(dead_code)]
    command_tx: SyncSender<Command>,
    active: Arc<AtomicBool>,
    #[allow(dead_code)]
    port: u16,
}

impl ConnectHub {
    pub fn start(port: u16, command_tx: SyncSender<Command>) -> Option<Self> {
        let bind_host = if cfg!(test) { "127.0.0.1" } else { "0.0.0.0" };
        let listener = TcpListener::bind((bind_host, port))
            .or_else(|error| {
                if port == DEFAULT_PORT {
                    TcpListener::bind((bind_host, DEFAULT_PORT + 1))
                } else {
                    Err(error)
                }
            })
            .ok()?;
        let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        let _ = listener.set_nonblocking(true);
        let active = Arc::new(AtomicBool::new(true));
        let state = Arc::new(RwLock::new(HubState {
            presence: Default::default(),
            queue_revision: 0,
            spotify_audio_error: None,
            spectrum: spectrum::Snapshot::default(),
            snapshot: Snapshot::default(),
            engine: None,
        }));

        let audio_active = active.clone();
        let audio_state = state.clone();
        let audio_thread = thread::spawn(move || {
            let mut buffer = vec![0u8; 262144];
            let (spectrum_tx, spectrum_rx) =
                std::sync::mpsc::sync_channel::<(Provider, u64, Arc<Vec<u8>>)>(16);
            let spectrum_state = audio_state.clone();
            let spectrum_active = audio_active.clone();
            let spectrum_epoch = Arc::new(AtomicU64::new(0));
            let worker_epoch = spectrum_epoch.clone();
            let spectrum_thread = thread::spawn(move || {
                let mut analyzer = spectrum::Analyzer::new();
                let mut generation = None;
                while let Ok((provider, current, pcm)) = spectrum_rx.recv() {
                    if !spectrum_active.load(Ordering::Relaxed) {
                        break;
                    }
                    if current != worker_epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    if generation != Some(current) {
                        analyzer = spectrum::Analyzer::new();
                        generation = Some(current);
                    }
                    if let Some(levels) = analyzer.push(&pcm) {
                        if let Ok(mut st) = spectrum_state.write() {
                            if st.snapshot.provider != provider
                                || current != worker_epoch.load(Ordering::Acquire)
                            {
                                continue;
                            }
                            let snapshot = spectrum::Snapshot {
                                levels,
                                trap_levels: analyzer.scene_levels(),
                                updated: Some(std::time::Instant::now()),
                            };
                            if let Ok(mut latest) = VISUALIZER_SPECTRUM
                                .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                                .write()
                            {
                                *latest = snapshot.clone();
                            }
                            st.spectrum = snapshot;
                        }
                    }
                }
            });
            let mut spectrum_generation = 0u64;
            let mut enabled_engine: Option<Arc<Deezer>> = None;
            let mut spotify_capture: Option<spotify_audio::Capture> = None;
            let mut previous_target = None;
            let mut previous_provider = Provider::Deezer;
            while audio_active.load(Ordering::Relaxed) {
                let (provider, engine) = audio_state
                    .read()
                    .map(|s| (s.snapshot.provider, s.engine.clone()))
                    .unwrap_or((Provider::Deezer, None));
                let target = if provider == Provider::Spotify {
                    crate::spotify::service::audio_target()
                } else {
                    None
                };
                if provider != previous_provider
                    || !spotify_audio::same_target(previous_target.as_ref(), target.as_ref())
                {
                    spotify_capture = None;
                    spectrum_generation = spectrum_generation.wrapping_add(1);
                    spectrum_epoch.store(spectrum_generation, Ordering::Release);
                    if let Ok(mut st) = audio_state.write() {
                        st.spectrum = spectrum::Snapshot::default();
                        st.spotify_audio_error = None;
                        if let Ok(mut latest) = VISUALIZER_SPECTRUM
                            .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                            .write()
                        {
                            *latest = spectrum::Snapshot::default();
                        }
                        if previous_target.is_some() || provider != previous_provider {
                            if let Ok(mut clients) = broadcaster().clients.lock() {
                                clients.clear();
                            }
                        }
                    }
                    previous_provider = provider;
                    previous_target = target.clone();
                }
                if engine.is_none() {
                    if let Some(previous) = enabled_engine.take() {
                        unsafe {
                            if let Ok(disable) = previous
                                .library
                                .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                            {
                                disable(0);
                            }
                        }
                    }
                }
                if let Some(engine) = engine {
                    unsafe {
                        if let (Ok(enable), Ok(read)) = (
                            engine
                                .library
                                .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0"),
                            engine
                                .library
                                .get::<unsafe extern "C" fn(*mut u8, i32) -> i32>(
                                    b"DZReadBroadcast\0",
                                ),
                        ) {
                            if !enabled_engine
                                .as_ref()
                                .is_some_and(|previous| Arc::ptr_eq(previous, &engine))
                            {
                                if let Some(previous) = enabled_engine.take() {
                                    if let Ok(disable) = previous
                                        .library
                                        .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                                    {
                                        disable(0);
                                    }
                                }
                                enable(1);
                                enabled_engine = Some(engine.clone());
                            }
                            loop {
                                if !audio_active.load(Ordering::Relaxed)
                                    || !audio_state
                                        .read()
                                        .is_ok_and(|st| st.snapshot.provider == Provider::Deezer)
                                {
                                    break;
                                }
                                let n = read(buffer.as_mut_ptr(), buffer.len() as i32);
                                if n <= 0 {
                                    break;
                                }
                                publish_pcm(
                                    &audio_state,
                                    &audio_active,
                                    &spectrum_tx,
                                    spectrum_generation,
                                    &buffer[..n as usize],
                                    Provider::Deezer,
                                );
                            }
                        }
                    }
                }
                if provider == Provider::YouTube {
                    for pcm in crate::youtube::routing::drain() {
                        publish_pcm(
                            &audio_state,
                            &audio_active,
                            &spectrum_tx,
                            spectrum_generation,
                            &pcm,
                            Provider::YouTube,
                        );
                    }
                }
                if provider == Provider::Spotify && broadcaster().has_listeners() {
                    let result = (|| {
                        let target = target.ok_or_else(|| "Spotify audio unavailable: start the host's supervised Soloist player first".to_string())?;
                        if spotify_capture.is_none() {
                            spotify_capture = Some(spotify_audio::Capture::start(target)?);
                            if let Ok(mut st) = audio_state.write() {
                                st.spotify_audio_error = None;
                            }
                        }
                        buffer.clear();
                        spotify_capture.as_mut().unwrap().read(&mut buffer)?;
                        if !buffer.is_empty() {
                            // Revalidate generation after asynchronous activation/capture.
                            let current = crate::spotify::service::audio_target();
                            if spotify_audio::same_target(
                                current.as_ref(),
                                spotify_capture.as_ref().map(|c| &c.target),
                            ) {
                                publish_pcm(
                                    &audio_state,
                                    &audio_active,
                                    &spectrum_tx,
                                    spectrum_generation,
                                    &buffer,
                                    Provider::Spotify,
                                );
                            }
                        }
                        Ok::<(), String>(())
                    })();
                    if let Err(error) = result {
                        spotify_capture = None;
                        spectrum_generation = spectrum_generation.wrapping_add(1);
                        spectrum_epoch.store(spectrum_generation, Ordering::Release);
                        if let Ok(mut st) = audio_state.write() {
                            st.spotify_audio_error = Some(error);
                            st.spectrum = spectrum::Snapshot::default();
                            if let Ok(mut latest) = VISUALIZER_SPECTRUM
                                .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                                .write()
                            {
                                *latest = spectrum::Snapshot::default();
                            }
                            if let Ok(mut clients) = broadcaster().clients.lock() {
                                clients.clear();
                            }
                        }
                    }
                } else {
                    if spotify_capture.take().is_some() {
                        spectrum_generation = spectrum_generation.wrapping_add(1);
                        spectrum_epoch.store(spectrum_generation, Ordering::Release);
                        if let Ok(mut st) = audio_state.write() {
                            st.spectrum = spectrum::Snapshot::default();
                        }
                        if let Ok(mut latest) = VISUALIZER_SPECTRUM
                            .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                            .write()
                        {
                            *latest = spectrum::Snapshot::default();
                        }
                    }
                }
                // The Deezer ABI reads into initialized storage; Spotify appends.
                buffer.resize(262144, 0);
                thread::sleep(Duration::from_millis(5));
            }
            drop(spectrum_tx);
            let _ = spectrum_thread.join();
            if let Some(engine) = enabled_engine {
                unsafe {
                    if let Ok(enable) = engine
                        .library
                        .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                    {
                        enable(0);
                    }
                }
            }
        });
        let active_clone = active.clone();
        let state_clone = state.clone();
        let cmd_tx_clone = command_tx.clone();

        let server_thread = thread::Builder::new()
            .name("deeznats-connect".into())
            .spawn(move || {
                while active_clone.load(Ordering::Relaxed) {
                    let stream: std::io::Result<TcpStream> = match listener.accept() {
                        Ok((stream, _)) => Ok(stream),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(25));
                            continue;
                        }
                        Err(_) => break,
                    };
                    if !active_clone.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(mut stream) = stream {
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        let st = state_clone.clone();
                        let cmd = cmd_tx_clone.clone();
                        let active = active_clone.clone();
                        thread::spawn(move || {
                            handle_client(&mut stream, &st, &cmd, &active);
                        });
                    }
                }
            })
            .ok()?;

        Some(Self {
            audio_thread: Some(audio_thread),
            server_thread: Some(server_thread),
            state,
            command_tx,
            active,
            port: actual_port,
        })
    }

    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn share_url(&self) -> String {
        if cfg!(test) {
            return format!("http://127.0.0.1:{}/", self.port);
        }
        // UDP connect selects the LAN route without sending a packet.
        let ip = std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|socket| {
                socket.connect("192.0.2.1:80")?;
                socket.local_addr()
            })
            .map(|address| address.ip())
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        format!("http://{ip}:{}/", self.port)
    }

    pub fn nicknames(&self) -> Vec<String> {
        let Ok(mut state) = self.state.write() else {
            return Vec::new();
        };
        state
            .presence
            .retain(|_, (_, seen)| seen.elapsed() < Duration::from_secs(45));
        let mut names: Vec<_> = state
            .presence
            .values()
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    pub fn update(&self, snapshot: &Snapshot) {
        if let Ok(mut st) = self.state.write() {
            if st.snapshot.provider != snapshot.provider
                || !Arc::ptr_eq(&st.snapshot.queue, &snapshot.queue)
            {
                st.queue_revision = st.queue_revision.wrapping_add(1);
            }
            if st.snapshot.provider != snapshot.provider {
                if let Ok(mut clients) = broadcaster().clients.lock() {
                    clients.clear();
                }
            }
            st.engine = if snapshot.provider == Provider::Deezer {
                snapshot.engine.clone()
            } else {
                None
            };
            st.snapshot = snapshot.clone();
        }
    }
}

impl Drop for ConnectHub {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Ok(mut clients) = broadcaster().clients.lock() {
            clients.clear();
        }
        if let Some(handle) = self.audio_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.server_thread.take() {
            let _ = handle.join();
        }
    }
}

fn handle_client(
    stream: &mut TcpStream,
    state: &Arc<RwLock<HubState>>,
    command_tx: &SyncSender<Command>,
    active: &Arc<AtomicBool>,
) {
    if !active.load(Ordering::Relaxed) {
        return;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut buffer = vec![0u8; 16384];
    let mut bytes_read = match stream.read(&mut buffer) {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    // TCP may split headers and JSON across reads.
    let header_end = loop {
        if let Some(pos) = buffer[..bytes_read]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
        {
            break pos + 4;
        }
        if bytes_read == buffer.len() {
            return;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return;
        };
        if stream.set_read_timeout(Some(remaining)).is_err() {
            return;
        }
        match stream.read(&mut buffer[bytes_read..]) {
            Ok(n) if n > 0 => bytes_read += n,
            _ => return,
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > 1024 * 1024 {
        return;
    }
    let total = header_end + content_length;
    if buffer.len() < total {
        buffer.resize(total, 0);
    }
    while bytes_read < total {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return;
        };
        if stream.set_read_timeout(Some(remaining)).is_err() {
            return;
        }
        match stream.read(&mut buffer[bytes_read..total]) {
            Ok(n) if n > 0 => bytes_read += n,
            _ => return,
        }
    }
    let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);
    let mut lines = request_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return,
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let full_path = parts.next().unwrap_or("/");

    let (path, query) = full_path.split_once('?').unwrap_or((full_path, ""));

    let body_start = request_str
        .find("\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(bytes_read);
    let body_slice = &request_str[body_start..];

    // A captured host provider prevents requests falling through to Deezer
    // catalog calls or public fallbacks while Spotify is selected.
    let host = state.read().unwrap().snapshot.clone();
    if method == "POST" && path == "/api/presence" {
        let payload = serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
        let id = payload["id"].as_str().unwrap_or("");
        let name = payload["nickname"].as_str().unwrap_or("").trim();
        if !id.is_empty()
            && id.len() <= 100
            && name.chars().count() <= 64
            && !name.chars().any(char::is_control)
        {
            if let Ok(mut state) = state.write() {
                state
                    .presence
                    .retain(|_, (_, seen)| seen.elapsed() < Duration::from_secs(45));
                if name.is_empty() {
                    state.presence.remove(id);
                } else if state.presence.len() < 256 || state.presence.contains_key(id) {
                    state
                        .presence
                        .insert(id.to_owned(), (name.to_owned(), Instant::now()));
                }
            }
        }
        send_json(stream, &serde_json::json!({"ok": true}));
        return;
    }
    let provider_name = if host.provider == Provider::Spotify {
        "spotify"
    } else if host.provider == Provider::YouTube {
        "youtube"
    } else {
        "deezer"
    };
    if get_query_param(query, "host_provider").is_some_and(|p| p != provider_name) {
        send_error(stream, "Host provider changed. Refresh the catalog.");
        return;
    }
    if method == "POST" && path.starts_with("/api/") && path != "/api/presence" {
        let payload = serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
        if !valid_host_payload(host.provider, &payload) {
            send_error(
                stream,
                "Host provider changed. Refresh the catalog and try again.",
            );
            return;
        }
    }
    let command_tx = HostCommands {
        tx: command_tx,
        state,
        provider: host.provider,
    };
    if matches!(host.provider, Provider::Spotify | Provider::YouTube) {
        if let Some(result) =
            spotify_station_request(method, path, query, body_slice, &host, &command_tx)
        {
            match result {
                Ok(value) => send_json(stream, &value),
                Err(error) => send_error(stream, &error),
            }
            return;
        }
        if let Some(result) = (host.provider == Provider::Spotify)
            .then(|| spotify_guest_request(method, path, query, &host))
            .flatten()
        {
            match result {
                Ok(value) => send_json(stream, &value),
                Err(error) => send_error(stream, &error),
            }
            return;
        }
    }
    if host.provider == Provider::YouTube {
        if let Some(result) = youtube_guest_request(method, path, query) {
            match result {
                Ok(value) => send_json(stream, &value),
                Err(error) => send_error(stream, &error),
            }
            return;
        }
    }
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            let html = include_str!("index.html");
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(resp.as_bytes());
        }
        ("GET", "/ui.js") | ("GET", "/ui.css") | ("GET", "/translations.json") => {
            let (content, mime) = if path == "/ui.js" {
                (include_str!("ui.js"), "text/javascript")
            } else if path == "/ui.css" {
                (include_str!("ui.css"), "text/css")
            } else {
                (include_str!("translations.json"), "application/json")
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                content.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(content.as_bytes());
        }
        ("GET", path) if path.starts_with("/style-mix/") => {
            if let Some(bytes) = crate::style_mix::artwork::http_data(path) {
                let mime = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                    "image/jpeg"
                } else {
                    "image/webp"
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: public, max-age=86400\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(bytes);
            } else {
                send_error(stream, "Unknown Style Mix artwork");
            }
        }
        ("GET", "/native-icons.ttf") => {
            let font = std::env::var_os("WINDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "C:/Windows".into())
                .join("Fonts/segmdl2.ttf");
            if let Ok(bytes) = std::fs::read(font) {
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: font/ttf\r\nContent-Length: {}\r\nCache-Control: public, max-age=86400\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&bytes);
            }
        }
        ("GET", "/api/appearance") => send_json(stream, &browser_appearance()),
        ("GET", "/api/stream") => {
            // The default remains WAV for existing/native clients.
            let mut encoder = if get_query_param(query, "format").as_deref() == Some("mp3") {
                match mp3::Encoder::new() {
                    Ok(encoder) => Some(encoder),
                    Err(error) => {
                        send_error(stream, &error);
                        return;
                    }
                }
            } else {
                None
            };
            let stream_target = if host.provider == Provider::Spotify {
                crate::spotify::service::audio_target()
            } else {
                None
            };
            let source_current = || {
                state
                    .read()
                    .is_ok_and(|s| s.snapshot.provider == host.provider)
                    && (host.provider != Provider::Spotify
                        || spotify_audio::same_target(
                            stream_target.as_ref(),
                            crate::spotify::service::audio_target().as_ref(),
                        ))
            };
            let available = if host.provider == Provider::Spotify {
                stream_target.is_some()
            } else if host.provider == Provider::YouTube {
                host.playback_connected
            } else {
                state
                    .read()
                    .ok()
                    .and_then(|s| s.engine.clone())
                    .is_some_and(|engine| unsafe {
                        engine
                            .library
                            .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                            .is_ok()
                            && engine
                                .library
                                .get::<unsafe extern "C" fn(*mut u8, i32) -> i32>(
                                    b"DZReadBroadcast\0",
                                )
                                .is_ok()
                    })
            };
            if !available {
                send_error(
                    stream,
                    if host.provider == Provider::Spotify {
                        "Spotify audio unavailable: start the host's supervised Soloist player first"
                    } else {
                        "Jam requires the updated playback engine and a Deezer connection"
                    },
                );
                return;
            }
            let _ = stream.set_nodelay(true);
            let _ = stream.set_read_timeout(None);
            let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
            let rx = broadcaster().subscribe();
            let (sample_rate, channels) = (spotify_audio::RATE, spotify_audio::CHANNELS);
            let header = wav_header(sample_rate, channels);
            let compressed = encoder.is_some();
            let content_type = if compressed {
                "audio/mpeg"
            } else {
                "audio/wav"
            };
            let http_head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccept-Ranges: none\r\nConnection: close\r\nCache-Control: no-cache, no-store, must-revalidate\r\nPragma: no-cache\r\nExpires: 0\r\nAccess-Control-Allow-Origin: *\r\n\r\n"
            );
            // Reserve bounded jitter headroom. Native clients retain the 500 ms
            // default; browsers can request more for mobile links and recovery.
            let buffer_bytes = stream_buffer_bytes(query);
            let mut prebuffer = Vec::with_capacity(buffer_bytes);
            let started = std::time::Instant::now();
            while prebuffer.len() < buffer_bytes && active.load(Ordering::Relaxed) {
                if !source_current() {
                    send_error(stream, "Host audio source changed. Reconnect to listen.");
                    return;
                }
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(chunk) => prebuffer.extend_from_slice(&chunk),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        let error = state
                            .read()
                            .ok()
                            .and_then(|s| s.spotify_audio_error.clone())
                            .unwrap_or_else(|| {
                                "Host audio source changed. Reconnect to listen.".into()
                            });
                        send_error(stream, &error);
                        return;
                    }
                    Err(_) if started.elapsed() > Duration::from_secs(10) => {
                        let error = if host.provider == Provider::Spotify {
                            "Spotify capture produced insufficient PCM to start listening. Confirm playback is on the host's supervised Soloist device; process capture does not capture other Spotify devices."
                        } else {
                            if host.provider == Provider::YouTube {
                                "YouTube produced insufficient PCM to start listening. Start playback on the desktop player."
                            } else {
                                "The Deezer engine produced insufficient PCM to start listening. Check the host audio output and matching playback DLL."
                            }
                        };
                        if host.provider == Provider::Spotify {
                            if let Ok(mut st) = state.write() {
                                if st.snapshot.provider == Provider::Spotify {
                                    st.spotify_audio_error = Some(error.into());
                                }
                            }
                        }
                        send_error(stream, error);
                        return;
                    }
                    Err(_) => continue,
                }
            }
            if !active.load(Ordering::Relaxed) || !source_current() {
                return;
            }
            // Encode before committing the response so initialization/input errors
            // can still be reported as HTTP errors instead of corrupt audio.
            let initial = if let Some(encoder) = &mut encoder {
                match encoder.encode(&prebuffer) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        send_error(stream, &error);
                        return;
                    }
                }
            } else {
                &prebuffer
            };
            if stream.write_all(http_head.as_bytes()).is_err() {
                return;
            }
            if !compressed && stream.write_all(&header).is_err() {
                return;
            }
            if stream.write_all(initial).is_err() {
                return;
            }
            while active.load(Ordering::Relaxed) && source_current() {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(chunk) => {
                        if !source_current() {
                            break;
                        }
                        let bytes = if let Some(encoder) = &mut encoder {
                            match encoder.encode(&chunk) {
                                Ok(bytes) => bytes,
                                Err(_) => break,
                            }
                        } else {
                            &chunk
                        };
                        if stream.write_all(bytes).is_err() {
                            break;
                        }
                    }
                    // Only the engine produces PCM (including paused silence).
                    // A wall-clock timeout must never alter the audio timeline.
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
            return;
        }
        ("GET", "/api/lyrics") => {
            let (engine, track, duration) = (host.engine.clone(), host.now.clone(), host.duration);
            let id = track.id.clone();
            let ui = crate::ui::Ui::load_preferences();
            let result = engine
                .ok_or_else(|| "Connect the selected provider on the host first".to_string())
                .and_then(|engine| {
                    crate::lyrics::fetch_best_lyrics(&engine, &track, &id, ui.lyrics_sources)
                });
            match result {
                Ok((provider, lines)) => {
                    let gaps = crate::lyrics::instrumental_gaps(&lines, duration);
                    let timed = crate::lyrics::has_timing(&lines);
                    let lines: Vec<_> = lines.iter().map(|l| {
                        let syllables: Vec<_> = l.syllables.iter().map(|w| serde_json::json!({"start_ms":w.start_ms,"end_ms":w.end_ms,"text":w.text})).collect();
                        serde_json::json!({"start_ms":l.start_ms,"end_ms":l.end_ms,"text":l.text,"translation":l.translation,"syllables":syllables})
                    }).collect();
                    send_json(
                        stream,
                        &serde_json::json!({"id": id, "provider": provider, "lines": lines, "karaoke": ui.karaoke_lyrics, "blur": ui.blur_inactive_lyrics, "gaps": gaps, "timed": timed}),
                    );
                }
                Err(error) => send_json(
                    stream,
                    &serde_json::json!({"id": id, "lines": [], "error": error}),
                ),
            }
        }
        ("GET", "/api/spectrum") => {
            let (levels, available) = state
                .read()
                .map(|st| {
                    let fresh = st.spectrum.fresh();
                    (if fresh { st.spectrum.levels } else { [0.0; 32] }, fresh)
                })
                .unwrap_or(([0.0; 32], false));
            send_json(
                stream,
                &serde_json::json!({"levels":levels,"available":available}),
            );
        }
        ("GET", "/api/queue") => {
            let (snapshot, revision) = {
                let st = state.read().unwrap();
                (st.snapshot.clone(), st.queue_revision)
            };
            send_json(
                stream,
                &serde_json::json!({
                "provider": if snapshot.provider == Provider::Spotify { "spotify" } else if snapshot.provider == Provider::YouTube { "youtube" } else { "deezer" },
                    "queue_revision": revision,
                    "queue": connect_queue(&snapshot),
                }),
            );
        }
        ("GET", "/api/state") => {
            let json = {
                let st = state.read().unwrap();
                let snap = &st.snapshot;

                let album = snap
                    .now
                    .raw
                    .pointer("/album/title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();

                // Sample the engine now, rather than forwarding a UI snapshot
                // that may already be several hundred milliseconds old.
                let (position, playback_state) = if let Some(engine) = &st.engine {
                    unsafe {
                        let playback_state = match (engine.state)() {
                            1 => "loading",
                            2 => "playing",
                            3 => "paused",
                            4 => "error",
                            _ => "stopped",
                        };
                        (
                            (engine.position)().max(0) as u64,
                            playback_state.to_string(),
                        )
                    }
                } else {
                    (snap.position, snap.state.to_ascii_lowercase())
                };
                let mut json = serde_json::json!({
                "provider": if snap.provider == Provider::Spotify { "spotify" } else if snap.provider == Provider::YouTube { "youtube" } else { "deezer" },
                "stream_available": if snap.provider == Provider::Spotify { crate::spotify::service::audio_target().is_some() } else if snap.provider == Provider::YouTube { snap.playback_connected } else { st.engine.is_some() },
                "stream_error": if snap.provider == Provider::Spotify { st.spotify_audio_error.clone() } else { None },
                "connected": snap.connected,
                    "state": playback_state,
                    "title": snap.now.title,
                    "artist": snap.now.subtitle,
                    "album": album,
                    "artwork": snap.now.artwork,
                    "id": snap.now.id,
                    "position_ms": position,
                    "duration_ms": snap.duration,
                    "queue_index": snap.queue_index,
                    "queue_revision": st.queue_revision,
                    "repeat": snap.repeat,
                    "shuffle": snap.shuffle,
                    "autoplay": snap.autoplay,
                });
                // Existing clients keep the full response; browser polls only
                // fetch the large queue separately when its revision changes.
                if get_query_param(query, "queue").as_deref() != Some("0") {
                    json["queue"] = serde_json::json!(connect_queue(snap));
                }
                json
            };
            send_json(stream, &json);
        }
        ("GET", "/api/discover") => {
            let engine = host.engine.clone();
            let json = fetch_discover(engine.as_deref());
            send_json(stream, &json);
        }
        ("GET", "/api/style-mix/genres") => {
            let region =
                get_query_param(query, "region").and_then(|region| region.parse::<usize>().ok());
            let result = region
                .ok_or_else(|| "Choose a valid language".to_string())
                .and_then(|region| {
                    crate::style_mix::Selection::new(region, 0)?;
                    let engine = host
                        .engine
                        .as_deref()
                        .ok_or("Connect Deezer on the host first")?;
                    crate::style_mix::available_genres(engine, region)
                });
            match result {
                Ok(value) => send_json(stream, &value),
                Err(error) => send_json(
                    stream,
                    &serde_json::json!({"region":region,"error":error,"genres":[]}),
                ),
            }
        }
        ("GET", "/api/style-mix") => {
            let mut response = crate::style_mix::options();
            if let Some(key) = get_query_param(query, "selection") {
                response["selection"] = serde_json::json!(key);
                let engine = host.engine.clone();
                let result = crate::style_mix::Selection::parse(&key).and_then(|selection| {
                    let engine = engine
                        .as_deref()
                        .ok_or("Connect Deezer on the host first")?;
                    crate::style_mix::tracks(engine, selection).map(|tracks| (selection, tracks))
                });
                match result {
                    Ok((selection, tracks)) => {
                        response["selection"] = serde_json::json!(selection.key());
                        response["title"] = serde_json::json!(selection.title());
                        response["tracks"] = serde_json::json!(
                            tracks.iter().map(style_mix_track).collect::<Vec<_>>()
                        );
                    }
                    Err(error) => response["error"] = serde_json::json!(error),
                }
            }
            send_json(stream, &response);
        }
        ("POST", "/api/style-mix/start") => {
            let payload = serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
            let engine = host.engine.clone();
            let result =
                crate::style_mix::Selection::parse(payload["selection"].as_str().unwrap_or(""))
                    .and_then(|selection| {
                        crate::style_mix::tracks(
                            engine
                                .as_deref()
                                .ok_or("Connect Deezer on the host first")?,
                            selection,
                        )
                    });
            match result {
                Ok(tracks) => {
                    let tracks: Vec<_> = tracks
                        .into_iter()
                        .map(|e| attribute_entry(e, &payload))
                        .collect();
                    let first = if let Some(id) = payload["track_id"].as_str() {
                        tracks.iter().find(|t| t.id == id)
                    } else {
                        tracks.first()
                    };
                    if let Some(first) = first {
                        if command_tx
                            .send(Command::PlayEntry(Box::new(first.clone()), tracks))
                            .is_ok()
                        {
                            send_ok(stream);
                        } else {
                            send_error(stream, "Playback is unavailable");
                        }
                    } else {
                        send_error(stream, "Track is no longer available in this mix");
                    }
                }
                Err(error) => send_error(stream, &error),
            }
        }
        ("GET", "/api/flow") => {
            let config_id = get_query_param(query, "config").unwrap_or_default();
            let engine = host.engine.clone();
            let json = fetch_flow(engine.as_deref(), &config_id);
            send_json(stream, &json);
        }
        ("POST", "/api/flow/start") => {
            let engine = host.engine.clone();
            let config_id = if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                val.get("config")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string()
            } else {
                String::new()
            };
            if let Some(dz) = engine {
                let call = if config_id.is_empty() || config_id == "default" {
                    dz.invoke("DZFlowJSON", &[])
                } else {
                    dz.invoke("DZFlowForConfigJSON", &[serde_json::json!(config_id)])
                };
                if let Ok(val) = call {
                    let payload =
                        serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
                    let tracks: Vec<_> = crate::catalog::entries(&val, "track")
                        .into_iter()
                        .map(|entry| attribute_entry(entry, &payload))
                        .collect();
                    if let Some(first) = tracks.first() {
                        send_command_result(
                            stream,
                            command_tx.send(Command::PlayEntry(Box::new(first.clone()), tracks)),
                        );
                        return;
                    }
                }
            }
            send_error(stream, "Flow could not be started");
        }
        ("GET", "/api/flow/tuner") => {
            let flow_id = get_query_param(query, "id").unwrap_or_else(|| "default".to_string());
            let engine = host.engine.clone();
            if let Some(dz) = engine {
                if let Ok(tuner) = dz.invoke("DZFlowTunerJSON", &[serde_json::json!(flow_id)]) {
                    send_json(stream, &tuner);
                    return;
                }
            }
            let fallback_genres = [
                ("132", "Pop", true),
                ("152", "Rock", true),
                ("116", "Rap/Hip Hop", true),
                ("113", "Dance/Electro", true),
                ("165", "R&B", true),
                ("153", "Metal", false),
                ("129", "Jazz", false),
                ("98", "Classical", false),
                ("85", "Alternative", true),
                ("464", "Folk/Acoustic", false),
                ("144", "Reggae", false),
                ("84", "Country", false),
                ("52", "Blues", false),
                ("71", "Indie", true),
                ("173", "Soundtrack", false),
            ];
            let genres: Vec<_> = fallback_genres
                .iter()
                .map(|(id, title, enabled)| {
                    serde_json::json!({
                        "id": id,
                        "title": title,
                        "isEnabled": enabled,
                    })
                })
                .collect();
            send_json(
                stream,
                &serde_json::json!({
                    "id": flow_id,
                    "customizable": true,
                    "genres": genres,
                }),
            );
        }
        ("POST", "/api/flow/tuner/save") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                let flow_id = val
                    .get("flow_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("default")
                    .to_string();

                let edits: Vec<serde_json::Value> = if let Some(edits_arr) =
                    val.get("edits").and_then(serde_json::Value::as_array)
                {
                    edits_arr.clone()
                } else if let Some(genres_arr) =
                    val.get("genres").and_then(serde_json::Value::as_array)
                {
                    genres_arr
                        .iter()
                        .filter_map(|g| {
                            let id = g.get("id").or_else(|| g.get("clusterId"))?.as_str()?;
                            let enabled = g
                                .get("isEnabled")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            Some(serde_json::json!({
                                "clusterId": id,
                                "isEnabled": enabled,
                                "isEditedByUser": true,
                            }))
                        })
                        .collect()
                } else {
                    Vec::new()
                };

                if flow_selection_is_empty(&val) {
                    send_json(
                        stream,
                        &serde_json::json!({"error":"Select at least one genre", "ok":false}),
                    );
                    return;
                }
                let engine = host.engine.clone();
                if let Some(dz) = engine {
                    let edits_str = serde_json::to_string(&edits).unwrap_or_default();
                    let invoke_res = dz.invoke(
                        "DZSaveFlowTunerJSON",
                        &[serde_json::json!(flow_id), serde_json::json!(edits_str)],
                    );
                    // A successful read does not prove a failed write succeeded.
                    match invoke_res {
                        Err(error) => {
                            send_json(stream, &serde_json::json!({"ok":false,"error":error}))
                        }
                        Ok(result) => {
                            if result.get("saved").and_then(serde_json::Value::as_bool)
                                == Some(false)
                            {
                                send_json(
                                    stream,
                                    &serde_json::json!({"ok":false,"error":"Deezer did not save the selection"}),
                                );
                            } else {
                                match dz.invoke("DZFlowTunerJSON", &[serde_json::json!(flow_id)]) {
                                    Ok(tuner) => send_json(
                                        stream,
                                        &serde_json::json!({"ok":true,"tuner":tuner}),
                                    ),
                                    Err(_) => send_json(
                                        stream,
                                        &serde_json::json!({"ok":true,"saved":true,"verified":false}),
                                    ),
                                }
                            }
                        }
                    }
                    return;
                } else {
                    send_json(
                        stream,
                        &serde_json::json!({"ok":false,"error":"Connect Deezer on the host before saving Flow"}),
                    );
                    return;
                }
            }
            send_error(stream, "Flow tercihleri kaydedilemedi");
        }

        ("GET", "/api/collection") => {
            let kind = get_query_param(query, "kind").unwrap_or_default();
            let id = get_query_param(query, "id").unwrap_or_default();
            let engine = host.engine.clone();
            let json = fetch_collection(engine.as_deref(), &kind, &id);
            send_json(stream, &json);
        }
        ("GET", "/api/search") => {
            let q = get_query_param(query, "q").unwrap_or_default();
            let engine = host.engine.clone();
            let results = search_deezer(engine.as_deref(), &q);
            send_json(stream, &results);
        }
        // Queue editing: Add Last ("Sıranın sonuna ekle")
        ("POST", "/api/queue/add-last") | ("POST", "/api/queue") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(track) = ConnectTrack::from_value(&val) {
                    send_command_result(
                        stream,
                        command_tx.send(Command::QueueAddLast(Box::new(attribute_entry(
                            track.to_entry(),
                            &val,
                        )))),
                    );
                    return;
                }
            }
            send_error(stream, "Invalid track payload");
        }
        // Queue editing: Insert Next ("Sıradaki şarkı olarak ekle")
        ("POST", "/api/queue/insert-next") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(track) = ConnectTrack::from_value(&val) {
                    send_command_result(
                        stream,
                        command_tx.send(Command::QueueInsertNext(Box::new(attribute_entry(
                            track.to_entry(),
                            &val,
                        )))),
                    );
                    return;
                }
            }
            send_error(stream, "Invalid track payload");
        }
        // Queue editing: Drag to move queue around
        ("POST", "/api/queue/move") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let (Some(from), Some(to)) = (
                    val.get("from").and_then(serde_json::Value::as_u64),
                    val.get("to").and_then(serde_json::Value::as_u64),
                ) {
                    let expected: Vec<String> = {
                        let st = state.read().unwrap();
                        st.snapshot.queue.iter().map(|t| t.id.clone()).collect()
                    };
                    send_command_result(
                        stream,
                        command_tx.send(Command::QueueMove {
                            from: from as usize,
                            to: to as usize,
                            expected,
                        }),
                    );
                    return;
                }
            }
            send_error(stream, "Invalid move parameters");
        }
        // Queue editing: Jump/Play directly by queue index
        ("POST", "/api/queue/play-index") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(idx) = val.get("index").and_then(serde_json::Value::as_u64) {
                    send_command_result(
                        stream,
                        command_tx.send(Command::PlayQueueIndex(idx as usize)),
                    );
                    return;
                }
            }
            send_error(stream, "Invalid index");
        }
        // Queue editing: Clear queue
        ("POST", "/api/queue/clear") => {
            send_command_result(stream, command_tx.send(Command::QueueClear));
        }
        // Direct Play Track
        ("POST", "/api/play") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(track) = ConnectTrack::from_value(&val) {
                    let entry = attribute_entry(track.to_entry(), &val);
                    send_command_result(
                        stream,
                        command_tx.send(Command::PlayEntry(Box::new(entry.clone()), vec![entry])),
                    );
                    return;
                }
            }
            send_error(stream, "Invalid track");
        }
        // Play entire album/playlist collection
        ("POST", "/api/play-collection") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(items) = val.get("tracks").and_then(serde_json::Value::as_array) {
                    let entries: Vec<_> = items
                        .iter()
                        .filter_map(ConnectTrack::from_value)
                        .map(|t| attribute_entry(t.to_entry(), &val))
                        .collect();
                    if let Some(first) = entries.first() {
                        send_command_result(
                            stream,
                            command_tx.send(Command::PlayEntry(Box::new(first.clone()), entries)),
                        );
                        return;
                    }
                }
            }
            send_error(stream, "Invalid collection");
        }
        ("POST", "/api/toggle") => {
            send_command_result(stream, command_tx.send(Command::Toggle));
        }
        ("POST", "/api/next") => {
            send_command_result(stream, command_tx.send(Command::Next));
        }
        ("POST", "/api/previous") => {
            send_command_result(stream, command_tx.send(Command::Previous));
        }
        ("POST", "/api/shuffle") | ("POST", "/api/repeat") | ("POST", "/api/autoplay") => {
            if !state.read().map(|s| s.snapshot.connected).unwrap_or(false) {
                send_error(stream, "Connect the selected provider on the host first");
                return;
            }
            let value = serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
            let command = if path == "/api/shuffle" {
                value["enabled"].as_bool().map(Command::Shuffle)
            } else if path == "/api/autoplay" {
                value["enabled"].as_bool().map(Command::Autoplay)
            } else {
                value["mode"]
                    .as_u64()
                    .filter(|mode| *mode <= 2)
                    .map(|mode| Command::Repeat(mode as u8))
            };
            match command {
                Some(command) => {
                    if command_tx.try_send(command).is_ok() {
                        send_ok(stream)
                    } else {
                        send_error(stream, "Playback is busy. Please retry.")
                    }
                }
                None => send_error(stream, "Invalid playback mode"),
            }
        }
        ("POST", "/api/seek") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(ms) = val.get("position_ms").and_then(serde_json::Value::as_u64) {
                    send_command_result(stream, command_tx.send(Command::SeekMs(ms)));
                    return;
                }
            }
            send_error(stream, "Invalid seek");
        }
        ("OPTIONS", _) => {
            let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        }
        _ => {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        }
    }
}

struct HostCommands<'a> {
    tx: &'a SyncSender<Command>,
    state: &'a Arc<RwLock<HubState>>,
    provider: Provider,
}
impl HostCommands<'_> {
    fn send(&self, command: Command) -> Result<(), String> {
        self.try_send(command)
    }
    fn try_send(&self, command: Command) -> Result<(), String> {
        let state = self.state.read().map_err(|_| "Host unavailable")?;
        if state.snapshot.provider != self.provider || Provider::active() != self.provider {
            return Err("Host provider changed".into());
        }
        self.tx
            .try_send(command)
            .map_err(|_| "Playback is busy. Please retry.".into())
    }
}

fn valid_host_payload(provider: Provider, value: &serde_json::Value) -> bool {
    let name = if provider == Provider::Spotify {
        "spotify"
    } else if provider == Provider::YouTube {
        "youtube"
    } else {
        "deezer"
    };
    if value["provider"].as_str().is_some_and(|p| p != name) {
        return false;
    }
    if let Some(id) = value["id"].as_str() {
        if !id.is_empty()
            && (id.starts_with("spotify:") != (provider == Provider::Spotify)
                || id.starts_with("youtube:") != (provider == Provider::YouTube))
        {
            return false;
        }
    }
    value["tracks"].as_array().is_none_or(|tracks| {
        tracks.iter().all(|track| {
            if !track.is_object() {
                return false;
            }
            valid_host_payload(provider, &track)
        })
    })
}

fn spotify_station_request(
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    _host: &Snapshot,
    commands: &HostCommands<'_>,
) -> Option<Result<serde_json::Value, String>> {
    use serde_json::{Value, json};
    if !path.starts_with("/api/flow") && !path.starts_with("/api/style-mix") {
        return None;
    }
    let youtube = _host.provider == Provider::YouTube;
    let invoke = |name: &str, args: &[Value]| {
        if youtube {
            crate::youtube::invoke(name, args)
        } else {
            crate::spotify::catalog_invoke(name, args)
        }
    };
    let payload: Value = serde_json::from_str(body).unwrap_or_default();
    Some((|| match (method, path) {
        ("GET", "/api/flow") => {
            let config = get_query_param(query, "config").unwrap_or_else(|| "default".into());
            let options = invoke("DZFlowConfigsJSON", &[])?;
            let tracks = if youtube {
                crate::youtube::connect_station(false, &config)?
            } else {
                crate::catalog::entries(&invoke("DZFlowForConfigJSON", &[json!(config)])?, "track")
            };
            let tuner = invoke("DZFlowTunerJSON", &[json!(config)])?;
            Ok(
                json!({"options":options["options"],"tracks":tracks.iter().map(style_mix_track).collect::<Vec<_>>(),"config":config,"customizable":tuner["customizable"]}),
            )
        }
        ("GET", "/api/flow/tuner") => invoke(
            "DZFlowTunerJSON",
            &[json!(
                get_query_param(query, "id").unwrap_or_else(|| "default".into())
            )],
        ),
        ("POST", "/api/flow/tuner/save") => {
            if flow_selection_is_empty(&payload) {
                return Err("Select at least one genre".into());
            }
            let id = payload["flow_id"].as_str().unwrap_or("default");
            let edits: Vec<_> = payload["genres"].as_array().into_iter().flatten().map(|genre| json!({"clusterId":genre["id"],"isEnabled":genre["isEnabled"].as_bool().or_else(|| genre["enabled"].as_bool()).unwrap_or(false),"isEditedByUser":true})).collect();
            invoke(
                "DZSaveFlowTunerJSON",
                &[json!(id), json!(json!(edits).to_string())],
            )
        }
        ("GET", "/api/style-mix/genres") => {
            let region = get_query_param(query, "region")
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or("Choose a valid language")?;
            invoke("StyleMixGenres", &[json!(region)])
        }
        ("GET", "/api/style-mix") if get_query_param(query, "selection").is_none() => {
            Ok(crate::style_mix::options())
        }
        ("GET", "/api/style-mix")
        | ("POST", "/api/style-mix/start")
        | ("POST", "/api/flow/start") => {
            let style = path.starts_with("/api/style-mix");
            let (name, args, selection) = if style {
                let key = if method == "GET" {
                    get_query_param(query, "selection").unwrap_or_default()
                } else {
                    payload["selection"].as_str().unwrap_or_default().to_owned()
                };
                let selection = crate::style_mix::Selection::parse(&key)?;
                (
                    "SPStyleMixJSON",
                    vec![json!(selection.key())],
                    Some(selection),
                )
            } else {
                (
                    "DZFlowForConfigJSON",
                    vec![json!(payload["config"].as_str().unwrap_or("default"))],
                    None,
                )
            };
            let entries = if youtube {
                crate::youtube::connect_station(style, args[0].as_str().unwrap_or("default"))?
            } else if style {
                crate::catalog::entries(
                    &crate::catalog::spotify_style_mix(&selection.as_ref().unwrap().key())?,
                    "track",
                )
            } else {
                crate::catalog::entries(&invoke(name, &args)?, "track")
            };
            let tracks: Vec<_> = entries
                .into_iter()
                .map(|entry| {
                    let mut entry = entry;
                    if let Some(selection) = &selection {
                        entry.raw[crate::style_mix::MARKER] = json!(selection.key());
                        entry.raw["style_mix_title"] = json!(selection.title());
                    }
                    if !youtube {
                        entry.raw["spotify_station"] = json!({"name":name,"args":args});
                    }
                    attribute_entry(entry, &payload)
                })
                .collect();
            if method == "POST" {
                let first = tracks
                    .iter()
                    .find(|entry| payload["track_id"].as_str() == Some(entry.id.as_str()))
                    .or_else(|| tracks.first())
                    .ok_or("This mix returned no playable tracks")?;
                commands.try_send(Command::PlayEntry(Box::new(first.clone()), tracks))?;
                Ok(json!({"ok":true}))
            } else {
                Ok(
                    json!({"selection":selection.as_ref().map(|s|s.key()),"title":selection.as_ref().map(|s|s.title()),"tracks":tracks.iter().map(style_mix_track).collect::<Vec<_>>()}),
                )
            }
        }
        _ => Err("Unknown Spotify station action".into()),
    })())
}
// These calls use the host's saved Spotify session, never guest credentials.
fn spotify_guest_request(
    method: &str,
    path: &str,
    query: &str,
    host: &Snapshot,
) -> Option<Result<serde_json::Value, String>> {
    use serde_json::{Value, json};
    let invoke = |name: &str, args: &[Value]| crate::spotify::catalog_invoke(name, args);
    let result = match (method, path) {
        ("GET", "/api/search") => {
            let q = get_query_param(query, "q").unwrap_or_default();
            if q.trim().is_empty() { Ok(normalize_search(&json!({}))) }
            else { invoke("DZSearchJSON", &[json!(q)]).map(|value| {
                let mut result = normalize_search(&value);
                result["podcasts"] = json!(search_items(&value["shows"], "podcast"));
                result
            }) }
        }
        ("GET", "/api/discover") => (|| {
            let value = invoke("SPCatalogDiscoveryJSON", &[])?;
            let sections: Vec<_> = value["sections"].as_array().into_iter().flatten().map(|section| json!({
                "title":section["title"],"items":search_items(&section["items"],"playlist"),"error":section["error"]
            })).collect();
            Ok(json!({"sections":sections,"provider":"spotify"}))
        })(),
        ("GET", "/api/collection") => (|| {
            let kind = get_query_param(query, "kind").unwrap_or_default();
            let id = get_query_param(query, "id").unwrap_or_default();
            if !id.starts_with("spotify:") { return Err("Refresh the host catalog before opening this item".into()); }
            let name = match kind.as_str() {
                "album" => "DZAlbumTracksJSON", "playlist" => "DZPlaylistTracksJSON",
                "podcast" | "show" => "DZPodcastEpisodesJSON", "artist" => "DZArtistSectionJSON",
                _ => return Err("Unsupported Spotify collection".into()),
            };
            let value = invoke(name, &[json!(id), json!(0)])?;
            let tracks = search_items(&value, if kind == "artist" { "album" } else if kind == "show" || kind == "podcast" { "episode" } else { "track" });
            Ok(json!({"id":id,"kind":kind,"tracks":tracks,"provider":"spotify"}))
        })(),
        ("GET", "/api/lyrics") => {
            let ui = crate::ui::Ui::load_preferences();
            crate::lyrics::fetch_spotify_lyrics(&host.now, &host.now.id, ui.lyrics_sources).map(|(provider, lines)| {
                let lines: Vec<_> = lines.iter().map(|l| json!({"start_ms":l.start_ms,"end_ms":l.end_ms,"text":l.text,"translation":l.translation,"syllables":l.syllables.iter().map(|w|json!({"start_ms":w.start_ms,"end_ms":w.end_ms,"text":w.text})).collect::<Vec<_>>()})).collect();
                json!({"id":host.now.id,"duration_ms":host.duration,"provider":provider,"lines":lines,"karaoke":ui.karaoke_lyrics,"blur":ui.blur_inactive_lyrics})
            })
        }
        (_, p) if p.starts_with("/api/flow") || p.starts_with("/api/style-mix") => Err("Spotify does not expose this mix or tuner through its supported API. Use host search or your Spotify library.".into()),
        _ => return None,
    };
    Some(result)
}

fn youtube_guest_request(
    method: &str,
    path: &str,
    query: &str,
) -> Option<Result<serde_json::Value, String>> {
    let request = match (method, path) {
        ("GET", "/api/search") => (
            "search",
            get_query_param(query, "q").unwrap_or_default(),
            String::new(),
        ),
        ("GET", "/api/discover") => ("discover", String::new(), String::new()),
        ("GET", "/api/collection") => (
            "collection",
            get_query_param(query, "kind").unwrap_or_default(),
            get_query_param(query, "id").unwrap_or_default(),
        ),
        (_, "/api/lyrics") => {
            return Some(Err("Use the desktop player for YouTube Music lyrics".into()));
        }
        _ => return None,
    };
    Some(crate::youtube::connect_catalog(
        request.0, &request.1, &request.2,
    ))
}

fn connect_queue(snapshot: &Snapshot) -> Vec<serde_json::Value> {
    snapshot
        .queue
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            serde_json::json!({
                "index": idx, "id": e.id, "kind": e.kind, "title": e.title,
                "artist": e.subtitle, "artwork": e.artwork,
                "duration": e.duration / 1000, "duration_ms": e.duration,
                "added_by": e.raw["added_by"].as_str().unwrap_or(""),
            })
        })
        .collect()
}

fn send_json(stream: &mut TcpStream, json: &serde_json::Value) {
    let body = json.to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_command_result(stream: &mut TcpStream, result: Result<(), String>) {
    match result {
        Ok(()) => send_ok(stream),
        Err(error) => send_error(stream, &error),
    }
}

fn send_ok(stream: &mut TcpStream) {
    let body = "{\"ok\":true}";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_error(stream: &mut TcpStream, msg: &str) {
    let body = serde_json::json!({ "ok": false, "error": msg }).to_string();
    let resp = format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn stream_buffer_bytes(query: &str) -> usize {
    let milliseconds = get_query_param(query, "buffer_ms")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(500)
        .clamp(250, 2000);
    (44100 * milliseconds / 1000) * 4
}
fn get_query_param(query: &str, param: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == param).then(|| url_decode(v))
    })
}

fn fetch_discover(engine: Option<&Deezer>) -> serde_json::Value {
    let personal = [
        (3, "\u{00d6}nerilen \u{00c7}alma Listeleri / Recommended"),
        (
            1,
            "Sizin \u{0130}\u{00e7}in Se\u{00e7}ilenler / Made for you",
        ),
        (6, "Sanat\u{00e7}\u{0131}lar\u{0131}n\u{0131}zdan Listeler"),
    ];
    let public = [
        (
            "chart/0/playlists?limit=25",
            "En \u{00c7}ok Dinlenen \u{00c7}alma Listeleri",
            "playlist",
        ),
        (
            "editorial/0/releases?limit=25",
            "Yeni \u{00c7}\u{0131}kan Alb\u{00fc}mler",
            "album",
        ),
        (
            "editorial/0/selection?limit=25",
            "Edit\u{00f6}r\u{00fc}n Se\u{00e7}tikleri",
            "album",
        ),
        (
            "editorial?limit=30",
            "M\u{00fc}zik T\u{00fc}rleri ve Kategoriler",
            "channel",
        ),
    ];
    let personal_count = if engine.is_some() { personal.len() } else { 0 };
    let mut results = vec![None; personal_count + public.len()];
    let _ = crate::catalog::parallel::load(
        results.len(),
        |index| {
            let (title, items) = if index < personal_count {
                let (module, title) = personal[index];
                let value = engine?
                    .invoke("DZHomeModuleJSON", &[serde_json::json!(module)])
                    .ok()?;
                let items = crate::catalog::home_module_entries(&value,module,"playlist").into_iter()
                .map(|entry| serde_json::json!({"id":entry.id,"title":entry.title,"subtitle":entry.subtitle,
                    "artwork":entry.artwork,"kind":entry.kind})).collect::<Vec<_>>();
                (title, items)
            } else {
                let (endpoint, title, kind) = public[index - personal_count];
                let url = format!("https://api.deezer.com/{endpoint}");
                let value = crate::catalog::public_json(&url, || {
                    crate::lyrics::http::request("GET", &url, &[], &[])
                })
                .ok()?;
                let rows = value
                    .get("data")
                    .or_else(|| value.get("playlists"))?
                    .as_array()?;
                let items = rows.iter().filter_map(|item| {
                let id = item.get("id")?.to_string();
                let title = item.get("title").or_else(||item.get("name"))?.as_str()?;
                let subtitle = item.pointer("/user/name").or_else(||item.pointer("/artist/name"))
                    .and_then(serde_json::Value::as_str).unwrap_or("");
                let artwork = item.get("picture_medium").or_else(||item.get("cover_medium"))
                    .or_else(||item.get("picture_big")).or_else(||item.get("cover_big"))
                    .and_then(serde_json::Value::as_str).unwrap_or("");
                Some(serde_json::json!({"id":id,"title":title,"subtitle":subtitle,"artwork":artwork,"kind":kind}))
            }).collect::<Vec<_>>();
                (title, items)
            };
            (!items.is_empty()).then(|| serde_json::json!({"title":title,"items":items}))
        },
        |index, section| {
            results[index] = section;
            Ok(())
        },
    );
    serde_json::json!({"sections":results.into_iter().flatten().collect::<Vec<_>>()})
}

fn style_mix_track(entry: &Entry) -> serde_json::Value {
    serde_json::json!({"id":entry.id,"title":entry.title,"artist":entry.subtitle,
        "album":entry.album,"artwork":entry.artwork,"duration_ms":entry.duration,
        "duration":entry.duration/1000,"kind":entry.kind, "style_mix":entry.raw[crate::style_mix::MARKER],
        "spotify_station":entry.raw["spotify_station"], "youtube_station":entry.raw["youtube_station"]})
}

fn fetch_flow(engine: Option<&Deezer>, config_id: &str) -> serde_json::Value {
    let mut options = Vec::new();
    let mut tracks = Vec::new();
    let mut customizable = false;

    if let Some(dz) = engine {
        if let Ok(cfg) = dz.invoke("DZFlowConfigsJSON", &[]) {
            if let Some(arr) = cfg.get("options").and_then(serde_json::Value::as_array) {
                for item in arr.iter().filter(|v| v["kind"].as_str() != Some("genre")) {
                    let id = crate::catalog::string(item, &["id"]);
                    let title = crate::catalog::string(item, &["title"]);
                    if !id.is_empty() {
                        let icon = match id.as_str() {
                            "default" => "🌊",
                            "chill" => "☕",
                            "motivation" => "⚡",
                            "focus" => "🎯",
                            "party" => "🎉",
                            "melancholy" => "🌧️",
                            "workout" => "🔥",
                            _ => "🎵",
                        };
                        options.push(serde_json::json!({
                            "id": id,
                            "title": title,
                            "description": "",
                            "icon": icon,
                            "active": id == config_id || (config_id.is_empty() && id == "default"),
                        }));
                    }
                }
            }
        }
    }

    if options.is_empty() {
        let default_options = [
            (
                "default",
                "Kişisel Flow",
                "Tamamen size özel kesintisiz müzik",
                "🌊",
            ),
            ("chill", "Chill", "Rahatlatıcı, huzurlu parçalar", "☕"),
            (
                "motivation",
                "Motivasyon",
                "Enerjinizi yükseltecek şarkılar",
                "⚡",
            ),
            (
                "focus",
                "Odaklanma",
                "Çalışırken ve üretirken eşlikçi",
                "🎯",
            ),
            ("party", "Parti", "Eğlenceli ve hareketli ritimler", "🎉"),
            ("melancholy", "Hüzün", "Duygusal, içsel anlar için", "🌧️"),
            ("workout", "Antrenman", "Spor ve koşu temposu", "🔥"),
        ];

        for (id, title, desc, icon) in default_options {
            options.push(serde_json::json!({
                "id": id,
                "title": title,
                "description": desc,
                "icon": icon,
                "active": id == config_id || (config_id.is_empty() && id == "default"),
            }));
        }
    }

    if let Some(dz) = engine {
        let target_id = if config_id.is_empty() {
            "default"
        } else {
            config_id
        };
        if let Ok(tuner) = dz.invoke("DZFlowTunerJSON", &[serde_json::json!(target_id)]) {
            customizable = tuner["customizable"].as_bool().unwrap_or(false);
        } else if target_id == "default" {
            customizable = true;
        }

        let call = if target_id == "default" {
            dz.invoke("DZFlowJSON", &[])
        } else {
            dz.invoke("DZFlowForConfigJSON", &[serde_json::json!(target_id)])
        };
        if let Ok(val) = call {
            tracks = crate::catalog::entries(&val, "track")
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "title": e.title,
                        "artist": e.subtitle,
                        "artwork": e.artwork,
                        "duration": e.duration / 1000, "duration_ms": e.duration,
                    })
                })
                .collect();
        }
    } else {
        customizable = true;
    }

    serde_json::json!({
        "options": options,
        "tracks": tracks,
        "customizable": customizable,
    })
}

fn fetch_collection(engine: Option<&Deezer>, kind: &str, id: &str) -> serde_json::Value {
    let mut title = String::new();
    let mut artwork = String::new();
    let mut tracks = Vec::new();

    if kind == "podcast" {
        let result = engine
            .ok_or_else(|| "Connect Deezer on the host".to_string())
            .and_then(|engine| engine.invoke("DZPodcastEpisodesJSON", &[serde_json::json!(id)]));
        return match result {
            Ok(value) => {
                serde_json::json!({"id":id, "kind":kind, "tracks":search_items(&value, "episode")})
            }
            Err(error) => serde_json::json!({"error":error,"tracks":[]}),
        };
    }
    if kind == "artist" {
        if let Some(engine) = engine {
            match engine.invoke("DZArtistProfileJSON", &[serde_json::json!(id)]) {
                Ok(value) => {
                    let artist =
                        crate::catalog::entry(&value["artist"], "artist").unwrap_or_default();
                    return serde_json::json!({"id":id, "kind":kind, "title":artist.title, "artwork":artist.artwork,
                        "tracks":search_items(&value["top"], "track"), "albums":search_items(&value["albums"], "album"),
                        "artists":search_items(&value["related"], "artist")});
                }
                Err(error) => return serde_json::json!({"error":error, "tracks":[]}),
            }
        }
        return serde_json::json!({"error":"Connect Deezer on the host", "tracks":[]});
    }
    // Use the same native collection methods as the desktop.
    if let Some(dz) = engine {
        let attempts: &[&str] = match kind {
            "mix" => &["DZSmartMixJSON", "DZPlaylistTracksJSON"],
            "album" => &["DZAlbumTracksJSON"],
            _ => &[
                "DZPlaylistTracksJSON",
                "DZSmartMixJSON",
                "DZAlbumTracksJSON",
            ],
        };

        for &method in attempts {
            if let Ok(val) = dz.invoke(method, &[serde_json::json!(id)]) {
                let entries = crate::catalog::table_entries(&val, "track");
                if !entries.is_empty() {
                    title = val
                        .pointer("/data/smartTracklist/title")
                        .or_else(|| val.pointer("/data/smartTracklist/displayTitle"))
                        .or_else(|| val.pointer("/smartTracklist/title"))
                        .or_else(|| val.pointer("/smartTracklist/displayTitle"))
                        .or_else(|| val.pointer("/data/playlist/title"))
                        .or_else(|| val.pointer("/playlist/title"))
                        .or_else(|| val.pointer("/data/album/title"))
                        .or_else(|| val.pointer("/album/title"))
                        .or_else(|| val.get("title"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    artwork = val
                        .pointer("/data/smartTracklist/pictures/medium")
                        .or_else(|| val.pointer("/smartTracklist/pictures/medium"))
                        .or_else(|| val.pointer("/data/playlist/pictures/medium"))
                        .or_else(|| val.pointer("/playlist/pictures/medium"))
                        .or_else(|| val.pointer("/data/album/pictures/medium"))
                        .or_else(|| val.pointer("/album/pictures/medium"))
                        .or_else(|| val.pointer("/picture_medium"))
                        .or_else(|| val.pointer("/cover_medium"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    if artwork.is_empty() {
                        if let Some(first) = entries.first() {
                            artwork = first.artwork.clone();
                        }
                    }

                    tracks = entries
                        .into_iter()
                        .map(|e| {
                            serde_json::json!({
                                "id": e.id,
                                "kind": e.kind,
                                "title": e.title,
                                "artist": e.subtitle,
                                "artwork": e.artwork,
                                "duration": e.duration / 1000, "duration_ms": e.duration,
                            })
                        })
                        .collect();
                    break;
                }
            }
        }
    }

    // 2. Fall back to public Deezer API (for editorial playlists, charts, etc.)
    if tracks.is_empty() {
        let endpoint = if kind == "album" {
            format!("https://api.deezer.com/album/{id}")
        } else {
            format!("https://api.deezer.com/playlist/{id}")
        };

        if let Ok(bytes) = crate::lyrics::http::request("GET", &endpoint, &[], &[]) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if title.is_empty() {
                    title = value
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }
                if artwork.is_empty() {
                    artwork = value
                        .pointer("/cover_big")
                        .or_else(|| value.pointer("/picture_big"))
                        .or_else(|| value.pointer("/cover_medium"))
                        .or_else(|| value.pointer("/picture_medium"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }

                let track_array = value
                    .pointer("/tracks/data")
                    .or_else(|| value.get("data"))
                    .and_then(serde_json::Value::as_array);

                if let Some(arr) = track_array {
                    tracks = arr
                        .iter()
                        .filter_map(|item| {
                            let id = item.get("id")?.to_string();
                            let t_title = item.get("title")?.as_str()?.to_string();
                            let artist = item
                                .pointer("/artist/name")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let t_artwork = item
                                .pointer("/album/cover_medium")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or(&artwork)
                                .to_string();
                            let duration = item
                                .get("duration")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0);
                            Some(serde_json::json!({
                                "id": id,
                                "title": t_title,
                                "artist": artist,
                                "artwork": t_artwork,
                                "duration": duration, "duration_ms": duration.saturating_mul(1000),
                            }))
                        })
                        .collect();
                }
            }
        }
    }

    serde_json::json!({
        "id": id,
        "kind": kind,
        "title": title,
        "artwork": artwork,
        "tracks": tracks,
    })
}

fn browser_hex(color: u32) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        color & 255,
        (color >> 8) & 255,
        (color >> 16) & 255
    )
}

fn browser_palette(ui: &crate::ui::Ui) -> serde_json::Value {
    let keys = ["Background", "Panel", "Raised", "Border", "Text", "Muted"];
    let protected =
        crate::appearance::readable_palette(keys.map(|key| ui.color(key, 0)), ui.light, 100);
    let mut palette = serde_json::json!({});
    for key in [
        "Background",
        "Panel",
        "Raised",
        "Border",
        "Text",
        "Muted",
        "Accent",
        "AccentText",
        "GradientStart",
        "GradientEnd",
    ] {
        palette[key] = serde_json::json!(browser_hex(ui.color(key, 0)));
    }
    palette["Text"] = serde_json::json!(browser_hex(protected[4]));
    palette["Muted"] = serde_json::json!(browser_hex(protected[5]));
    palette
}

fn browser_appearance() -> serde_json::Value {
    let themes: Vec<_> = crate::ui::SOLID_THEMES.iter().enumerate().map(|(index, theme)| {
        let mut ui = crate::ui::Ui::default(); ui.apply_solid_theme(index);
        serde_json::json!({"id":index,"name":theme.0,"light":theme.1,"palette":browser_palette(&ui)})
    }).collect();
    serde_json::json!({"themes":themes,"defaults":{"theme":8,"blur":20,"opacity":40}})
}

fn flow_selection_is_empty(value: &serde_json::Value) -> bool {
    // Edits are a delta, so an all-disabled delta can still leave an unchanged
    // genre enabled. Only a complete selection can establish that none remain.
    value
        .get("genres")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|genres| {
            !genres
                .iter()
                .any(|genre| genre["isEnabled"].as_bool().unwrap_or(false))
        })
}

fn search_items(value: &serde_json::Value, kind: &str) -> Vec<serde_json::Value> {
    crate::catalog::entries(value, kind).into_iter().map(|e| serde_json::json!({
        "id": e.id, "kind": e.kind, "title": e.title, "artist": e.subtitle,
        "subtitle": e.subtitle, "album": e.album, "artwork": e.artwork, "duration": e.duration / 1000, "duration_ms": e.duration,
    })).collect()
}

fn normalize_search(value: &serde_json::Value) -> serde_json::Value {
    let mut result = serde_json::json!({});
    for (key, kind) in [
        ("tracks", "track"),
        ("albums", "album"),
        ("playlists", "playlist"),
        ("artists", "artist"),
        ("podcasts", "podcast"),
    ] {
        result[key] = serde_json::json!(search_items(&value[key], kind));
    }
    result
}

fn search_deezer(engine: Option<&Deezer>, query: &str) -> serde_json::Value {
    let mut empty = normalize_search(&serde_json::json!({}));
    if query.trim().is_empty() {
        return empty;
    }
    let Some(engine) = engine else {
        empty["error"] = serde_json::json!("Connect Deezer on the host to search");
        return empty;
    };
    match engine.invoke("DZSearchJSON", &[serde_json::json!(query)]) {
        Ok(value) => {
            let mut result = normalize_search(&value);
            if result["podcasts"].as_array().is_some_and(|p| p.is_empty()) {
                if let Ok(podcasts) =
                    engine.invoke("DZSearchPodcastsJSON", &[serde_json::json!(query)])
                {
                    result["podcasts"] = serde_json::json!(search_items(&podcasts, "podcast"));
                }
            }
            result
        }
        Err(error) => {
            empty["error"] = serde_json::json!(error);
            empty
        }
    }
}

fn url_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut iter = input.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hex1 = iter.next();
            let hex2 = iter.next();
            if let (Some(h1), Some(h2)) = (hex1, hex2) {
                if let Ok(val) = u8::from_str_radix(&format!("{}{}", h1 as char, h2 as char), 16) {
                    bytes.push(val);
                    continue;
                }
            }
        } else if b == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejected_commands_do_not_report_success() {
        for disconnected in [false, true] {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            tx.send(Command::Next).unwrap();
            let _receiver = if disconnected { None } else { Some(rx) };
            let hub = ConnectHub::start(0, tx).unwrap();
            let mut stream = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.write_all(b"POST /api/queue/clear HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
            assert!(!response.contains("\"ok\":true"));
        }
    }

    #[test]
    fn fragmented_headers_and_body_are_read_until_complete() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let hub = ConnectHub::start(0, tx).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let body = r#"{"id":"fragmented","nickname":"Alice"}"#;
        write!(
            stream,
            "POST /api/presence HTTP/1.1\r\nHost: localhost\r\nContent-Len"
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(80));
        write!(stream, "gth: {}\r\n\r\n{}", body.len(), &body[..10]).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        stream.write_all(body[10..].as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert_eq!(hub.nicknames(), ["Alice"]);
    }

    #[test]
    fn presence_tracks_and_expires_browser_sessions() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let hub = ConnectHub::start(0, tx).unwrap();
        fn post(hub: &ConnectHub, body: &str) {
            let mut stream = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            write!(
                stream,
                "POST /api/presence HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"));
        }
        post(&hub, r#"{"id":"one","nickname":"Alice"}"#);
        post(&hub, r#"{"id":"two","nickname":"Bob"}"#);
        assert_eq!(hub.nicknames(), ["Alice", "Bob"]);
        post(&hub, r#"{"id":"one","nickname":""}"#);
        assert_eq!(hub.nicknames(), ["Bob"]);
        hub.state
            .write()
            .unwrap()
            .presence
            .get_mut("two")
            .unwrap()
            .1 = Instant::now() - Duration::from_secs(46);
        assert!(hub.nicknames().is_empty());
    }

    #[test]
    fn audio_delivery_continues_when_spectrum_worker_is_backlogged() {
        let state = RwLock::new(HubState {
            presence: Default::default(),
            queue_revision: 0,
            spotify_audio_error: None,
            spectrum: spectrum::Snapshot::default(),
            snapshot: Snapshot::default(),
            engine: None,
        });
        let active = AtomicBool::new(true);
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let listener = broadcaster().subscribe();
        publish_pcm(&state, &active, &tx, 0, &[1, 2, 3, 4], Provider::Deezer);
        publish_pcm(&state, &active, &tx, 0, &[5, 6, 7, 8], Provider::Deezer);
        assert_eq!(listener.try_recv().unwrap().as_slice(), &[1, 2, 3, 4]);
        assert_eq!(listener.try_recv().unwrap().as_slice(), &[5, 6, 7, 8]);
    }

    #[test]
    fn pcm_subscriptions_release_idle_capture_demand_and_disconnect_slow_clients() {
        use super::*;
        let broadcaster = AudioBroadcaster {
            clients: Mutex::new(Vec::new()),
        };
        let listener = broadcaster.subscribe();
        assert!(broadcaster.has_listeners());
        drop(listener);
        assert!(!broadcaster.has_listeners());
        let listener = broadcaster.subscribe();
        for sample in 0..128u8 {
            broadcaster.publish(Arc::new(vec![sample; MAX_QUEUED_PCM / 128]));
        }
        broadcaster.publish(Arc::new(vec![255; MAX_QUEUED_PCM]));
        assert!(!broadcaster.has_listeners());
        for sample in 0..128u8 {
            assert_eq!(
                &**listener.try_recv().unwrap(),
                &vec![sample; MAX_QUEUED_PCM / 128]
            );
        }
        assert!(matches!(
            listener.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }
    #[test]
    fn small_pcm_packets_fill_mobile_prebuffer_and_reuse_budget() {
        let hub = AudioBroadcaster {
            clients: Mutex::new(Vec::new()),
        };
        let listener = hub.subscribe();
        for _ in 0..2 {
            for _ in 0..1500 {
                hub.publish(Arc::new(vec![7; 176]));
            }
            assert!(hub.has_listeners());
            for _ in 0..1500 {
                assert_eq!(listener.try_recv().unwrap().len(), 176);
            }
            assert_eq!(listener.queued.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn host_payloads_reject_cross_provider_ids_and_stale_collections() {
        use super::*;
        use serde_json::json;
        assert!(valid_host_payload(
            Provider::Spotify,
            &json!({"provider":"spotify","id":"spotify:track:abc"})
        ));
        assert!(!valid_host_payload(
            Provider::Spotify,
            &json!({"provider":"spotify","id":"123"})
        ));
        assert!(valid_host_payload(
            Provider::Spotify,
            &json!({"id":"spotify:track:abc"})
        ));
        assert!(!valid_host_payload(
            Provider::Deezer,
            &json!({"provider":"spotify"})
        ));
        assert!(!valid_host_payload(
            Provider::Deezer,
            &json!({"id":"spotify:track:abc"})
        ));
        assert!(!valid_host_payload(
            Provider::Spotify,
            &json!({"provider":"spotify","tracks":[{"id":"spotify:track:abc"},{"id":"123"}]})
        ));
        assert!(valid_host_payload(
            Provider::Deezer,
            &json!({"tracks":[{"id":"123"}]})
        ));
    }
    #[test]
    fn spotify_guest_routes_never_enter_deezer_fallbacks() {
        use super::*;
        let host = Snapshot {
            provider: Provider::Spotify,
            ..Default::default()
        };
        assert!(spotify_guest_request("GET", "/api/stream", "", &host).is_none());
        for (method, route, query) in [
            ("GET", "/api/flow", ""),
            ("POST", "/api/flow/start", ""),
            ("GET", "/api/style-mix", ""),
            ("GET", "/api/collection", "id=123&kind=album"),
        ] {
            assert!(
                spotify_guest_request(method, route, query, &host)
                    .unwrap()
                    .is_err()
            );
        }
        let empty = spotify_guest_request("GET", "/api/search", "q=", &host)
            .unwrap()
            .unwrap();
        assert!(empty["tracks"].as_array().unwrap().is_empty());
    }
    #[test]
    fn youtube_catalog_routes_and_payloads_stay_on_host_provider() {
        use super::*;
        use serde_json::json;
        let empty = youtube_guest_request("GET", "/api/search", "q=")
            .unwrap()
            .unwrap();
        assert_eq!(empty["provider"], "youtube");
        assert!(empty["tracks"].as_array().unwrap().is_empty());
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx.clone()).unwrap();
        hub.update(&Snapshot {
            provider: Provider::YouTube,
            ..Default::default()
        });
        let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            client,
            "GET /api/search?q=&host_provider=youtube HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            empty
        );
        for query in [
            "kind=album&id=123",
            "kind=playlist&id=spotify:playlist:abc",
            "kind=album&id=youtube:playlist:abc",
        ] {
            assert!(
                youtube_guest_request("GET", "/api/collection", query)
                    .unwrap()
                    .is_err()
            );
        }
        assert!(youtube_guest_request("POST", "/api/play", "").is_none());
        assert!(youtube_guest_request("GET", "/api/flow", "").is_none());
        let host = Snapshot {
            provider: Provider::YouTube,
            ..Default::default()
        };
        let commands = HostCommands {
            tx: &tx,
            state: &hub.state,
            provider: Provider::YouTube,
        };
        let options = spotify_station_request("GET", "/api/style-mix", "", "", &host, &commands)
            .unwrap()
            .unwrap();
        assert_eq!(options, crate::style_mix::options());
        assert!(
            spotify_station_request(
                "GET",
                "/api/flow/tuner",
                "id=invalid-mood",
                "",
                &host,
                &commands
            )
            .unwrap()
            .is_err()
        );
        let saved = spotify_station_request(
            "POST",
            "/api/flow/tuner/save",
            "",
            &json!({"flow_id":crate::spotify::flow::Selection { mood:"chill".into(), genres:vec!["Jazz".into()] }.key(),"genres":[{"id":"Jazz","isEnabled":true}]}).to_string(),
            &host,
            &commands,
        )
        .unwrap()
        .unwrap();
        let restored =
            crate::spotify::flow::Selection::parse(saved["id"].as_str().unwrap()).unwrap();
        assert!(restored.genres.iter().any(|genre| genre == "Jazz"));
        assert!(
            spotify_station_request(
                "GET",
                "/api/style-mix",
                "selection=invalid",
                "",
                &host,
                &commands
            )
            .unwrap()
            .is_err()
        );
        assert!(valid_host_payload(
            Provider::YouTube,
            &json!({"provider":"youtube","tracks":[{"id":"youtube:track:abc"}]})
        ));
        for provider in [Provider::Deezer, Provider::Spotify] {
            assert!(!valid_host_payload(
                provider,
                &json!({"id":"youtube:track:abc"})
            ));
        }
        for payload in [
            json!({"id":"123"}),
            json!({"id":"spotify:track:abc"}),
            json!({"tracks":[{"provider":"deezer","id":"youtube:track:abc"}]}),
        ] {
            assert!(!valid_host_payload(Provider::YouTube, &payload));
        }
    }
    #[test]
    fn queue_attribution_is_bounded_and_removes_control_characters() {
        let entry = super::attribute_entry(
            Default::default(),
            &serde_json::json!({"added_by":" Alice\n "}),
        );
        assert_eq!(entry.raw["added_by"], "Alice");
        let entry = super::attribute_entry(
            Default::default(),
            &serde_json::json!({"added_by":"x".repeat(100)}),
        );
        assert_eq!(entry.raw["added_by"].as_str().unwrap().len(), 64);
    }
    #[test]
    fn style_mix_artwork_http_works_without_provider_session() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        for path in [
            "/style-mix/region/0.webp",
            "/style-mix/genre/0.webp",
            "/style-mix/region/99999.webp",
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(client, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).unwrap();
            let split = response
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap();
            let header = std::str::from_utf8(&response[..split]).unwrap();
            let body = &response[split + 4..];
            match crate::style_mix::artwork::http_data(path) {
                Some(expected) => {
                    assert!(header.starts_with("HTTP/1.1 200"));
                    let mime = if expected.starts_with(&[0xff, 0xd8, 0xff]) {
                        "image/jpeg"
                    } else {
                        "image/webp"
                    };
                    assert!(header.contains(&format!("Content-Type: {mime}")));
                    assert_eq!(body, expected);
                }
                None => assert!(header.starts_with("HTTP/1.1 400")),
            }
        }
    }

    #[test]
    fn style_mix_options_and_errors_are_separate_from_flow() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        for (path, expected) in [
            ("/api/style-mix", "Japanese"),
            ("/api/style-mix/genres?region=1", "genres"),
            (
                "/api/style-mix/genres?region=invalid",
                "Choose a valid language",
            ),
            (
                "/api/style-mix?selection=0%3A0",
                "Connect Deezer on the host first",
            ),
            (
                "/api/style-mix?selection=999%3A0",
                "Choose a valid country and genre",
            ),
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.contains(expected), "{response}");
        }
        let entry = attribute_entry(
            Entry {
                raw: serde_json::json!({}),
                ..Default::default()
            },
            &serde_json::json!({"style_mix":"1:1", "added_by":"Guest"}),
        );
        assert_eq!(entry.raw[crate::style_mix::MARKER], "1:1");
        assert_eq!(entry.raw["added_by"], "Guest");

        // Invalid selections must never expose the unverified global genre list.
        for provider in [Provider::Deezer, Provider::Spotify, Provider::YouTube] {
            hub.update(&Snapshot {
                provider,
                ..Default::default()
            });
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(
                    b"GET /api/style-mix/genres?region=99999 HTTP/1.1\r\nHost: localhost\r\n\r\n",
                )
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with(if provider == Provider::Deezer {
                    "HTTP/1.1 200"
                } else {
                    "HTTP/1.1 400"
                }),
                "Provider {:?} failed: {}",
                provider,
                response
            );
            let body = response.split_once("\r\n\r\n").unwrap().1;
            let data: serde_json::Value = serde_json::from_str(body).unwrap();
            assert!(data["error"].as_str().is_some(), "{provider:?}: {data}");
            assert!(
                data["genres"].as_array().is_none_or(Vec::is_empty),
                "{provider:?}: {data}"
            );
        }
    }

    #[test]
    fn lightweight_state_polls_keep_large_queues_out_of_playback_updates() {
        fn get(hub: &ConnectHub, path: &str) -> (String, serde_json::Value) {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(client, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            let (head, body) = response.split_once("\r\n\r\n").unwrap();
            assert!(head.starts_with("HTTP/1.1 200"));
            assert!(head.contains("Cache-Control: no-store"));
            (body.into(), serde_json::from_str(body).unwrap())
        }
        for provider in [Provider::Deezer, Provider::Spotify] {
            let (tx, _rx) = std::sync::mpsc::sync_channel(4);
            let hub = ConnectHub::start(0, tx).unwrap();
            let mut snapshot = Snapshot {
                provider,
                connected: true,
                queue: Arc::new(
                    (0..1568)
                        .map(|i| Entry {
                            id: i.to_string(),
                            title: format!("Track {i}"),
                            artwork: "https://example.invalid/cover.jpg".into(),
                            ..Entry::default()
                        })
                        .collect(),
                ),
                ..Snapshot::default()
            };
            hub.update(&snapshot);
            let (body, first) = get(&hub, "/api/state?queue=0");
            assert!(body.len() < 2048, "playback poll grows with queue size");
            assert!(first.get("queue").is_none());
            let revision = first["queue_revision"].clone();
            snapshot.position = 2000;
            snapshot.queue_index = 3;
            hub.update(&snapshot);
            let (_, next) = get(&hub, "/api/state?queue=0");
            assert_eq!(next["queue_revision"], revision);
            assert_eq!(next["queue_index"], 3);
            let (_, queue) = get(&hub, "/api/queue");
            assert_eq!(queue["queue_revision"], revision);
            assert_eq!(queue["queue"].as_array().unwrap().len(), 1568);
            let (_, legacy) = get(&hub, "/api/state");
            assert_eq!(legacy["queue"], queue["queue"]);
            Arc::make_mut(&mut snapshot.queue)[0].title = "Edited".into();
            hub.update(&snapshot);
            let (_, changed) = get(&hub, "/api/state?queue=0");
            assert_ne!(changed["queue_revision"], revision);
        }
    }

    #[test]
    fn phone_stream_buffer_is_bounded_and_native_default_is_unchanged() {
        assert_eq!(stream_buffer_bytes(""), 88200);
        assert_eq!(stream_buffer_bytes("buffer_ms=1200"), 211680);
        assert_eq!(stream_buffer_bytes("buffer_ms=1500"), 264600);
        assert_eq!(stream_buffer_bytes("buffer_ms=999999"), 352800);
        assert_eq!(stream_buffer_bytes("buffer_ms=1"), 44100);
        assert_eq!(stream_buffer_bytes("buffer_ms=invalid"), 88200);
    }
    use super::*;
    #[test]
    fn named_queue_additions_are_shared_with_other_clients() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        let mut queued = Vec::new();
        for nickname in ["Alice", "Bob"] {
            let body =
                serde_json::json!({"id":"1","title":"Same song","added_by":nickname}).to_string();
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(
                    format!(
                        "POST /api/queue/add-last HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"));
            match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                Command::QueueAddLast(entry) => queued.push(*entry),
                _ => panic!("Expected a queue addition"),
            }
        }
        hub.update(&Snapshot {
            queue: Arc::new(queued),
            ..Snapshot::default()
        });
        let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .write_all(b"GET /api/state HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        let state: serde_json::Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(state["queue"][0]["added_by"], "Alice");
        assert_eq!(state["queue"][1]["added_by"], "Bob");
    }

    #[test]
    fn browser_shuffle_and_repeat_dispatch_explicit_modes() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        hub.update(&Snapshot {
            connected: true,
            ..Snapshot::default()
        });
        for (path, body, ok) in [
            ("shuffle", r#"{"enabled":true}"#, true),
            ("repeat", r#"{"mode":2}"#, true),
            ("repeat", r#"{"mode":3}"#, false),
            ("autoplay", r#"{"enabled":true}"#, true),
            ("autoplay", r#"{"enabled":false}"#, true),
            ("autoplay", r#"{"enabled":"true"}"#, false),
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(
                    format!(
                        "POST /api/{path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert_eq!(response.starts_with("HTTP/1.1 200"), ok);
        }
        assert!(matches!(rx.try_recv().unwrap(), Command::Shuffle(true)));
        assert!(matches!(rx.try_recv().unwrap(), Command::Repeat(2)));
        assert!(matches!(rx.try_recv().unwrap(), Command::Autoplay(true)));
        assert!(matches!(rx.try_recv().unwrap(), Command::Autoplay(false)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn browser_assets_and_appearance_are_served_by_the_jam_host() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        for (path, mime, expected) in [
            ("/ui.js", "text/javascript", "JamUI"),
            ("/ui.css", "text/css", "DeezNatsIcons"),
            ("/translations.json", "application/json", "Appearance"),
            ("/api/appearance", "application/json", "Blood Moon"),
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{path}");
            assert!(response.contains(mime), "{path}");
            assert!(response.contains(expected), "{path}");
        }
    }

    #[test]
    fn browser_appearance_exposes_only_themes_opacity_and_blur() {
        let appearance = browser_appearance();
        assert_eq!(
            appearance["themes"].as_array().unwrap().len(),
            crate::ui::SOLID_THEMES.len()
        );
        assert_eq!(appearance["defaults"].as_object().unwrap().len(), 3);
        assert_eq!(
            appearance["defaults"],
            serde_json::json!({"theme":8,"opacity":40,"blur":20})
        );
        assert!(appearance.get("palettes").is_none());
        assert_eq!(appearance["themes"][11]["name"], "Blood Moon");
        assert_eq!(
            appearance["themes"][11]["palette"]["Background"],
            browser_hex(crate::ui::SOLID_THEMES[11].2)
        );
        assert_eq!(browser_hex(0x00332211), "#112233");
        let translations: serde_json::Value =
            serde_json::from_str(include_str!("translations.json")).unwrap();
        assert!(translations["Appearance"].is_string());
        assert!(translations["Loading lyrics..."].is_string());
    }

    #[test]
    fn flow_accepts_one_unchanged_genre_when_other_genres_are_disabled() {
        let selection = serde_json::json!({"genres":[{"id":"rock","isEnabled":true},{"id":"pop","isEnabled":false}],
            "edits":[{"clusterId":"pop","isEnabled":false}]});
        assert!(!flow_selection_is_empty(&selection));
        assert!(flow_selection_is_empty(
            &serde_json::json!({"genres":[{"id":"rock","isEnabled":false}]})
        ));
        assert!(!flow_selection_is_empty(
            &serde_json::json!({"edits":[{"clusterId":"pop","isEnabled":false}]})
        ));
    }

    #[test]
    fn jam_durations_preserve_milliseconds_through_search_and_queue() {
        let results = search_items(
            &serde_json::json!([{"id":"1","title":"Levels","durationMs":199000,"kind":"track"}]),
            "track",
        );
        assert_eq!(results[0]["duration_ms"], 199000);
        assert_eq!(results[0]["duration"], 199);
        let queued = ConnectTrack::from_value(&results[0]).unwrap();
        assert_eq!(queued.to_entry().duration, 199000);
        assert_eq!(queued.to_json()["duration_ms"], 199000);
        let legacy = ConnectTrack::from_value(
            &serde_json::json!({"id":"1","title":"Levels","duration":199}),
        )
        .unwrap();
        assert_eq!(legacy.to_entry().duration, 199000);
    }

    #[test]
    fn search_preserves_categories_and_requires_host_session() {
        let raw = serde_json::json!({
            "tracks":[{"id":"1","title":"Song","kind":"track"}],
            "albums":[{"id":"2","title":"Album","kind":"album"}],
            "artists":[{"id":"3","name":"Artist","kind":"artist"}],
            "playlists":[{"id":"4","title":"Playlist","kind":"playlist"}]
        });
        let results = normalize_search(&raw);
        for key in ["tracks", "albums", "artists", "playlists"] {
            assert_eq!(results[key].as_array().unwrap().len(), 1, "{key}");
        }
        assert_eq!(results["albums"][0]["kind"], "album");
        assert!(search_deezer(None, "Song")["error"].is_string());
        assert_eq!(
            get_query_param("q=AC%2BDC%20%2525", "q").unwrap(),
            "AC+DC %25"
        );
        let episode = ConnectTrack::from_value(
            &serde_json::json!({"id":"8","title":"Episode","kind":"episode"}),
        )
        .unwrap();
        assert_eq!(episode.to_entry().kind, "episode");
    }

    #[test]
    fn state_api_normalizes_engine_playback_state() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        let snapshot = Snapshot {
            state: "Playing".into(),
            ..Snapshot::default()
        };
        hub.update(&snapshot);
        let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .write_all(b"GET /api/state HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1;
        let state: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(state["state"], "playing");
    }

    #[test]
    fn fragmented_request_and_shutdown() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        let port = hub.port();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let body = br#"{"position_ms":42}"#;
        let request = format!(
            "POST /api/seek HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        for byte in request.bytes().chain(body.iter().copied()) {
            client.write_all(&[byte]).unwrap();
        }
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Command::SeekMs(42)
        ));
        drop(hub);
        let rebound = TcpListener::bind(("127.0.0.1", port)).unwrap();
        drop(rebound);
    }

    #[test]
    fn test_fetch_flow_structure() {
        let flow = fetch_flow(None, "default");
        assert!(
            flow.get("options")
                .and_then(serde_json::Value::as_array)
                .is_some()
        );
        assert!(
            flow.get("customizable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_flow_tuner_endpoints() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(32);
        let hub = ConnectHub::start(3195, tx).expect("failed to start test hub");
        let port = hub.port();

        // Test GET /api/flow/tuner
        let url = format!("http://127.0.0.1:{port}/api/flow/tuner?id=default");
        if let Ok(bytes) = crate::lyrics::http::request("GET", &url, &[], &[]) {
            let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(val["id"], "default");
            assert!(val["genres"].as_array().unwrap().len() > 0);
        }

        // Test POST /api/flow/tuner/save empty validation check
        let save_url = format!("http://127.0.0.1:{port}/api/flow/tuner/save");
        let empty_payload = serde_json::json!({
            "flow_id": "default",
            "genres": [
                { "id": "132", "isEnabled": false }
            ]
        });
        let body = serde_json::to_vec(&empty_payload).unwrap();
        if let Ok(bytes) = crate::lyrics::http::request("POST", &save_url, &[], &body) {
            let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(val.get("error").is_some());
        }

        // Test POST /api/flow/tuner/save valid save
        let valid_payload = serde_json::json!({
            "flow_id": "default",
            "genres": [
                { "id": "132", "isEnabled": true }
            ]
        });
        let body = serde_json::to_vec(&valid_payload).unwrap();
        if let Ok(bytes) = crate::lyrics::http::request("POST", &save_url, &[], &body) {
            let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(val["ok"], false);
            assert!(val["error"].as_str().unwrap().contains("Connect Deezer"));
        }
    }
}
