//! Offline Soloist process fixture. Never connects to Spotify or plays audio.
use std::{
    io::{self, Write},
    path::PathBuf,
    thread,
    time::Duration,
};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--version") {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        println!("Spotify Soloist fixture 1.2.3\nBuild timestamp: {now}\nPlatform: windows\nArchitecture: x86_64");
        return;
    }
    let value = |key: &str| {
        args.iter()
            .position(|s| s == key)
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_default()
    };
    let data = PathBuf::from(value("--data-dir"));
    if args.get(1).is_some_and(|s| s == "ctl") {
        if args.iter().any(|a| a == "trace") {
            let pairing = data.join("fixture-pairing");
            if pairing.exists() && std::fs::read_to_string(&pairing).unwrap() == "waiting" {
                println!("123 {{\"type\":\"auth_state\",\"logged_in\":false,\"is_active\":false,\"device_name\":\"Fixture\"}}");
                io::stdout().flush().unwrap();
                while std::fs::read_to_string(&pairing).unwrap() == "waiting" {
                    thread::sleep(Duration::from_millis(20));
                }
            }
            println!("123 {{\"type\":\"auth_state\",\"logged_in\":true,\"is_active\":true,\"device_name\":\"Fixture\"}}");
            println!("124 {{\"type\":\"playback_state\",\"status\":\"paused\",\"item\":null,\"context\":null,\"position\":{{\"position_ms\":1234,\"timestamp_ms\":0,\"speed\":0.0}},\"volume\":42,\"options\":{{\"shuffle\":false,\"repeat\":\"off\"}},\"is_active\":true}}");
            io::stdout().flush().unwrap();
        } else {
            if args.iter().any(|a| a == "volume") {
                std::process::exit(3);
            }
            if args
                .iter()
                .any(|a| a == "status" || a == "now" || a == "queue")
            {
                println!("{{\"fixture\":true}}");
            }
            return;
        }
    } else {
        if value("--api-key") == "pair-later" && !data.join("fixture-pairing").exists() {
            std::fs::write(data.join("fixture-pairing"), "waiting").unwrap();
        }
        if value("--api-key") == "expired" {
            std::process::exit(10);
        }
        if value("--api-key") == "invalid-key-fixture-secret" {
            eprintln!("Invalid API key: {}", value("--api-key"));
            std::process::exit(1);
        }
        if value("--api-key") != "no-api" {
            std::fs::write(data.join("soloist.pid"), std::process::id().to_string()).unwrap();
            std::fs::write(data.join("ws.addr"), "127.0.0.1").unwrap();
            std::fs::write(data.join("ws.port"), "12345").unwrap();
        }
        if value("--api-key") == "late-invalid-key-fixture-secret" {
            while !data.join("reject-key").exists() {
                thread::sleep(Duration::from_millis(10));
            }
            eprintln!("API key rejected: {}", value("--api-key"));
        }
    }
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
