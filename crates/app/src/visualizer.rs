use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows_sys::Win32::System::Com::*;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xBCDE0395,
    data2: 0xE52F,
    data3: 0x467C,
    data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
};
const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xA95664D2,
    data2: 0x9614,
    data3: 0x4F35,
    data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
};
const IID_IAUDIO_CLIENT: Guid = Guid {
    data1: 0x1CB9AD4C,
    data2: 0xDBFA,
    data3: 0x4C32,
    data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
};
const IID_IAUDIO_CAPTURE_CLIENT: Guid = Guid {
    data1: 0xC8ADBD64,
    data2: 0xE71E,
    data3: 0x48A0,
    data4: [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17],
};

const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x00020000;
const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;

#[repr(C)]
struct IMMDeviceEnumeratorVtbl {
    query_interface: usize,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    enum_audio_endpoints: usize,
    get_default_audio_endpoint:
        unsafe extern "system" fn(*mut c_void, i32, i32, *mut *mut c_void) -> i32,
}

#[repr(C)]
struct IMMDeviceVtbl {
    query_interface: usize,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    activate: unsafe extern "system" fn(
        *mut c_void,
        *const Guid,
        u32,
        *const c_void,
        *mut *mut c_void,
    ) -> i32,
}

#[repr(C)]
struct IAudioClientVtbl {
    query_interface: usize,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    initialize: unsafe extern "system" fn(
        *mut c_void,
        i32,
        u32,
        i64,
        i64,
        *const c_void,
        *const Guid,
    ) -> i32,
    get_buffer_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    get_stream_latency: usize,
    get_current_padding: usize,
    is_format_supported: usize,
    get_mix_format: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_device_period: usize,
    start: unsafe extern "system" fn(*mut c_void) -> i32,
    stop: unsafe extern "system" fn(*mut c_void) -> i32,
    reset: usize,
    set_event_handle: usize,
    get_service: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
}

#[repr(C)]
struct IAudioCaptureClientVtbl {
    query_interface: usize,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_buffer: unsafe extern "system" fn(
        *mut c_void,
        *mut *mut u8,
        *mut u32,
        *mut u32,
        *mut u64,
        *mut u64,
    ) -> i32,
    release_buffer: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    get_next_packet_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WaveFormatEx {
    w_format_tag: u16,
    n_channels: u16,
    n_samples_per_sec: u32,
    n_avg_bytes_per_sec: u32,
    n_block_align: u16,
    w_bits_per_sample: u16,
    cb_size: u16,
}

pub struct Visualizer {
    active: Arc<AtomicBool>,
    output_volume: Arc<AtomicU32>,
    render_levels: Mutex<([f32; 32], Instant)>,
    levels: Arc<Mutex<[f32; 32]>>,
    scene_levels: Arc<Mutex<[f32; 32]>>,
    trap_state: Mutex<TrapResponse>,
    has_live_signal: Arc<Mutex<bool>>,
    last_update: Arc<Mutex<Instant>>,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visualizer {
    pub fn new() -> Self {
        let active = Arc::new(AtomicBool::new(false));
        let active_clone = active.clone();
        let output_volume = Arc::new(AtomicU32::new(1f32.to_bits()));
        let volume_clone = output_volume.clone();
        let levels = Arc::new(Mutex::new([0.0f32; 32]));
        let has_live_signal = Arc::new(Mutex::new(false));
        let last_update = Arc::new(Mutex::new(Instant::now()));

        let scene_levels = Arc::new(Mutex::new([0.; 32]));
        let scene_clone = scene_levels.clone();
        let levels_clone = Arc::clone(&levels);
        let signal_clone = Arc::clone(&has_live_signal);
        let update_clone = Arc::clone(&last_update);

        std::thread::Builder::new()
            .name("deeznats-wasapi-visualizer".into())
            .spawn(move || unsafe {
                run_wasapi_capture(
                    levels_clone,
                    signal_clone,
                    update_clone,
                    active_clone,
                    volume_clone,
                    scene_clone,
                );
            })
            .ok();

        Self {
            active,
            output_volume,
            render_levels: Mutex::new(([0.; 32], Instant::now())),
            levels,
            scene_levels,
            trap_state: Mutex::new(TrapResponse::new()),
            has_live_signal,
            last_update,
        }
    }
    pub(crate) fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
    }

