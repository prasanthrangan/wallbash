// --------------------------------------------------------------------- / tittu
// wallbash
// a fast and minimal wallpaper engine for HyDE
//


// --------------------------------------------------------------------- / imports

pub mod wallbashed;
pub mod wayland;
pub mod vulkan;
use std::{
    env, io::Write,
    os::unix::net::UnixStream,
    process::Command,
    thread::sleep,
    time::Duration,
};
const SOCKET_PATH: &str = "/tmp/wallbash.sock";
const LOG_FILE: &str = "/tmp/wallbash.log";


// --------------------------------------------------------------------- / funtions

fn print_usage() {
    eprintln!("[Usage]");
    eprintln!("  wallbash start                  |  Start the wallpaper daemon");
    eprintln!("  wallbash set /path/to/file.img  |  Set the wallpaper");
    eprintln!("  wallbash stop                   |  Stop the daemon");
    eprintln!("  wallbash status                 |  Show daemon status");
}

fn check_daemon() -> bool {
    UnixStream::connect(SOCKET_PATH).is_ok()
}

fn wait_loop() -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..100 {
        if check_daemon() {
            return Ok(());
        }
        sleep(Duration::from_millis(100));
    }
    Err("Waiting for daemon...".into())
}

fn send_command(cmd: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    writeln!(stream, "{}", cmd)?;
    Ok(())
}

fn parse_mode(args: &[String]) -> &str {
    let i = args.iter().position(|a| a == "--mode" || a == "-m");
    match i.and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
        Some("fit")      => "fit",
        Some("original") => "original",
        _                => "cover",
    }
}

fn parse_anchor(args: &[String]) -> (f32, f32) {
    let i = args.iter().position(|a| a == "--anchor" || a == "-a");
    let num = i
        .and_then(|i| args.get(i + 1))
        .and_then(|val| val.parse::<u8>().ok());
    match num {
        Some(1) => (0.0, 0.0),
        Some(2) => (0.5, 0.0),
        Some(3) => (1.0, 0.0),
        Some(4) => (0.0, 0.5),
        Some(5) => (0.5, 0.5),
        Some(6) => (1.0, 0.5),
        Some(7) => (0.0, 1.0),
        Some(8) => (0.5, 1.0),
        Some(9) => (1.0, 1.0),
        _ => (0.5, 0.5),
    }
}


// --------------------------------------------------------------------- / main

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "start" => {
            if check_daemon() {
                eprintln!("Daemon is already running.");
                return;
            }
            if let Err(e) = wallbashed::run(SOCKET_PATH) {
                eprintln!("Failed to start daemon {}", e);
            }
        }
        "set" => {
            if args.len() < 3 {
                eprintln!("Missing image path.");
                return;
            }
            if !check_daemon() {
                println!("Starting daemon");
                let log_file = std::fs::File::create(LOG_FILE).expect("Cannot create log");
                let mut child = Command::new(env::current_exe().unwrap())
                    .arg("start")
                    .stdout(log_file.try_clone().unwrap())
                    .stderr(log_file)
                    .spawn()
                    .expect("Failed to start daemon");
                if let Err(e) = wait_loop() {
                    eprintln!("Error {}", e);
                    let _ = child.kill();
                    return;
                }
            }
            let mode = parse_mode(&args);
            let (anchor_h, anchor_v) = parse_anchor(&args);
            let cmd = format!("set{}\x01{}\x01{}\x01{}", mode, anchor_h, anchor_v, args[2]);
            if let Err(e) = send_command(&cmd) {
                eprintln!("Failed to set wallpaper {}. Is the daemon running?", e);
            }
        }
        "stop" => {
            if let Err(e) = send_command("stop") {
                eprintln!("Failed to stop daemon {}. Is it running?", e);
            }
        }
        "status" => {
            if check_daemon() {
                println!("Daemon is running.");
            } else {
                println!("Daemon is not running.");
            }
        }
        _ => print_usage(),
    }
}

