// --------------------------------------------------------------------- / tittu
// wallbash
// a fast and minimal wallpaper engine for HyDE
//


// --------------------------------------------------------------------- / imports

pub mod wallbash;
pub mod wayland;
pub mod vulkan;
pub mod filters;
pub mod colors;

use std::{
    env, io::Write,
    os::unix::net::UnixStream,
    process::Command,
    thread::sleep,
    time::Duration,
    path::PathBuf,
};

const SOCKET_PATH: &str = "/tmp/wallbash.sock";
const LOG_FILE: &str = "/tmp/wallbash.log";


// --------------------------------------------------------------------- / sock

fn print_usage() {
    eprintln!(r"
    ::Usage
        wallbash start                  |  Start the wallpaper daemon
        wallbash set /path/to/file.img  |  Set wallpaper (auto start daemon)
        wallbash stop                   |  Stop the daemon
        wallbash status                 |  Show daemon status

    ::Options
        wallbash set [option] <value>
            -p, --palette <color>       |  Generate color palette (auto, dark, light)
            -c, --cycle <signed int>    |  Cycle in current folder (+1, -2, etc.)
            -m, --mode <scale>          |  Scaling mode (cover, fit, original)
            -a, --anchor <1-9>          |  Anchor point (1=top-left ... 9=bottom-right)
            -w, --wall <file>           |  Wallpaper file /path/to/file.img
"   );
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


// --------------------------------------------------------------------- / args

fn parse_args(args: &[String]) -> (String, i32, String, f32, f32) {

    // wallpaper – default "cached"
    args.iter().position(|a| a == "--wall" || a == "-w")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| {
            args.iter().skip(2).scan(false, |skip, a| {
                if *skip {
                    *skip = false;
                    Some(None)
                } else if a.starts_with('-') {
                    *skip = true;
                    Some(None)
                } else {
                    Some(Some(a.clone()))
                }
            }).flatten().last()
        }).map(|p| {
            save_cache(&p);
        }).unwrap_or_default();

    // color generation - default "skip" 
    let palette = args.iter().position(|a| a == "--palette" || a == "-p")
        .and_then(|i| args.get(i + 1))
        .filter(|s| matches!(s.as_str(), "auto" | "dark" | "light"))
        .map(|s| s.clone())
        .unwrap_or_else(|| "skip".into());

    // wallpaper cycle – default "0"
    let cycle: i32 = args.iter().position(|a| a == "--cycle" || a == "-c")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);

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

    (palette, cycle, mode, ax, ay)
}


// --------------------------------------------------------------------- / cache

fn cache_file() -> PathBuf {
    let cache = env::var("XDG_CACHE_HOME").ok().or_else(|| env::var("HOME")
        .ok().map(|home| format!("{}/.cache", home))).unwrap_or_default();
    PathBuf::from(cache).join("wallbash/state")
}

fn save_cache(path: &str) {
    let resolved = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());
    if let Some(parent) = cache_file().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(cache_file(), &resolved);
}

fn load_cache() -> Option<String> {
    std::fs::read_to_string(cache_file()).ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}


// --------------------------------------------------------------------- / cycle

fn scan_images(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|e| e.to_str())
                .map(|ext| matches!(ext.to_lowercase().as_str(), "jpg"|"jpeg"|"png"|"bmp"|"gif"|"webp"))
                .unwrap_or(false)
            }).collect(),
        Err(_) => return vec![],
    };
    files.sort();
    files
}

fn cycle_wallpaper(cycle: i32) -> String {

    // read cached image
    let current = load_cache().unwrap_or_else(|| {
        eprintln!("Cached wallpaper not found");
        std::process::exit(1);
    });

    // resolve parent dir
    let dir = std::path::Path::new(&current).parent().unwrap_or_else(|| {
        eprintln!("Cached directory not found");
        std::process::exit(1);
    });

    // scan parent dir
    let images = scan_images(dir);
    if images.is_empty() {
        eprintln!("No images found in {:?}", dir);
        std::process::exit(1);
    }

    // cycle logic
    let index = images.iter().position(|p| p.to_string_lossy() == current).unwrap_or(0);
    let count = images.len() as i32;
    let index = ((index as i32 + cycle) % count + count) % count;
    let wall = images[index as usize].to_string_lossy().to_string();
    save_cache(&wall);
    wall
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
            let (palette, cycle, mode, ax, ay) = parse_args(&args);
            let wall = cycle_wallpaper(cycle);
            let cmd = format!("set{}\x01{}\x01{}\x01{}\x01{}", palette, mode, ax, ay, wall);
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

