// --------------------------------------------------------------------- / tittu
// wallbash
// a minimal wallpaper engine for HyDE
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
            let cmd = format!("set {}", args[2]);
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

