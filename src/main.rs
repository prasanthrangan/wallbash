// --------------------------------------------------------------------- / tittu
// wallbash
// a minimal wallpaper engine for HyDE
//


// --------------------------------------------------------------------- / imports

pub mod wallbashed;
pub mod wayland;
pub mod vulkan;
use std::env;
use std::io::Write;
use std::os::unix::net::UnixStream;
const SOCKET_PATH: &str = "/tmp/wallbash.sock";


// --------------------------------------------------------------------- / funtioon

fn send_command(cmd: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    writeln!(stream, "{}", cmd)?;
    Ok(())
}

fn print_usage() {
    eprintln!("[Usage]");
    eprintln!("  wallbash start              Start the wallpaper daemon");
    eprintln!("  wallbash set <image_path>   Set the wallpaper");
    eprintln!("  wallbash stop               Stop the daemon");
    eprintln!("  wallbash status             Show daemon status");
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
            // Check if already running
            if UnixStream::connect(SOCKET_PATH).is_ok() {
                eprintln!("Daemon is already running.");
                return;
            }
            // Start the daemon (this will block until stopped)
            if let Err(e) = wallbashed::run(SOCKET_PATH) {
                eprintln!("Failed to start daemon: {}", e);
            }
        }
        "set" => {
            if args.len() < 3 {
                eprintln!("Missing image path.");
                return;
            }
            let cmd = format!("set {}", args[2]);
            if let Err(e) = send_command(&cmd) {
                eprintln!("Failed to set wallpaper: {}. Is the daemon running?", e);
            }
        }
        "stop" => {
            if let Err(e) = send_command("stop") {
                eprintln!("Failed to stop daemon: {}. Is it running?", e);
            }
        }
        "status" => {
            match UnixStream::connect(SOCKET_PATH) {
                Ok(_) => println!("Daemon is running."),
                Err(_) => println!("Daemon is not running."),
            }
        }
        _ => print_usage(),
    }
}

