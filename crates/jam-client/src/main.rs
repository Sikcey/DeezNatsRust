#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod glyphs;
mod model;
#[cfg(windows)]
mod network;
#[cfg(windows)]
mod pages;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod sound;
#[cfg(windows)]
mod ui;
fn main() {
    #[cfg(windows)]
    if let Err(error) = app::run() {
        if let Some(path) = std::env::args().skip_while(|a| a != "--error-file").nth(1) {
            let _ = std::fs::write(path, &error);
        } else {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    model::wide(&error).as_ptr(),
                    model::wide("DeezNats Connect").as_ptr(),
                    windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                );
            }
        }
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    println!("DeezNats Connect requires Windows.");
}
