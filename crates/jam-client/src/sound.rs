//! Bounded PCM streaming with software gain; never changes Windows/host volume.
use crate::network::Request;
use std::{
    collections::VecDeque,
    mem::size_of,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};
use windows_sys::Win32::Media::Audio::*;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Threading::{CreateEventW, SetEvent, WaitForSingleObject, INFINITE},
};
const FRAMES: usize = 4410;
struct Wake(HANDLE);
unsafe impl Send for Wake {}
unsafe impl Sync for Wake {}
impl Wake {
    fn new() -> Result<Self, String> {
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if handle.is_null() {
            Err("Could not create the audio event".into())
        } else {
            Ok(Self(handle))
        }
    }
    fn signal(&self) {
        unsafe {
            SetEvent(self.0);
        }
    }
}
impl Drop for Wake {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub struct Sound {
    stop: Option<(Arc<AtomicBool>, Arc<Wake>)>,
    pub ready: Arc<AtomicBool>,
    pub delay: Arc<AtomicU32>,
    pub error: Arc<Mutex<String>>,
    volume: Arc<AtomicU32>,
}
impl Sound {
    pub fn new(volume: f32) -> Self {
        Self {
            stop: None,
            ready: Arc::new(AtomicBool::new(false)),
            delay: Arc::new(AtomicU32::new(0)),
            error: Arc::new(Mutex::new(String::new())),
            volume: Arc::new(AtomicU32::new((volume / 100.).to_bits())),
        }
    }
    pub fn volume(&self, v: f32) {
        self.volume
            .store((v.clamp(0., 100.) / 100.).to_bits(), Ordering::Relaxed)
    }
    pub fn stop(&mut self) {
        if let Some((stop, wake)) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
            wake.signal();
        }
        self.ready.store(false, Ordering::Relaxed);
    }
    pub fn start(&mut self, host: String) {
        self.stop();
        self.ready = Arc::new(AtomicBool::new(false));
        self.delay = Arc::new(AtomicU32::new(0));
        self.error = Arc::new(Mutex::new(String::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let wake = match Wake::new() {
            Ok(w) => Arc::new(w),
            Err(e) => {
                *self.error.lock().unwrap() = e;
                return;
            }
        };
        self.stop = Some((stop.clone(), wake.clone()));
        let (ready, delay, error, volume) = (
            self.ready.clone(),
            self.delay.clone(),
            self.error.clone(),
            self.volume.clone(),
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let (tx, rx) = mpsc::sync_channel(3);
                let debt = Arc::new(AtomicU32::new(0));
                let (reader_stop, reader_host, reader_debt, reader_wake) =
                    (stop.clone(), host.clone(), debt.clone(), wake.clone());
                std::thread::spawn(move || {
                    let result = (|| {
                        let request =
                            Request::open("GET", &format!("{reader_host}/api/stream"), &[], 3000)?;
                        let mut read = |out: &mut [u8]| {
                            let mut n = 0;
                            while n < out.len() {
                                let k = request.read(&mut out[n..])?;
                                if k == 0 {
                                    return Err("The host stopped the audio stream".into());
                                }
                                n += k;
                            }
                            Ok(())
                        };
                        read_header(&mut read)?;
                        let mut bytes = vec![0u8; FRAMES * 4];
                        while !reader_stop.load(Ordering::Relaxed) {
                            read(&mut bytes)?;
                            reader_debt.fetch_add(FRAMES as u32, Ordering::Relaxed);
                            if tx.send(Ok(bytes.clone())).is_err() {
                                break;
                            }
                            reader_wake.signal();
                        }
                        Ok::<_, String>(())
                    })();
                    if let Err(e) = result {
                        let _ = tx.send(Err(e));
                        reader_wake.signal();
                    }
                });
                let result = unsafe { play(rx, &stop, &ready, &delay, &volume, &debt, &wake) };
                ready.store(false, Ordering::Relaxed);
                if let Err(e) = result {
                    *error.lock().unwrap() = e;
                }
                for _ in 0..20 {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
    }
}
impl Drop for Sound {
    fn drop(&mut self) {
        self.stop()
    }
}
fn read_header(read: &mut impl FnMut(&mut [u8]) -> Result<(), String>) -> Result<(), String> {
    let mut header = [0; 12];
    read(&mut header)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err("Host audio is not PCM WAV".into());
    }
    let mut valid = false;
    let mut header_bytes = 12usize;
    loop {
        let mut chunk = [0; 8];
        read(&mut chunk)?;
        let n = u32::from_le_bytes(chunk[4..].try_into().unwrap()) as usize;
        if &chunk[..4] == b"data" {
            return if valid {
                Ok(())
            } else {
                Err("Unsupported host audio format".into())
            };
        }
        if n > 65536 || header_bytes + n > 131072 {
            return Err("Invalid WAV header".into());
        }
        let mut payload = vec![0; n + (n % 2)];
        read(&mut payload)?;
        header_bytes += 8 + payload.len();
        if &chunk[..4] == b"fmt " && n >= 16 {
            valid = u16::from_le_bytes(payload[0..2].try_into().unwrap()) == 1
                && u16::from_le_bytes(payload[2..4].try_into().unwrap()) == 2
                && u32::from_le_bytes(payload[4..8].try_into().unwrap()) == 44100
                && u16::from_le_bytes(payload[14..16].try_into().unwrap()) == 16;
        }
    }
}
fn apply_gain(input: &[u8], out: &mut [i16], from: f32, to: f32) {
    let frames = out.len() / 2;
    for (i, (source, target)) in input.chunks_exact(2).zip(out).enumerate() {
        let gain = from + (to - from) * (i / 2) as f32 / frames.max(1) as f32;
        *target = (i16::from_le_bytes([source[0], source[1]]) as f32 * gain)
            .round()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    }
}
struct Buffer {
    samples: Box<[i16]>,
    header: Box<WAVEHDR>,
    queued: bool,
}
unsafe fn play(
    rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    stop: &AtomicBool,
    ready: &AtomicBool,
    delay: &AtomicU32,
    volume: &AtomicU32,
    debt: &AtomicU32,
    wake: &Wake,
) -> Result<(), String> {
    let format = WAVEFORMATEX {
        wFormatTag: 1,
        nChannels: 2,
        nSamplesPerSec: 44100,
        nAvgBytesPerSec: 176400,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut device = null_mut();
    if waveOutOpen(
        &mut device,
        WAVE_MAPPER,
        &format,
        wake.0 as usize,
        0,
        CALLBACK_EVENT,
    ) != 0
    {
        return Err("Could not open the audio output device".into());
    }
    let mut buffers = Vec::new();
    for _ in 0..4 {
        let mut samples = vec![0i16; FRAMES * 2].into_boxed_slice();
        let mut header = Box::new(WAVEHDR {
            lpData: samples.as_mut_ptr().cast(),
            dwBufferLength: (samples.len() * 2) as u32,
            dwBytesRecorded: 0,
            dwUser: 0,
            dwFlags: 0,
            dwLoops: 0,
            lpNext: null_mut(),
            reserved: 0,
        });
        if waveOutPrepareHeader(device, header.as_mut(), size_of::<WAVEHDR>() as u32) != 0 {
            waveOutReset(device);
            for b in &mut buffers {
                let b: &mut Buffer = b;
                waveOutUnprepareHeader(device, b.header.as_mut(), size_of::<WAVEHDR>() as u32);
            }
            waveOutClose(device);
            return Err("Could not prepare audio buffers".into());
        }
        buffers.push(Buffer {
            samples,
            header,
            queued: false,
        });
    }
    let mut previous = f32::from_bits(volume.load(Ordering::Relaxed));
    let mut result = Ok(());
    let mut pending = VecDeque::new();
    'output: while !stop.load(Ordering::Relaxed) {
        let mut queued = 0;
        for buffer in &mut buffers {
            if buffer.queued && buffer.header.dwFlags & WHDR_DONE == 0 {
                queued += 1;
                continue;
            }
            if buffer.queued {
                debt.fetch_sub(FRAMES as u32, Ordering::Relaxed);
            }
            buffer.queued = false;
            if pending.is_empty() {
                match rx.try_recv() {
                    Ok(Ok(data)) => pending.push_back(data),
                    Ok(Err(e)) => {
                        result = Err(e);
                        break 'output;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        result = Err("Audio stream disconnected".into());
                        break 'output;
                    }
                    Err(mpsc::TryRecvError::Empty) => continue,
                }
            }
            if let Some(data) = pending.pop_front() {
                let gain = f32::from_bits(volume.load(Ordering::Relaxed));
                apply_gain(&data, &mut buffer.samples, previous, gain);
                previous = gain;
                buffer.header.dwFlags &= !WHDR_DONE;
                if waveOutWrite(device, buffer.header.as_mut(), size_of::<WAVEHDR>() as u32) != 0 {
                    result = Err("Audio output failed".into());
                    break 'output;
                }
                buffer.queued = true;
                queued += 1;
                ready.store(true, Ordering::Relaxed);
            }
        }
        delay.store(
            debt.load(Ordering::Relaxed) * 1000 / 44100,
            Ordering::Relaxed,
        );
        if queued == 0 {
            ready.store(false, Ordering::Relaxed);
        }
        WaitForSingleObject(wake.0, INFINITE);
    }
    waveOutReset(device);
    for buffer in &mut buffers {
        waveOutUnprepareHeader(device, buffer.header.as_mut(), size_of::<WAVEHDR>() as u32);
    }
    waveOutClose(device);
    result
}
pub fn smoke() -> Result<(), String> {
    let (tx, rx) = mpsc::sync_channel(2);
    tx.send(Ok(vec![0u8; FRAMES * 4])).unwrap();
    drop(tx);
    let ready = AtomicBool::new(false);
    let result = unsafe {
        play(
            rx,
            &AtomicBool::new(false),
            &ready,
            &AtomicU32::new(0),
            &AtomicU32::new(0f32.to_bits()),
            &AtomicU32::new(FRAMES as u32),
            &Wake::new()?,
        )
    };
    if ready.load(Ordering::Relaxed) {
        Ok(())
    } else {
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_gain_never_mutates_stream_or_other_listeners() {
        let samples = [12000i16, -12000, 30000, -30000];
        let input: Vec<_> = samples.iter().flat_map(|n| n.to_le_bytes()).collect();
        let original = input.clone();
        let mut muted = [0; 4];
        apply_gain(&input, &mut muted, 0., 0.);
        assert_eq!(muted, [0; 4]);
        let mut loud = [0; 4];
        apply_gain(&input, &mut loud, 1., 1.);
        assert_eq!(loud, samples);
        assert_eq!(input, original);
    }
    #[test]
    fn wav_parser_accepts_extra_chunks_and_rejects_compressed_audio() {
        let mut wav = b"RIFF\xff\xff\xff\x7fWAVEfmt \x10\0\0\0".to_vec();
        wav.extend_from_slice(&[1, 0, 2, 0, 0x44, 0xac, 0, 0, 0x10, 0xb1, 2, 0, 4, 0, 16, 0]);
        wav.extend_from_slice(b"JUNK\x02\0\0\0xxdata\xff\xff\xff\x7f");
        let parse = |bytes: Vec<u8>| {
            let mut at = 0;
            read_header(&mut |out| {
                if at + out.len() > bytes.len() {
                    return Err("short".into());
                }
                out.copy_from_slice(&bytes[at..at + out.len()]);
                at += out.len();
                Ok(())
            })
        };
        assert!(parse(wav.clone()).is_ok());
        wav[20] = 3;
        assert!(parse(wav).is_err());
    }
}
