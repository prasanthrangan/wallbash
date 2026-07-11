// --------------------------------------------------------------------- / tittu
// wallbash
// an inter process communication module for HyDE
//


// --------------------------------------------------------------------- / imports

use std::{
    os::unix::net::{UnixListener, UnixStream},
    io::{BufRead, BufReader},
    sync::mpsc,
};


// --------------------------------------------------------------------- / datatypes

#[derive(Debug, PartialEq)]
pub enum ScalingMode {
    Cover,
    Fit,
    Original,
}

#[derive(Debug, PartialEq)]
pub enum PaletteMode {
    Auto,
    Dark,
    Light,
    Skip,
}

pub struct IpcMessage {
    pub cmd: String,
    pub stream: UnixStream,
}

pub enum Command {
    Stop,
    Set {
        palette: PaletteMode,
        bezier: String,
        scale: ScalingMode,
        anchor_x: f32,
        anchor_y: f32,
        path: String,
    }
}


// --------------------------------------------------------------------- / parser

impl ScalingMode {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "fit" => Self::Fit,
            "original" => Self::Original,
            _ => Self::Cover,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Fit => "fit",
            Self::Original => "original",
        }
    }
}

impl PaletteMode {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "auto" => Self::Auto,
            "dark" => Self::Dark,
            "light" => Self::Light,
            _ => Self::Skip,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Skip => "skip",
        }
    }
}

impl Command {
    pub fn parse_raw(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw == "stop" { return Ok(Command::Stop); }
        if raw.starts_with("set") {
            let payload = &raw[3..];
            let mut parts = payload.splitn(6, '\x01');
            let palette = PaletteMode::from_str(parts.next().ok_or("missing palette")?);
            let bezier = parts.next().ok_or("missing bezier")?.to_string();
            let scale = ScalingMode::from_str(parts.next().ok_or("missing mode")?);
            let anchor_x = parts.next().ok_or("missing anchor_x")?.parse().map_err(|_| "invalid anchor_x")?;
            let anchor_y = parts.next().ok_or("missing anchor_y")?.parse().map_err(|_| "invalid anchor_y")?;
            let path = parts.next().ok_or("missing path")?.to_string();
            return Ok(Command::Set { palette, bezier, scale, anchor_x, anchor_y, path});
        }
        Err(format!("unknown internal command: {}", raw))
    }
}


// --------------------------------------------------------------------- / listener

pub fn start_ipc(socket: &str) -> Result<mpsc::Receiver<IpcMessage>, Box<dyn std::error::Error>> {

    // remove any stale socket from previous run
    let _ = std::fs::remove_file(socket);

    // create listener and channel
    let listener = UnixListener::bind(socket)?;
    let (tx, rx) = mpsc::channel::<IpcMessage>();
    println!("[ipc] listening: {}", socket);

    // start listener thread
    std::thread::spawn(move || { for stream in listener.incoming() {
        match stream {

            // read and send the message
            Ok(stream) => {
                let reader = BufReader::new(&stream);
                if let Some(Ok(cmd)) = reader.lines().next() {
                    let cmd = cmd.trim().to_string();
                    if !cmd.is_empty() {
                        if tx.send(IpcMessage { cmd, stream }).is_err() { return; }
                    }
                }
            }
            Err(e) => {
                eprintln!("[ipc] accept error: {}", e);
                break;
            }
        }
    }});
    Ok(rx)
}