    pub(crate) fn set_output_volume(&self, volume: u32) {
        self.output_volume
            .store((volume.min(100) as f32 / 100.).to_bits(), Ordering::Relaxed);
    }
    #[cfg(test)]
    pub(crate) fn fixture_levels(&self, bands: [f32; 32]) {
        *self.levels.lock().unwrap() = bands;
        *self.scene_levels.lock().unwrap() = bands;
        *self.render_levels.lock().unwrap() = (bands, Instant::now());
        *self.last_update.lock().unwrap() = Instant::now();
        let now = Instant::now();
        let start = now - std::time::Duration::from_secs(1);
        let mut state = self.trap_state.lock().unwrap();
        state.last = start;
        for frame in 1..=60 {
            state.frame(
                bands,
                start + std::time::Duration::from_millis(frame * 16),
                199,
                80.,
            );
        }
    }
    fn spectrum(&self) -> [f32; 32] {
        let fresh = self
            .last_update
            .lock()
            .map(|at| at.elapsed().as_millis() < 400)
            .unwrap_or(false);
        let raw = crate::connect::visualizer_spectrum().unwrap_or_else(|| {
            if fresh {
                self.levels.lock().map(|v| *v).unwrap_or([0.; 32])
            } else {
                [0.; 32]
            }
        });
        let mut state = self.render_levels.lock().unwrap();
        let dt = state.1.elapsed().as_secs_f32().min(0.1);
        state.1 = Instant::now();
        follow_levels(&mut state.0, &raw, dt);
        state.0
    }
    pub fn compute_wave(
        &self,
        width: f32,
        height: f32,
        baseline_y: f32,
        playing: bool,
        _position_ms: u64,
        _eq: &[i32; 11],
    ) -> Vec<(f32, f32)> {
        let bands = self.spectrum();
        let max_h = (height * 0.28).clamp(60., 220.);
        (0..96)
            .map(|i| {
                let norm = i as f32 / 95.;
                let at = norm * 31.;
                let low = at.floor() as usize;
                let high = (low + 1).min(31);
                let magnitude = if playing {
                    let smooth = |index: usize| {
                        let mut sum = 0.;
                        let mut weight_sum = 0.;
                        for offset in -3i32..=3 {
                            let weight = (4 - offset.abs()) as f32;
                            let band = (index as i32 + offset).clamp(0, 31) as usize;
                            sum += bands[band] * weight;
                            weight_sum += weight;
                        }
                        sum / weight_sum
                    };
                    smooth(low) * (1. - at.fract()) + smooth(high) * at.fract()
                } else {
                    0.
                };
                (norm * width, baseline_y - magnitude * max_h)
            })
            .collect()
    }
    pub fn compute_bars(&self, count: usize, _max_h: f32) -> Vec<f32> {
        let bands = self.spectrum();
        (0..count)
            .map(|i| {
                // Mirror the spectrum around the center: bass in the middle,
                // treble at both edges.
                let position = i as f32 / (count.max(2) - 1) as f32;
                let at = (position * 2. - 1.).abs() * 31.;
                let low = at.floor() as usize;
                bands[low] * (1. - at.fract()) + bands[(low + 1).min(31)] * at.fract()
            })
            .collect()
    }
    pub fn compute_trap_nation(&self, count: usize, max_spike: f32) -> (Vec<Vec<f32>>, f32) {
        let raw = crate::connect::visualizer_trap_spectrum().unwrap_or_else(|| {
            if self
                .last_update
                .lock()
                .map(|at| at.elapsed().as_millis() < 400)
                .unwrap_or(false)
            {
                *self.scene_levels.lock().unwrap()
            } else {
                [0.; 32]
            }
        });
        self.trap_state
            .lock()
            .unwrap()
            .frame(raw, Instant::now(), count, max_spike)
    }
}

// Translates the supplied scene into time-based smoothing and layer trails.
// Scene frame delays 0/3/6/9/12 are represented as 0/50/100/150/200 ms.
struct TrapResponse {
    levels: [f32; 32],
    beat: f32,
    last: Instant,
    history: VecDeque<(Instant, [f32; 32])>,
    kernel_count: Option<usize>,
    kernels: [(Vec<f32>, f32); 5],
    outline_scratch: Vec<f32>,
}
impl TrapResponse {
    fn new() -> Self {
        Self {
            levels: [0.; 32],
            beat: 0.,
            last: Instant::now(),
            history: VecDeque::new(),
            kernel_count: None,
            kernels: std::array::from_fn(|_| (Vec::new(), 0.)),
            outline_scratch: Vec::new(),
        }
    }
    fn frame(
        &mut self,
        raw: [f32; 32],
        now: Instant,
        count: usize,
        max_spike: f32,
    ) -> (Vec<Vec<f32>>, f32) {
        let dt = now
            .saturating_duration_since(self.last)
            .as_secs_f32()
            .min(0.1);
        self.last = now;
        let mut raw = raw;
        for bass in &mut raw[..5] {
            *bass = (*bass * 1.5).clamp(0., 1.);
        }
        follow_levels(&mut self.levels, &raw, dt);
        let bass = self.levels[..5].iter().copied().fold(0., f32::max);
        self.beat += (bass - self.beat) * (1. - 0.6f32.powf(dt * 60.));
        self.history.push_back((now, self.levels));
        while self.history.len() > 64
            || self
                .history
                .front()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at).as_millis() > 450)
        {
            self.history.pop_front();
        }
        if self.kernel_count != Some(count) {
            for (layer, (weights, sum)) in self.kernels.iter_mut().enumerate() {
                let radius = (10 + layer * 2).min(count.saturating_sub(1));
                let sigma = (radius as f32 / 2.4).max(1.);
                weights.clear();
                weights.extend(
                    (-(radius as isize)..=radius as isize)
                        .map(|offset| (-(offset as f32).powi(2) / (2. * sigma * sigma)).exp()),
                );
                *sum = weights.iter().sum();
            }
            self.kernel_count = Some(count);
        }
        let layers = (0..5)
            .map(|layer| {
                let target = now
                    .checked_sub(std::time::Duration::from_millis(layer * 50))
                    .unwrap_or(now);
                let levels = delayed_levels(&self.history, target);
                let gain = (3.5 + layer as f32 * 0.5) / 3.5;
                scene_contour(
                    &levels,
                    count,
                    max_spike * gain,
                    &self.kernels[layer as usize],
                    &mut self.outline_scratch,
                )
            })
            .collect();
        (layers, self.beat)
    }
}
fn delayed_levels(history: &VecDeque<(Instant, [f32; 32])>, target: Instant) -> [f32; 32] {
    let Some(first) = history.front() else {
        return [0.; 32];
    };
    if target < first.0 {
        return [0.; 32];
    }
    for index in 1..history.len() {
        let a = &history[index - 1];
        let b = &history[index];
        if target <= b.0 {
            let span = b.0.saturating_duration_since(a.0).as_secs_f32();
            let blend = if span > 0. {
                target.saturating_duration_since(a.0).as_secs_f32() / span
            } else {
                1.
            };
            return std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * blend);
        }
    }
    history.back().unwrap().1
}
fn scene_contour(
    bands: &[f32; 32],
    count: usize,
    height: f32,
    kernel: &(Vec<f32>, f32),
    scratch: &mut Vec<f32>,
) -> Vec<f32> {
    let raw = trap_outline_with_scratch(bands, count, height, scratch);
    if count == 0 {
        return raw;
    }
    let (weights, sum) = kernel;
    let radius = weights.len() / 2;
    (0..count)
        .map(|index| {
            weights
                .iter()
                .enumerate()
                .map(|(offset, weight)| raw[(index + count + offset - radius) % count] * weight)
                .sum::<f32>()
                / *sum
        })
        .collect()
}
fn follow_levels(levels: &mut [f32; 32], targets: &[f32; 32], dt: f32) {
    for (level, target) in levels.iter_mut().zip(targets) {
        let target = target.clamp(0., 1.);
        let tau = if target > *level { 0.018 } else { 0.080 };
        *level += (target - *level) * (1. - (-dt.max(0.) / tau).exp());
    }
}
#[cfg(test)]
fn trap_outline(bands: &[f32; 32], count: usize, max_spike: f32) -> Vec<f32> {
    trap_outline_with_scratch(bands, count, max_spike, &mut Vec::new())
}
fn trap_outline_with_scratch(
    bands: &[f32; 32],
    count: usize,
    max_spike: f32,
    raw: &mut Vec<f32>,
) -> Vec<f32> {
    if count < 4 {
        return vec![0.; count];
    }
    let sample = |band: f32| {
        let low = band.floor().clamp(0., 31.) as usize;
        bands[low] * (1. - band.fract()) + bands[(low + 1).min(31)] * band.fract()
    };
    raw.clear();
    raw.extend((0..count).map(|k| {
        let angle = k as f32 / count as f32 * std::f32::consts::TAU;
        let band =
            (1. - angle.abs().min(std::f32::consts::TAU - angle) / std::f32::consts::PI) * 31.;
        sample(band.clamp(0., 31.)) * max_spike
    }));
    (0..count)
        .map(|k| {
            (raw[(k + count - 2) % count]
                + raw[(k + count - 1) % count] * 4.
                + raw[k] * 6.
                + raw[(k + 1) % count] * 4.
                + raw[(k + 2) % count])
                / 16.
        })
        .collect()
}
// Compensate only the known playback volume taper. Never normalize the
// music's own RMS: doing that turns every quiet phrase into a loud one.
fn analysis_gain(volume: f32) -> f32 {
    let volume = volume.clamp(0., 1.);
    if volume > 0. {
        1. / volume.powi(3)
    } else {
        0.
    }
}
unsafe fn run_wasapi_capture(
    levels: Arc<Mutex<[f32; 32]>>,
    has_live_signal: Arc<Mutex<bool>>,
    last_update: Arc<Mutex<Instant>>,
    active: Arc<AtomicBool>,
    output_volume: Arc<AtomicU32>,
    scene_levels: Arc<Mutex<[f32; 32]>>,
) {
    let com_result = CoInitializeEx(std::ptr::null_mut(), 0);
    if com_result < 0 {
        return;
    }
    loop {
        if active.load(Ordering::Relaxed) {
            capture_session(
                &levels,
                &has_live_signal,
                &last_update,
                &active,
                &output_volume,
                &scene_levels,
            );
        }
        if let Ok(mut value) = levels.lock() {
            *value = [0.; 32];
        }
        if let Ok(mut value) = scene_levels.lock() {
            *value = [0.; 32];
        }
        if let Ok(mut value) = has_live_signal.lock() {
            *value = false;
        }
        // Retry failed/invalidated endpoints, and reopen after a window restore.
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

unsafe fn capture_session(
    levels: &Mutex<[f32; 32]>,
    has_live_signal: &Mutex<bool>,
    last_update: &Mutex<Instant>,
    active: &AtomicBool,
    output_volume: &AtomicU32,
    scene_levels: &Mutex<[f32; 32]>,
) {
    let mut enumerator: *mut c_void = std::ptr::null_mut();
    let hr = CoCreateInstance(
        &CLSID_MM_DEVICE_ENUMERATOR as *const _ as _,
        std::ptr::null_mut(),
        CLSCTX_ALL,
        &IID_IMM_DEVICE_ENUMERATOR as *const _ as _,
        &mut enumerator,
    );
    if hr != 0 || enumerator.is_null() {
        return;
    }

    let enum_vtbl = *(enumerator as *mut *mut IMMDeviceEnumeratorVtbl);
    let mut device: *mut c_void = std::ptr::null_mut();
    let hr = ((*enum_vtbl).get_default_audio_endpoint)(enumerator, 0, 0, &mut device);
    if hr != 0 || device.is_null() {
        ((*enum_vtbl).release)(enumerator);
        return;
    }

    let dev_vtbl = *(device as *mut *mut IMMDeviceVtbl);
    let mut audio_client: *mut c_void = std::ptr::null_mut();
    let hr = ((*dev_vtbl).activate)(
        device,
        &IID_IAUDIO_CLIENT,
        CLSCTX_ALL,
        std::ptr::null(),
        &mut audio_client,
    );
    ((*dev_vtbl).release)(device);
    ((*enum_vtbl).release)(enumerator);

    if hr != 0 || audio_client.is_null() {
        return;
    }

    let client_vtbl = *(audio_client as *mut *mut IAudioClientVtbl);
    let mut format_ptr: *mut c_void = std::ptr::null_mut();
    let hr = ((*client_vtbl).get_mix_format)(audio_client, &mut format_ptr);
    if hr != 0 || format_ptr.is_null() {
        ((*client_vtbl).release)(audio_client);
        return;
    }

    let fmt = *(format_ptr as *const WaveFormatEx);
    let channels = fmt.n_channels.max(1) as usize;
    let sample_rate = fmt.n_samples_per_sec.max(8000) as f32;
    let is_32bit = fmt.w_bits_per_sample == 32;
    let is_16bit = fmt.w_bits_per_sample == 16;

    let hr = ((*client_vtbl).initialize)(
        audio_client,
        0, // AUDCLNT_SHAREMODE_SHARED
        AUDCLNT_STREAMFLAGS_LOOPBACK,
        1_000_000i64, // 100 ms buffer
        0i64,
        format_ptr,
        std::ptr::null(),
    );
    CoTaskMemFree(format_ptr);
    if hr != 0 {
        ((*client_vtbl).release)(audio_client);
        return;
    }

    let mut capture_ptr: *mut c_void = std::ptr::null_mut();
    let hr =
        ((*client_vtbl).get_service)(audio_client, &IID_IAUDIO_CAPTURE_CLIENT, &mut capture_ptr);
    if hr != 0 || capture_ptr.is_null() {
        ((*client_vtbl).release)(audio_client);
        return;
    }

    let capture_vtbl = *(capture_ptr as *mut *mut IAudioCaptureClientVtbl);

    let mut smoothed = [0.0f32; 64];
    let mut mono_ring: Vec<f32> = Vec::with_capacity(8192);

    const N: usize = 4096;
    let pi2 = 2.0 * std::f32::consts::PI;

    // Precalculate Hann window coefficients
    let mut hann = [0.0f32; N];
    for n in 0..N {
        hann[n] = 0.5 * (1.0 - (pi2 * n as f32 / N as f32).cos());
    }

    // Precompute the windowed DFT coefficients: no per-frame trigonometry.
    let coefficients: Vec<_> = (0..64)
        .map(|band| {
            let frequency = if band < 32 {
                40. * 300f32.powf((band as f32 + 0.5) / 32.)
            } else {
                crate::connect::scene_frequency(((band - 32) as f32 + 0.5) / 32.)
            };
            let omega = pi2 * frequency / sample_rate;
            (
                (0..N)
                    .map(|n| (omega * n as f32).cos() * hann[n])
                    .collect::<Vec<_>>(),
                (0..N)
                    .map(|n| (omega * n as f32).sin() * hann[n])
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    if ((*client_vtbl).start)(audio_client) < 0 {
        ((*capture_vtbl).release)(capture_ptr);
        ((*client_vtbl).release)(audio_client);
        return;
    }
    'capture: loop {
        if !active.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));

        let mut packet_length = 0u32;
        let mut got_audio = false;

        loop {
            let hr = ((*capture_vtbl).get_next_packet_size)(capture_ptr, &mut packet_length);
            if hr < 0 {
                break 'capture;
            }
            if packet_length == 0 {
                break;
            }
            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut num_frames = 0u32;
            let mut flags = 0u32;
            let mut dev_pos = 0u64;
            let mut qpc_pos = 0u64;

            let hr = ((*capture_vtbl).get_buffer)(
                capture_ptr,
                &mut data_ptr,
                &mut num_frames,
                &mut flags,
                &mut dev_pos,
                &mut qpc_pos,
            );

            if hr == 0 && num_frames > 0 && !data_ptr.is_null() {
                if (flags & AUDCLNT_BUFFERFLAGS_SILENT) == 0 {
                    let nf = num_frames as usize;
                    if is_32bit {
                        let ptr = data_ptr as *const f32;
                        for i in 0..nf {
                            let mut sum = 0.0f32;
                            for c in 0..channels {
                                sum += *ptr.add(i * channels + c);
                            }
                            mono_ring.push(sum / channels as f32);
                        }
                    } else if is_16bit {
                        let ptr = data_ptr as *const i16;
                        for i in 0..nf {
                            let mut sum = 0.0f32;
                            for c in 0..channels {
                                sum += *ptr.add(i * channels + c) as f32 / 32768.0;
                            }
                            mono_ring.push(sum / channels as f32);
                        }
                    }

                    if mono_ring.len() > 8192 {
                        let excess = mono_ring.len() - 4096;
                        mono_ring.drain(0..excess);
                    }

                    got_audio = true;
                }
                if ((*capture_vtbl).release_buffer)(capture_ptr, num_frames) < 0 {
                    break 'capture;
                }
            } else {
                break 'capture;
            }
        }

        // Release queued packets before analyzing only the newest audio window.
        if got_audio && mono_ring.len() >= N {
            let window_start = mono_ring.len() - N;
            let window = &mono_ring[window_start..];
            let gain =
                analysis_gain(f32::from_bits(output_volume.load(Ordering::Relaxed)));
            for b in 0..64 {
                let (cosine, sine) = &coefficients[b];
                let mut re = 0.;
                let mut im = 0.;
                for n in 0..N {
                    let sample = window[n] * gain;
                    re += sample * cosine[n];
                    im += sample * sine[n];
                }
                let magnitude = (re * re + im * im).sqrt() / (N as f32 * 0.25);
                let db = if magnitude > 0. {
                    20. * magnitude.log10()
                } else {
                    -120.
                };
                smoothed[b] = crate::connect::dynamic_level(db);
            }
        }
        if !got_audio {
            for b in smoothed.iter_mut() {
                *b *= 0.88;
            }
        }

        let total_energy: f32 = smoothed.iter().sum();
        let active = got_audio && total_energy > 0.03;

        if let Ok(mut l) = levels.lock() {
            l.copy_from_slice(&smoothed[..32]);
        }
        if let Ok(mut value) = scene_levels.lock() {
            value.copy_from_slice(&smoothed[32..]);
        }
        if let Ok(mut s) = has_live_signal.lock() {
            *s = active;
        }
        if active {
            if let Ok(mut u) = last_update.lock() {
                *u = Instant::now();
            }
        }
    }
    let _ = ((*client_vtbl).stop)(audio_client);
    ((*capture_vtbl).release)(capture_ptr);
    ((*client_vtbl).release)(audio_client);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_kernels_match_fresh_smoothing_after_size_changes() {
        let mut state = TrapResponse::new();
        let bands = std::array::from_fn(|i| i as f32 / 32.);
        for count in [0, 1, 3, 17, 199, 17] {
            state.frame(bands, Instant::now(), count, 100.);
            for layer in 0..5 {
                let radius = (10 + layer * 2).min(count.saturating_sub(1));
                let sigma = (radius as f32 / 2.4).max(1.);
                let weights: Vec<_> = (-(radius as isize)..=radius as isize)
                    .map(|offset| (-(offset as f32).powi(2) / (2. * sigma * sigma)).exp())
                    .collect();
                let sum: f32 = weights.iter().sum();
                let raw = trap_outline(&bands, count, 100.);
                let expected: Vec<_> = (0..count)
                    .map(|index| {
                        weights
                            .iter()
                            .enumerate()
                            .map(|(offset, weight)| {
                                raw[(index + count + offset - radius) % count] * weight
                            })
                            .sum::<f32>()
                            / sum
                    })
                    .collect();
                let actual = scene_contour(
                    &bands,
                    count,
                    100.,
                    &state.kernels[layer],
                    &mut state.outline_scratch,
                );
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn scene_layers_trail_a_hit_then_return_to_rest() {
        let mut state = TrapResponse::new();
        let start = state.last;
        let mut frame = (Vec::new(), 0.);
        for index in 1..=5 {
            frame = state.frame(
                [0.7; 32],
                start + std::time::Duration::from_millis(index * 16),
                199,
                100.,
            );
        }
        assert!(frame.0[0].iter().any(|v| *v > 20.));
        assert!(frame.0[4].iter().all(|v| *v == 0.));
        for index in 6..=90 {
            frame = state.frame(
                [0.; 32],
                start + std::time::Duration::from_millis(index * 16),
                199,
                100.,
            );
        }
        assert!(frame.0.iter().flatten().all(|v| *v < 0.01));
        assert!(frame.1 < 0.001);
        assert!(state.history.len() <= 64);
    }
    #[test]
    fn hits_rise_quickly_and_drop_between_beats_without_lifting_quiet_music() {
        let mut levels = [0.; 32];
        let mut hit = [0.; 32];
        hit[3] = 0.9;
        follow_levels(&mut levels, &hit, 0.036);
        assert!(levels[3] > 0.75);
        assert!(levels.iter().enumerate().all(|(i, v)| i == 3 || *v == 0.));
        let quiet = [0.04; 32];
        follow_levels(&mut levels, &quiet, 0.2);
        assert!(levels[3] < 0.12);
        follow_levels(&mut levels, &quiet, 1.);
        assert!(levels.iter().all(|value| (*value - 0.04).abs() < 0.0001));
    }
    #[test]
    fn trap_outline_is_closed_symmetric_and_preserves_frequency_position() {
        let mut low = [0.; 32];
        low[0] = 1.;
        let mut high = [0.; 32];
        high[31] = 1.;
        let bass = trap_outline(&low, 128, 100.);
        let treble = trap_outline(&high, 128, 100.);
        assert!(bass[64] > bass[0]);
        assert!(treble[0] > treble[64]);
        for k in 1..128 {
            assert!((bass[k] - bass[128 - k]).abs() < 0.001);
        }
        assert!((treble[0] - treble[127]).abs() < 20.);
        assert!(trap_outline(&[0.; 32], 128, 100.).iter().all(|v| *v == 0.));
    }
    #[test]
    fn analysis_level_is_invariant_to_output_gain() {
        for music_amplitude in [0.003, 0.03, 0.3] {
            for volume in [0.01f32, 0.1, 0.5, 1.] {
                let recorded = music_amplitude * volume.powi(3);
                assert!((recorded * analysis_gain(volume) - music_amplitude).abs() < 0.00001);
            }
        }
        assert_eq!(analysis_gain(0.), 0.);
        // A quiet phrase must remain ten times quieter than a loud hit.
        assert!((0.03 * analysis_gain(0.5)) / (0.3 * analysis_gain(0.5)) < 0.11);
    }
    #[test]
    fn test_visualizer_creation_and_wave_generation() {
        let v = Visualizer::new();
        let fg = v.compute_wave(1200.0, 800.0, 784.0, true, 45000, &[0; 11]);
        assert_eq!(fg.len(), 96);
        for p in &fg {
            assert!(p.1 <= 784.0);
        }
    }
}
