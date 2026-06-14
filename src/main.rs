// --------------------------------------------------------------------- / tittu
// wallbash
// a fast and minimal wallpaper engine for HyDE
//


// --------------------------------------------------------------------- / imports

pub mod wallbash;
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
    eprintln!(r"::Usage
    wallbash start                  |  Start the wallpaper daemon
    wallbash set /path/to/file.img  |  Set wallpaper (auto start daemon)
    wallbash stop                   |  Stop the daemon
    wallbash status                 |  Show daemon status

::Options
    wallbash set [option] <value>
        -m, --mode <mode>           | Scaling mode (cover, fit, original)
        -a, --anchor <1-9>          | Anchor point (1=top-left ... 9=bottom-right)
        -w, --wall <file>           | Wallpaper file /path/to/file.img");
}

fn send_command(cmd: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    writeln!(stream, "{}", cmd)?;
    Ok(())
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

fn parse_args(args: &[String]) -> (String, String, f32, f32) {

    // wallpaper – mandatory
    let wall = args.iter().position(|a| a == "--wall" || a == "-w")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| {
            args.iter().skip(2).find(|a| !a.starts_with('-')).cloned()
        })
        .unwrap_or_else(|| {
            eprintln!("Missing wallpaper (use --wall <path> or bare path)");
            print_usage();
            std::process::exit(1);
        });

    // mode – default "cover"
    let mode = args.iter().position(|a| a == "--mode" || a == "-m")
        .and_then(|i| args.get(i + 1))
        .filter(|s| matches!(s.as_str(), "cover" | "fit" | "original"))
        .map(|s| s.clone())
        .unwrap_or_else(|| "cover".into());

    // anchor – default "center"
    let anchor_num = args.iter().position(|a| a == "--anchor" || a == "-a")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|&n| (1..10).contains(&n))
        .unwrap_or(5);
    let (ax, ay) = match anchor_num {
        1 => (0.0, 0.0),
        2 => (0.5, 0.0),
        3 => (1.0, 0.0),
        4 => (0.0, 0.5),
        5 => (0.5, 0.5),
        6 => (1.0, 0.5),
        7 => (0.0, 1.0),
        8 => (0.5, 1.0),
        9 => (1.0, 1.0),
        _ => (0.5, 0.5),
    };

    (wall, mode, ax, ay)
}


// --------------------------------------------------------------------- / main

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("start") => {
            if check_daemon() {
                eprintln!("Daemon is already running.");
                return;
            }
            if let Err(e) = wallbash::run(SOCKET_PATH) {
                eprintln!("Failed to start daemon {}", e);
            }
        }
        Some("set") => {
            let (wall, mode, ax, ay) = parse_args(&args);
            let cmd = format!("set{}\x01{}\x01{}\x01{}", mode, ax, ay, wall);
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
            if let Err(e) = send_command(&cmd) {
                eprintln!("Failed to set wallpaper {}. Is the daemon running?", e);
            }
        }
        Some("stop") => {
            if let Err(e) = send_command("stop") {
                eprintln!("Failed to stop daemon {}. Is it running?", e);
            }
        }
        Some("status") => {
            if check_daemon() {
                println!("Daemon is running.");
            } else {
                println!("Daemon is not running.");
            }
        }
        _ => print_usage()
    }
}

