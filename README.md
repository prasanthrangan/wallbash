###### *<div align="right"><sub>// design by t2</sub></div>*
<p align="center"><img src="https://github.com/prasanthrangan/hyprdots/blob/3c8b0dfb5e7f8e41a67b80463513f10d57cab1a4/Source/assets/hyde.png" width="100"></p>

# wa*ll*bash
A fast and minimal wallpaper engine for HyDE

Use `wallbash` as a core component of your Wayland desktop environment — set wallpapers, generate color palettes, and dynamically theme your desktop.


## Features

- Vulkan-powered GPU acceleration for smooth performance
- Color palette generation for dynamic theming (WIP)
- Fluid transitions and animations (WIP)
- Multi-monitor support (WIP)
- Scale and anchor the image to your liking
- Dynamic blur fill for mismatched aspect ratios to eliminate black bars


## Build

```bash
git clone https://github.com/prasanthrangan/wallbash
cd wallbash
cargo build --release
sudo cp target/release/wallbash /usr/local/bin/
```


## Usage

```bash
wallbash start                  #  Start the wallpaper daemon
wallbash set /path/to/file.img  #  Set wallpaper (auto start daemon)
wallbash stop                   #  Stop the daemon
wallbash status                 #  Show daemon status

# options for "set"
wallbash set [option] <value>
    -m, --mode <mode>           # Scaling mode (cover, fit, original)
    -a, --anchor <1-9>          # Anchor point (1=top-left ... 9=bottom-right)
    -w, --wall <file>           # Wallpaper file /path/to/file.img
```

The Rust binary compiles to a single executable, `wallbash`. It acts as both a client and a daemon:
- `wallbash start` Launches the daemon (background process). The daemon initializes the Wayland and Vulkan subsystems and listens for commands on a Unix socket.
- `wallbash set` Sends a command to load and display the image. If the daemon is not running, it automatically starts it and waits for it to be ready before sending the command.
- `wallbash stop` Terminate the daemon.
- `wallbash status` Query the daemon status.


## Architecture

```
src/
├── main.rs
├── wallbash.rs
├── wayland.rs
├── vulkan.rs
└── filters.rs
```

The core project is structured in simple modules:
- `main.rs` Entry point of the binary. Works as a CLI tool to parse arguments and handle the daemon.
- `wallbash.rs` The core daemon module. It manages the IPC listener, handles incoming commands, and orchestrates the wallpaper loading and rendering process.
- `wayland.rs` Handles the Wayland integration. It creates a Wayland surface, binds to the layer shell protocol, and sets up the layer surface for the wallpaper.
- `vulkan.rs` Manages the Vulkan rendering pipeline. It initializes the Vulkan instance, selects a physical device (preferring a discrete GPU), creates a swapchain, and renders the wallpaper image.
- `filters.rs` – Implements image filters and post‑processing effects, including dynamic background blur, scaling algorithms, and other visual transformations.

###### *<div align="right"><sub>// HyDE</sub></div>*
<p align="center"><img src="https://github.com/prasanthrangan/hyprdots/blob/3c8b0dfb5e7f8e41a67b80463513f10d57cab1a4/Source/assets/Arch.svg" width="100"></p>
