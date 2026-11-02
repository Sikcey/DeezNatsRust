//! Hidden, bounded child processes tied to the application's Windows job lifetime.
use std::{
    io::Read,
    os::windows::{
        io::{AsHandle, AsRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::*,
};
const NO_WINDOW: u32 = 0x08000000;
pub(super) const MAX_OUTPUT: usize = 2 * 1024 * 1024;

/// A duplicated handle pins the supervised process identity, including after exit.
/// Never trust a PID file or discover an arbitrary process by executable name.
pub(crate) struct AudioTarget {
    pub pid: u32,
    process: OwnedHandle,
}
impl AudioTarget {
    pub(crate) fn from_child(child: &Child) -> Result<Self, String> {
        let process = child
            .as_handle()
            .try_clone_to_owned()
            .map_err(|_| "Could not retain the supervised Soloist process")?;
        Ok(Self {
            pid: child.id(),
            process,
        })
    }
    pub fn is_alive(&self) -> bool {
        unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(
                self.process.as_raw_handle(),
                0,
            ) == windows_sys::Win32::Foundation::WAIT_TIMEOUT
        }
    }
}

struct Job(HANDLE);
unsafe impl Send for Job {}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Job {
    fn new(child: &Child) -> Result<Self, String> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("Could not create Soloist process job".into());
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                return Err("Could not bind Soloist to the app lifetime".into());
            }
            Ok(job)
        }
    }
}
pub(super) struct Process {
    pub child: Child,
    job: Option<Job>,
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_reader: Option<thread::JoinHandle<()>>,
}
impl Process {
    pub fn audio_target(&self) -> Result<AudioTarget, String> {
        AudioTarget::from_child(&self.child)
    }
    pub fn spawn(command: &mut Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(NO_WINDOW)
            .spawn()
            .map_err(|error| format!("Could not start Soloist process: {error}"))?;
        match Job::new(&child) {
            Ok(job) => {
                let mut pipe = child
                    .stderr
                    .take()
                    .ok_or("Soloist diagnostic stream unavailable")?;
                let stderr = Arc::new(Mutex::new(Vec::new()));
                let output = stderr.clone();
                let reader = thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    while let Ok(count) = pipe.read(&mut buffer) {
                        if count == 0 {
                            break;
                        }
                        if let Ok(mut bytes) = output.lock() {
                            bytes.extend_from_slice(&buffer[..count]);
                            let excess = bytes.len().saturating_sub(16384);
                            bytes.drain(..excess);
                        }
                    }
                });
                Ok(Self {
                    child,
                    job: Some(job),
                    stderr,
                    stderr_reader: Some(reader),
                })
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
    #[cfg(test)]
    pub fn job_for_test_close(&mut self) {
        self.job.take();
    }
    pub fn stop(&mut self) {
        self.job.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
    pub fn diagnostics(&self) -> String {
        self.stderr
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) fn capture(
    command: &mut Command,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>), String> {
    let mut process = Process::spawn(command.stdout(Stdio::piped()))?;
    let output = process
        .child
        .stdout
        .take()
        .ok_or("Soloist output unavailable")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(MAX_OUTPUT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read Soloist response")?;
        if bytes.len() > MAX_OUTPUT {
            Err("Soloist response exceeded its size limit")
        } else {
            Ok(bytes)
        }
    });
    let started = Instant::now();
    let status = loop {
        match process.child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            Ok(None) => break Err("Soloist request timed out"),
            Err(_) => break Err("Could not inspect Soloist process"),
        }
    };
    // Close the job before joining the reader, including any descendants holding stdout.
    process.stop();
    let bytes = reader
        .join()
        .map_err(|_| "Soloist response reader stopped")?
        .map_err(str::to_owned)?;
    Ok((status.map_err(str::to_owned)?, bytes))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_bounds_output_and_times_out_hung_process() {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::Write('ok')",
        ]);
        let (status, output) = capture(&mut command, Duration::from_secs(10)).unwrap();
        assert!(status.success());
        assert_eq!(output, b"ok");
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ]);
        let start = Instant::now();
        assert!(capture(&mut command, Duration::from_millis(150))
            .unwrap_err()
            .contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(5));
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::Write(('x' * 2200000))",
        ]);
        assert!(capture(&mut command, Duration::from_secs(10))
            .unwrap_err()
            .contains("size limit"));
    }
}
