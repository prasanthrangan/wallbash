// --------------------------------------------------------------------- / tittu
// wallbash
// a fast and minimal wallpaper engine for HyDE
//


// --------------------------------------------------------------------- / imports

pub mod wallbash;
pub mod ipc;
pub mod wayland;
pub mod vulkan;
pub mod filters;
pub mod transitions;
pub mod colors;
use std::{
    fs, env, error, process,
    os::unix::net::UnixStream,
    io::{Write,Read},
    path::PathBuf,
    time::Duration,
    thread::sleep,
};


// --------------------------------------------------------------------- / datatypes

const SOCKET: &str = "/tmp/wallbash.sock";

struct CachedState {
    wall: String,
    palette: String,
    bezier: String,
    mode: String,
    anchor_x: f32,
    anchor_y: f32,
}


// --------------------------------------------------------------------- / help

fn print_usage() {
    eprintln!(r"
    ::Usage
        wallbash start                  |  Start the wallpaper daemon
        wallbash set /path/to/file.img  |  Set wallpaper (auto start daemon)
        wallbash stop                   |  Stop the daemon
        wallbash status                 |  Show daemon status

    ::Options
        wallbash set [option] <value>
            -w, --wall <file>           |  Wallpaper file '/path/to/file.img'
            -c, --cycle <signed int>    |  Cycle in current folder (+1, -2, etc.)
            -p, --palette <color>       |  Generate color palette (auto, dark, light)
            -b, --bezier <curve>        |  Custom animation curve (ex. '0.64,0.56,0.17,0.84')
            -m, --mode <scale>          |  Scaling mode (cover, fit, original)
            -a, --anchor <1-9>          |  Anchor point (1=top-left ... 9=bottom-right)
"   );
}


// --------------------------------------------------------------------- / sock

fn send_command(cmd: &str) -> Result<(), Box<dyn error::Error>> {
    let mut stream = UnixStream::connect(SOCKET)?;
    writeln!(stream, "{}", cmd)?;
    if cmd.starts_with("set") {
        let mut buf = [0u8; 1];
        stream.read_exact(&mut buf)?; // hey daemon, are you done?
    }
    Ok(())
}

fn check_daemon() -> bool {
    UnixStream::connect(SOCKET).is_ok()
}

fn wait_loop() -> Result<(), Box<dyn error::Error>> {
    for _ in 0..100 {
        if check_daemon() {
            return Ok(());
        }
        sleep(Duration::from_millis(100));
    }
    Err("waiting for daemon...".into())
}


// --------------------------------------------------------------------- / log

fn cache_dir() -> PathBuf {
    let base = env::var("XDG_CACHE_HOME").ok().or_else(|| env::var("HOME")
    .ok().map(|home| format!("{}/.cache", home))).unwrap_or_default();
    let dir = PathBuf::from(base).join("wallbash");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn cache_log() -> fs::File {
    let path = cache_dir().join("wallbash.log");
    fs::File::create(&path).expect("cannot create log")
}


// --------------------------------------------------------------------- / state

impl CachedState {
    fn default() -> Self {
        Self {
            wall: String::new(),
            palette: "skip".into(),
            bezier: "0.64,0.56,0.17,0.84".into(),
            mode: "cover".into(),
            anchor_x: 0.5,
            anchor_y: 0.5,
        }
    }
}

fn cache_state() -> PathBuf {
    cache_dir().join("state")
}

fn save_state(state: &CachedState) {
    let resolved = fs::canonicalize(&state.wall)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| state.wall.clone());
    let content = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        resolved, state.palette, state.bezier, state.mode, state.anchor_x, state.anchor_y
    );
    let _ = fs::write(cache_state(), content);
}

fn load_state() -> CachedState {
    let content = match fs::read_to_string(cache_state()) {
        Ok(c) => c,
        Err(_) => return CachedState::default(), 
    };
    let default = CachedState::default();
    let mut lines = content.lines();
    let wall = lines.next().map(|s| s.to_string()).unwrap_or(default.wall);
    let palette = lines.next().map(|s| s.to_string()).unwrap_or(default.palette);
    let bezier = lines.next().map(|s| s.to_string()).unwrap_or(default.bezier);
    let mode = lines.next().map(|s| s.to_string()).unwrap_or(default.mode);
    let anchor_x: f32 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(default.anchor_x);
    let anchor_y: f32 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(default.anchor_y);
    CachedState { wall, palette, bezier, mode, anchor_x, anchor_y }
}


// --------------------------------------------------------------------- / cycle

