###### *<div align="right"><sub>// design by t2</sub></div>*
<p align="center"><img src="https://github.com/prasanthrangan/hyprdots/blob/3c8b0dfb5e7f8e41a67b80463513f10d57cab1a4/Source/assets/hyde.png" width="100"></p>

# wa*ll*bash

A fast and minimal wallpaper engine for HyDE

Use `wallbash` as a core component of your Wayland desktop environment — set wallpapers, generate color palettes, and apply dynamic themes to your desktop.


## Features

- GPU acceleration powered by Vulkan for smooth, high‑performance rendering
- Built for seamless integration with your Wayland compositor
- Dynamic color palette generation based on Material Design with <kbd>auto</kbd>, <kbd>dark</kbd>, and <kbd>light</kbd> modes
- Scale the image to your liking using <kbd>cover</kbd>, <kbd>fit</kbd>, or <kbd>original</kbd> modes
- Precise anchor point positioning from <kbd>1</kbd> to <kbd>9</kbd> for fine‑tuned wallpaper placement
- Automatic background blur fill for mismatched aspect ratios, eliminating black bars
- Fluid transitions and animations*
- Multi-monitor support*
<div align="right"><body>*work in progress</body></div>


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
    -p, --palette <color>       # Generate color palette (auto, dark, light)
    -m, --mode <scale>          # Scaling mode (cover, fit, original)
    -a, --anchor <1-9>          # Anchor point (1=top-left ... 9=bottom-right)
    -w, --wall <file>           # Wallpaper file /path/to/file.img
```


## Architecture

Wallbash is a **single binary** that can run in two modes:

- **Client Mode:** Run <kbd>wallbash set --wall ...</kbd> to set a wallpaper. The command either:
  - Connects to an existing daemon and sends the command, or
  - Starts the daemon automatically if it's not already running, then sends the command.

- **Daemon Mode:** Run <kbd>wallbash start</kbd> to explicitly launch the daemon, or let it be started automatically on first use. The daemon:
  - Manages the Wayland surface and Vulkan rendering pipeline.
  - Listens for commands via a Unix socket (<kbd>/tmp/wallbash.sock</kbd>).
  - Persists in the background until stopped with <kbd>wallbash stop</kbd>.

The core **modules** are structured as:

- **main.rs** The CLI entry point. Parses command-line arguments and routes them to the appropriate handler (start, stop, set, status). It also manages the daemon lifecycle and socket communication.
- **wallbash.rs** The core daemon logic. Manages IPC server, incoming commands, and coordinates wayland and rendering pipeline. It acts as the central controller for other modules.
- **wayland.rs** Handles the Wayland connection. Creates and manages the surface, sets up the output, and window events. This is the interface between wallbash and your Wayland compositor.
- **vulkan.rs** Handles the GPU initialization, texture creation, shader compilation, and draws the wallpaper surface using Vulkan. This provides the image rendering pipeline.
- **filters.rs** Applies Vulkan compute shaders which currently implements a blur effect for the background. The module is designed to be extensible for additional filters in future.
- **colors.rs** Extracts the dominant color from the wallpaper using k-means clustering, converts colors and generates a color palette. It's then deployed to your config files based on templates.


## Theming

Wallbash generates a color palette from your wallpaper. You can use these colors to dynamically theme your entire desktop environment.
For detailed guides, usage, and application specific examples, check out the [wiki](https://github.com/prasanthrangan/wallbash/wiki/Theming).

###### *<div align="right"><sub>// HyDE</sub></div>*
<p align="center"><img src="https://github.com/prasanthrangan/hyprdots/blob/3c8b0dfb5e7f8e41a67b80463513f10d57cab1a4/Source/assets/Arch.svg" width="100"></p>
