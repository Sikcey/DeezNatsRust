use crate::model::{ImageData, State};
use serde_json::Value;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::*;

pub struct Handle(pub *mut c_void);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
pub struct Request {
    pub handle: Handle,
    _connection: Handle,
}
fn session() -> Result<&'static Handle, String> {
    static SESSION: OnceLock<Result<Handle, String>> = OnceLock::new();
    SESSION
        .get_or_init(|| unsafe {
            let handle = Handle(WinHttpOpen(
                crate::model::wide("DeezNatsJam/0.1").as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                null(),
                null(),
                0,
            ));
            if handle.0.is_null() {
                Err("Could not initialize networking".into())
            } else {
                Ok(handle)
            }
        })
        .as_ref()
        .map_err(Clone::clone)
}
#[derive(Clone, Debug)]
pub struct Address {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub secure: bool,
}
impl Address {
    pub fn parse(url: &str) -> Result<Self, String> {
        let (secure, rest) = if let Some(s) = url.strip_prefix("http://") {
            (false, s)
        } else if let Some(s) = url.strip_prefix("https://") {
            (true, s)
        } else {
            return Err("Use an http:// or https:// host address".into());
        };
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() || authority.contains(['@', '?', '#', '\\', '\r', '\n', ' ']) {
            return Err("Invalid host address".into());
        }
        let (host, port) = if authority.starts_with('[') {
            let end = authority.find(']').ok_or("Invalid IPv6 address")?;
            let port = authority[end + 1..]
                .strip_prefix(':')
                .map(str::parse)
                .transpose()
                .map_err(|_| "Invalid port")?
                .unwrap_or(if secure { 443 } else { 80 });
            (authority[..=end].to_owned(), port)
        } else if let Some((h, p)) = authority.rsplit_once(':') {
            (h.to_owned(), p.parse().map_err(|_| "Invalid port")?)
        } else {
            (authority.to_owned(), if secure { 443 } else { 80 })
        };
        if host.is_empty() || port == 0 || path.contains(['\r', '\n']) {
            return Err("Invalid host address".into());
        }
        Ok(Self {
            host,
            port,
            path: format!("/{path}"),
            secure,
        })
    }
}
pub fn normalize_host(s: &str) -> Result<String, String> {
    let s = s.trim().trim_end_matches('/');
    let u = if s.contains("://") {
        s.to_owned()
    } else {
        format!("http://{s}")
    };
    let a = Address::parse(&u)?;
    if a.path != "/" {
        return Err("Enter only the host address and port".into());
    }
    Ok(u)
}
impl Request {
    pub fn open(method: &str, url: &str, body: &[u8], timeout: i32) -> Result<Self, String> {
        let a = Address::parse(url)?;
        unsafe {
            let session = session()?;
            let connection = Handle(WinHttpConnect(
                session.0,
                crate::model::wide(a.host.trim_matches(['[', ']'])).as_ptr(),
                a.port,
                0,
            ));
            if connection.0.is_null() {
                return Err("Could not connect to the host".into());
            }
            let handle = Handle(WinHttpOpenRequest(
                connection.0,
                crate::model::wide(method).as_ptr(),
                crate::model::wide(&a.path).as_ptr(),
                null(),
                null(),
                null(),
                if a.secure { WINHTTP_FLAG_SECURE } else { 0 },
            ));
            if handle.0.is_null() {
                return Err("Could not create request".into());
            }
            WinHttpSetTimeouts(handle.0, timeout, timeout, timeout, timeout);
            let header =
                crate::model::wide("Content-Type: application/json\r\nCache-Control: no-cache\r\n");
            if WinHttpSendRequest(
                handle.0,
                header.as_ptr(),
                (header.len() - 1) as u32,
                if body.is_empty() {
                    null_mut()
                } else {
                    body.as_ptr() as *mut _
                },
                body.len() as u32,
                body.len() as u32,
                0,
            ) == 0 {
                return Err(format!("Could not send to the DeezNats host: {}", std::io::Error::last_os_error()));
            }
            if WinHttpReceiveResponse(handle.0, null_mut()) == 0 {
                return Err(format!("Could not receive from the DeezNats host: {}", std::io::Error::last_os_error()));
            }
            let mut status = 0u32;
            let mut len = 4;
            WinHttpQueryHeaders(
                handle.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut status as *mut u32).cast(),
                &mut len,
                null_mut(),
            );
            if !(200..300).contains(&status) {
                return Err(format!("Host returned HTTP {status}"));
            }
            Ok(Self {
                _connection: connection,
                handle,
            })
        }
    }
    pub fn read(&self, out: &mut [u8]) -> Result<usize, String> {
        let mut n = 0;
        unsafe {
            if WinHttpReadData(
                self.handle.0,
                out.as_mut_ptr().cast(),
                out.len() as u32,
                &mut n,
            ) == 0
            {
                return Err("Connection interrupted".into());
            }
        }
        Ok(n as usize)
    }
    pub fn bytes(&self, limit: usize) -> Result<Vec<u8>, String> {
        let mut all = Vec::new();
        let mut buf = [0u8; 16384];
        loop {
            let n = self.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if all.len() + n > limit {
                return Err("Response exceeds the client limit".into());
            }
            all.extend_from_slice(&buf[..n]);
        }
        Ok(all)
    }
}
pub fn get(url: &str) -> Result<Vec<u8>, String> {
    Request::open("GET", url, &[], 6000)?.bytes(24 * 1024 * 1024)
}
pub fn json(method: &str, url: &str, body: &Value) -> Result<Value, String> {
    let b = if method == "POST" {
        body.to_string().into_bytes()
    } else {
        Vec::new()
    };
    let request = Request::open(method, url, &b, 6000)?;
    let bytes = request.bytes(8 * 1024 * 1024)?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Host returned invalid JSON".into())
}

