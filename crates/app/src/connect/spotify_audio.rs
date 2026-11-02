//! Process-isolated Soloist capture. No endpoint enumeration or system-loopback fallback.
//! Matches Microsoft's ApplicationLoopback sample: WASAPI converts/resamples into
//! the existing Connect wire format (44,100 Hz, stereo, signed 16-bit little endian).
use crate::spotify::service::AudioTarget;
use std::{
    mem::ManuallyDrop,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use windows::{
    core::{implement, Interface, Ref, HRESULT},
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
        Media::Audio::*,
        System::{
            Com::{
                CoInitializeEx, CoUninitialize,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
                BLOB, COINIT_MULTITHREADED,
            },
            Threading::{CreateEventW, WaitForSingleObject},
            Variant::VT_BLOB,
        },
    },
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
    format!("Spotify process-isolated audio unavailable ({stage}: {error}). Requires Windows application-loopback support (build 20348 or newer). No system audio fallback is used.")
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
                            pBlobData: (&mut *params as *mut AUDIOCLIENT_ACTIVATION_PARAMS).cast(),
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
            return Err("Spotify audio unavailable: the supervised Soloist process stopped".into());
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
                Err(mpsc::TryRecvError::Empty) if started.elapsed() < Duration::from_secs(5) => {
                    return Ok(())
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
                (AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 | AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0)
                    as u32,
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
            assert!(append_packet(&mut output, std::ptr::null(), u32::MAX, 2, u32::MAX).is_err());
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
        assert!(handler
            .cast::<windows::Win32::System::Com::IAgileObject>()
            .is_ok());
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
            use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).unwrap();
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    1_000_000,
                    0,
                    &wire_format(),
                    None,
                )
                .unwrap();
            let frames = client.GetBufferSize().unwrap();
            let renderer: IAudioRenderClient = client.GetService().unwrap();
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