fn scan_images(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
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

fn cycle_wallpaper(current: &str, cycle: i32) -> String {

    // resolve parent dir
    let dir = std::path::Path::new(&current).parent().unwrap_or_else(|| {
        eprintln!("[error] cached directory not found");
        process::exit(1);
    });

    // scan parent dir
    let images = scan_images(dir);
    if images.is_empty() {
        eprintln!("[error] no images found in {:?}", dir);
        process::exit(1);
    }

    // cycle logic
    let index = images.iter().position(|p| p.to_string_lossy() == current).unwrap_or(0);
    let count = images.len() as i32;
    let index = ((index as i32 + cycle) % count + count) % count;
    images[index as usize].to_string_lossy().to_string()
}


// --------------------------------------------------------------------- / args

fn parse_args(args: &[String]) -> CachedState {

    // get previous state
    let mut state = load_state();
    let default = CachedState::default();

    // wallpaper – default "cached"
    let wall = args.iter().position(|a| a == "--wall" || a == "-w")
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
        });
    if let Some(wall) = wall {
        let resolved = fs::canonicalize(&wall)
            .map(|p| p.to_string_lossy().to_string()).unwrap_or(wall);
        state.wall = resolved;
    }

    // cycle wallpaper – default "0"
    let cycle: i32 = args.iter().position(|a| a == "--cycle" || a == "-c")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if cycle != 0 {
        if state.wall.is_empty() {
            eprintln!("[error] no cached wallpaper");
            process::exit(1);
        }
        state.wall = cycle_wallpaper(&state.wall, cycle);
    }
    if state.wall.is_empty() {
        eprintln!("[error] missing wallpaper (use --wall <path> or bare path)");
        print_usage();
        process::exit(1);
    }

    // color generation - default "skip" 
    if let Some(pos) = args.iter().position(|a| a == "--palette" || a == "-p") {
        let pal = args.get(pos + 1)
            .filter(|s| matches!(s.as_str(), "auto" | "dark" | "light"))
            .map(|s| s.clone()).unwrap_or_else(|| default.palette);
        state.palette = pal;
    }

    // bezier curve - default "linear" 
    if let Some(pos) = args.iter().position(|a| a == "--bezier" || a == "-b") {
        let val = args.get(pos + 1)
            .map(|s| s.clone()).unwrap_or_else(|| default.bezier);
        state.bezier = val;
    }

    // mode – default "cover"
    if let Some(pos) = args.iter().position(|a| a == "--mode" || a == "-m") {
        let m = args.get(pos + 1)
            .filter(|s| matches!(s.as_str(), "cover" | "fit" | "original"))
            .map(|s| s.clone()).unwrap_or_else(|| default.mode);
        state.mode = m;
    }

    // anchor – default "center"
    if let Some(pos) = args.iter().position(|a| a == "--anchor" || a == "-a") {
        let anchor = args.get(pos + 1)
            .and_then(|s| s.parse::<u8>().ok())
            .filter(|&n| (1..10).contains(&n));
        let (ax, ay) = match anchor {
            Some(1) => (0.0, 0.0),
            Some(2) => (0.5, 0.0),
            Some(3) => (1.0, 0.0),
            Some(4) => (0.0, 0.5),
            Some(5) => (0.5, 0.5),
            Some(6) => (1.0, 0.5),
            Some(7) => (0.0, 1.0),
            Some(8) => (0.5, 1.0),
            Some(9) => (1.0, 1.0),
            _       => (default.anchor_x, default.anchor_y),
        };
        state.anchor_x = ax;
        state.anchor_y = ay;
    }

    // save and return
    save_state(&state);
    state
}


// --------------------------------------------------------------------- / main

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(|s| s.as_str()) {

        // hey, do your job!
        Some("start") => {
            if check_daemon() {
                eprintln!("[wallbash] daemon is already running...");
                return;
            }
            if let Err(e) = wallbash::run(SOCKET) {
                eprintln!("[error] {}", e);
            }
        }

        // your wish is my command!
        Some("set") => {
            let state = parse_args(&args);
            let cmd = format!("set{}\x01{}\x01{}\x01{}\x01{}\x01{}",
                state.palette, state.bezier, state.mode, state.anchor_x, state.anchor_y, state.wall);

            // hey daemon, wake up!
            if !check_daemon() {
                println!("[wallbash] starting daemon...");
                let log = cache_log();
                let mut child = process::Command::new(env::current_exe().unwrap())
                    .arg("start").stdout(log.try_clone().unwrap()).stderr(log)
                    .spawn().expect("[error] failed to start daemon!");
                if let Err(e) = wait_loop() {
                    eprintln!("[error] {}", e);
                    let _ = child.kill();
                    return;
                }
            }
            if let Err(e) = send_command(&cmd) {
                eprintln!("[error] {}. Is it even running?", e);
            }
        }

        // stop it, enough!
        Some("stop") => {
            println!("[wallbash] goodbye...");
            if let Err(e) = send_command("stop") {
                eprintln!("[error] {}. Is it even running?", e);
            }
        }

        // hey, are you alive?
        Some("status") => {
            if check_daemon() {
                println!("[wallbash] :: Daemon is running");
            } else {
                println!("[wallbash] :: Daemon is not running");
            }
            let state = load_state();
            if state.wall.is_empty() {
                println!("[wallbash] :: No wallpaper cached yet");
            } else {
                println!("Wallpaper  :: {}", state.wall);
                println!("Palette    :: {}", state.palette);
                println!("bezier     :: {}", state.bezier);
                println!("Mode       :: {}", state.mode);
                println!("Anchor     :: ({:.1}, {:.1})", state.anchor_x, state.anchor_y);
            }
        }
        _ => print_usage()
    }
}