pub enum Event {
    State(Result<Value, String>),
    Data(String, u64, Result<Value, String>),
    Image(String, Result<Arc<ImageData>, String>),
    Spectrum([f32; 32]),
}
struct Job {
    key: String,
    generation: u64,
    url: String,
    method: &'static str,
    body: Value,
}
pub struct Worker {
    pub events: mpsc::Receiver<Event>,
    jobs: mpsc::SyncSender<Job>,
    actions: mpsc::SyncSender<Job>,
    image_jobs: mpsc::SyncSender<String>,
    pub visible: Arc<AtomicBool>,
    pub lyrics: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    host: String,
}
impl Worker {
    pub fn start(host: String) -> Self {
        let (tx, events) = mpsc::channel();
        let (jobs, jobs_rx) = mpsc::sync_channel::<Job>(32);
        let (actions, actions_rx) = mpsc::sync_channel::<Job>(32);
        let (image_jobs, image_rx) = mpsc::sync_channel::<String>(128);
        let visible = Arc::new(AtomicBool::new(true));
        let lyrics = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (tx, host, visible, lyrics, stop) = (
                tx.clone(),
                host.clone(),
                visible.clone(),
                lyrics.clone(),
                stop.clone(),
            );
            std::thread::spawn(move || {
                let mut next_state = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    if now >= next_state {
                        let result = json("GET", &format!("{host}/api/state"), &Value::Null);
                        if tx.send(Event::State(result)).is_err() {
                            break;
                        }
                        next_state = Instant::now()
                            + Duration::from_millis(if visible.load(Ordering::Relaxed) {
                                500
                            } else {
                                2000
                            });
                    }
                    if visible.load(Ordering::Relaxed) && lyrics.load(Ordering::Relaxed) {
                        if let Ok(v) = json("GET", &format!("{host}/api/spectrum"), &Value::Null) {
                            let mut levels = [0.; 32];
                            if let Some(a) = v["levels"].as_array() {
                                for (i, value) in a.iter().take(32).enumerate() {
                                    levels[i] = value.as_f64().unwrap_or(0.).clamp(0., 1.) as f32
                                }
                            }
                            let _ = tx.send(Event::Spectrum(levels));
                        }
                    }
                    std::thread::sleep(Duration::from_millis(
                        if lyrics.load(Ordering::Relaxed) && visible.load(Ordering::Relaxed) {
                            80
                        } else {
                            100
                        },
                    ));
                }
            });
        }
        let jobs_rx = Arc::new(Mutex::new(jobs_rx));
        for _ in 0..2 {
            let (tx, rx) = (tx.clone(), jobs_rx.clone());
            std::thread::spawn(move || loop {
                let Ok(job) = rx.lock().unwrap().recv() else {
                    break;
                };
                let result = json(job.method, &job.url, &job.body);
                if tx
                    .send(Event::Data(job.key, job.generation, result))
                    .is_err()
                {
                    break;
                }
            });
        }
        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                while let Ok(job) = actions_rx.recv() {
                    let result = json(job.method, &job.url, &job.body);
                    if tx
                        .send(Event::Data(job.key, job.generation, result))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        let image_rx = Arc::new(Mutex::new(image_rx));
        for _ in 0..3 {
            let (tx, rx) = (tx.clone(), image_rx.clone());
            std::thread::spawn(move || loop {
                let Ok(url) = rx.lock().unwrap().recv() else {
                    break;
                };
                let result = (|| {
                    let bytes = get(&url)?;
                    let img = image::load_from_memory(&bytes).map_err(|_| "Invalid image")?;
                    let img = img.thumbnail(400, 400).into_rgba8();
                    let (width, height) = img.dimensions();
                    Ok(Arc::new(ImageData {
                        width,
                        height,
                        rgba: img.into_raw(),
                    }))
                })();
                if tx.send(Event::Image(url, result)).is_err() {
                    break;
                }
            });
        }
        Self {
            events,
            jobs,
            actions,
            image_jobs,
            visible,
            lyrics,
            stop,
            host,
        }
    }
    pub fn request(&self, key: &str, generation: u64, path: &str) -> Result<(), String> {
        self.jobs
            .try_send(Job {
                key: key.into(),
                generation,
                url: format!("{}{path}", self.host),
                method: "GET",
                body: Value::Null,
            })
            .map_err(|_| "The host is busy. Please retry.".into())
    }
    pub fn action(&self, path: &str, body: Value) -> Result<(), String> {
        self.actions
            .try_send(Job {
                key: format!("action:{path}"),
                generation: 0,
                url: format!("{}{path}", self.host),
                method: "POST",
                body,
            })
            .map_err(|_| "The host is busy. Please retry.".into())
    }
    pub fn image(&self, url: &str) -> bool {
        self.image_jobs.try_send(url.into()).is_ok()
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn refresh_images(&self, state: &mut State, urls: Vec<String>) {
        let needed: std::collections::HashSet<_> =
            urls.into_iter().filter(|url| !url.is_empty()).collect();
        state.image_retry_at.retain(|url, _| needed.contains(url));
        for url in needed {
            if !url.is_empty()
                && !state.images.contains_key(&url)
                && state.image_due(&url, Instant::now())
            {
                if self.image(&url) {
                    state.requested_images.insert(url);
                } else {
                    state.image_failed(url, Instant::now());
                }
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_follow_the_selected_host_after_reconnect() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        fn fixture(ip: &str) -> (String, mpsc::Receiver<String>) {
            let listener = TcpListener::bind((ip, 0)).unwrap();
            let host = format!("http://{}", listener.local_addr().unwrap());
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = stream.unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                    }
                    let headers = String::from_utf8(request).unwrap();
                    let length = headers.lines().filter_map(|l| l.split_once(':'))
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .map(|(_, v)| v.trim().parse::<usize>().unwrap()).unwrap_or(0);
                    stream.read_exact(&mut vec![0; length]).unwrap();
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
                    if headers.starts_with("POST ") {
                        tx.send(headers.lines().next().unwrap().to_owned()).unwrap();
                        break;
                    }
                }
            });
            (host, rx)
        }

        let (original, original_commands) = fixture("127.0.0.1");
        let (selected, selected_commands) = fixture("127.0.0.2");
        let mut worker = Worker::start(original);
        worker.action("/api/toggle", Value::Null).unwrap();
        assert_eq!(original_commands.recv_timeout(Duration::from_secs(3)).unwrap(), "POST /api/toggle HTTP/1.1");
        worker = Worker::start(selected.clone());
        worker.action("/api/next", Value::Null).unwrap();
        assert_eq!(worker.host(), selected);
        assert_eq!(selected_commands.recv_timeout(Duration::from_secs(3)).unwrap(), "POST /api/next HTTP/1.1");
        assert!(original_commands.try_recv().is_err());
    }

    #[test]
    fn addresses_support_duckdns_lan_https_and_ipv6() {
        for u in [
            "http://myjam.duckdns.org:3131",
            "http://192.168.1.31:3131",
            "https://music.example.org",
            "http://[::1]:3131",
        ] {
            assert!(Address::parse(u).is_ok(), "{u}")
        }
        for u in [
            "file:///C:/secret",
            "http://user:password@host",
            "http://host:0",
            "http://host\r\nCookie:x",
        ] {
            assert!(Address::parse(u).is_err(), "{u}")
        }
        assert_eq!(
            normalize_host("myjam.duckdns.org:3131/").unwrap(),
            "http://myjam.duckdns.org:3131"
        );
    }
}
