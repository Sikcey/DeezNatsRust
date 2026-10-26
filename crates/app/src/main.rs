#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
mod appearance;
#[cfg(windows)]
mod audio;
#[cfg(windows)]
pub(crate) mod connect;
#[cfg(windows)]
mod catalog;
#[cfg(windows)]
mod downloads;
#[cfg(windows)]
mod drawing;
#[cfg(windows)]
mod engine;
#[cfg(windows)]
mod integration;

#[cfg(windows)]
mod lyrics;
#[cfg(windows)]
mod session;
#[cfg(windows)]
mod storage;
#[cfg(windows)]
mod deezer_webview;

#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod visualizer;
#[cfg(windows)]
mod window;

fn main() {
    #[cfg(windows)]
    {
        let result = if std::env::args().any(|a| a == "--integration-test") {
            integration::run()
        } else if std::env::args().any(|a| a == "--deezer-playback-test") {
            audio::deezer_playback_test()
        } else if std::env::args().any(|a| a == "--engine-smoke-test") {
            audio::engine_smoke_test()
        } else {
            window::run()
        };
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    println!("This native prototype requires Windows.");
}
