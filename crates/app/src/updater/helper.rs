use deeznats_updater::ApplyPlan;
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0},
    System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
};

pub fn spawn(stage: &Path) -> Result<Child, String> {
    let helper = stage.join("update-helper.exe");
    fs::copy(std::env::current_exe().map_err(|e| e.to_string())?, &helper)
        .map_err(|e| e.to_string())?;
    let mut child = Command::new(helper)
        .arg("--apply-update")
        .arg(stage)
        .current_dir(stage)
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if stage.join("ready").is_file() {
            return Ok(child);
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err(fs::read_to_string(stage.join("result.txt"))
                .unwrap_or_else(|_| "Could not start the update helper.".into()));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("Update helper did not become ready. The application was left running.".into())
}

pub fn run(stage: &Path) -> Result<(), String> {
    super::validate_stage(stage)?;
    let plan: ApplyPlan =
        serde_json::from_slice(&fs::read(stage.join("plan.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut parent_exited = false;
    let operation = (|| -> Result<(), String> {
        plan.validate(stage)?;
        let parent = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, plan.parent_pid) };
        if parent.is_null() {
            return Err("The original application is no longer running.".into());
        }
        struct Process(windows_sys::Win32::Foundation::HANDLE);
        impl Drop for Process {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let parent = Process(parent);
        fs::write(stage.join("ready"), b"ready").map_err(|e| e.to_string())?;
        if unsafe { WaitForSingleObject(parent.0, 60_000) } != WAIT_OBJECT_0 {
            return Err("The application did not close. No files were changed.".into());
        }
        parent_exited = true;
        deeznats_updater::commit(&plan, stage)?;
        update_installed_version(&plan);
        Ok(())
    })();
    let message = operation
        .as_ref()
        .map(|_| "Update installed.".to_owned())
        .unwrap_or_else(|error| error.clone());
    let _ = fs::write(stage.join("result.txt"), &message);
    // Only restart after the parent exited, including after a rolled-back install.
    if parent_exited {
        let executable = plan.directory.join("deeznats-prototype.exe");
        Command::new(executable)
            .arg("--update-result")
            .arg(stage)
            .current_dir(&plan.directory)
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    operation
}

fn update_installed_version(plan: &ApplyPlan) {
    use windows_sys::Win32::System::Registry::*;
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    unsafe {
        let mut value = [0u16; 32768];
        let mut bytes = (value.len() * 2) as u32;
        if RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\DeezNats").as_ptr(),
            wide("InstallDir").as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut bytes,
        ) != 0
        {
            return;
        }
        let length = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
        let registered = PathBuf::from(String::from_utf16_lossy(&value[..length]));
        if registered.canonicalize().ok() != plan.directory.canonicalize().ok() {
            return;
        }
        let version = wide(&plan.version);
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\DeezNats").as_ptr(),
            wide("DisplayVersion").as_ptr(),
            REG_SZ,
            version.as_ptr().cast(),
            (version.len() * 2) as u32,
        );
    }
}

pub fn startup_result() -> Option<String> {
    let args: Vec<_> = std::env::args_os().collect();
    let index = args.iter().position(|arg| arg == "--update-result")?;
    let stage = PathBuf::from(args.get(index + 1)?);
    super::validate_stage(&stage).ok()?;
    let message = fs::read_to_string(stage.join("result.txt")).ok()?;
    // The helper may still be exiting. Cleanup is limited to this exact private
    // staging directory and happens off the UI thread.
    std::thread::spawn(move || {
        for _ in 0..20 {
            if super::clean_stage(&stage).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    Some(message)
}
