// --------------------------------------------------------------------- / tittu
// wallbashed
// a daemon module for HyDE
//


// --------------------------------------------------------------------- / imports

use crate::{vulkan, wayland};
use std::{
    os::unix::net::{UnixListener, UnixStream},
    io::{BufRead, BufReader},
    sync::mpsc,
};


// --------------------------------------------------------------------- / datatypes

enum Command {
    Set(String),
    Stop,
}


// --------------------------------------------------------------------- / socket

fn start_ipc(socket_path: &str) -> Result<mpsc::Receiver<String>, Box<dyn std::error::Error>> {

    // Remove any stale socket file from a previous run
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    let (tx, rx) = mpsc::channel::<String>();
    println!("[ipc] listening: {}", socket_path);

    // start listener thread
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        if let Ok(path) = line {
                            let path = path.trim().to_string();
                            if !path.is_empty() {
                                if tx.send(path).is_err() {
                                    return; // main thread has dropped the receiver
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ipc] accept error: {}", e);
                    break;
                }
            }
        }
    });

    Ok(rx)
}


// --------------------------------------------------------------------- / wallpaper

fn set_wallpaper(
    path: &str,
    vk_core: &vulkan::VulkanCore,
    vk_surfchain: &vulkan::VulkanSurfchain,
    layer_width: u32,
    layer_height: u32,
    wallpaper: &mut Option<vulkan::VulkanTexture>,
) -> Result<(), Box<dyn std::error::Error>> {

    // load the wallpaper
    let img = image::open(path)?;
    let pixel_format = img.to_rgba8();
    let pixel_bytes = pixel_format.into_raw();

    // call the vulkan pipeline
    let texture = vulkan::vulkan_pipeline(
        &vk_core.instance,
        &vk_core.device,
        vk_core.physical_device,
        vk_core.graphics_queue,
        vk_core.graphics_family_index,
        &pixel_bytes,
        img.width(),
        img.height(),
    )?;

    // drop the old texture resources (if any)
    if let Some(old_tex) = wallpaper.take() {
        unsafe {
            vk_core.device.destroy_image_view(old_tex._view, None);
            vk_core.device.destroy_image(old_tex.image, None);
            vk_core.device.free_memory(old_tex._memory, None);
        }
    }
    *wallpaper = Some(texture);

    // draw the wallpaper
    vulkan::draw_wallpaper(
        &vk_core.instance,
        &vk_core.device,
        vk_core.graphics_queue,
        vk_core.graphics_family_index,
        vk_surfchain.swapchain,
        &vk_surfchain.swapchain_images,
        wallpaper.as_ref().unwrap().image,
        img.width(),
        img.height(),
        layer_width,
        layer_height,
    )?;

    Ok(())
}


// --------------------------------------------------------------------- / daemon

pub fn run(socket_path: &str) -> Result<(), Box<dyn std::error::Error>> {

    // init listener
    if UnixStream::connect(socket_path).is_ok() {
        return Err("Daemon is already running.".into());
    }
    let _ = std::fs::remove_file(socket_path);
    let rx = start_ipc(socket_path)?;

    // init wayland
    let mut wl_core = wayland::wayland_core()?;

    // init vulkan
    let vk_core = vulkan::vulkan_core()?;
    let vk_surfchain = vulkan::vulkan_surfchain(
        &vk_core.entry,
        &vk_core.instance,
        vk_core.physical_device,
        &vk_core.device,
        &wl_core.display,
        &wl_core.surface,
        wl_core.state.layer_width,
        wl_core.state.layer_height,
    )?;

    // blank surface until a command arrives
    let mut wallpaper: Option<vulkan::VulkanTexture> = None;
    println!("[wallbash] ready, press Ctrl+C to quit.");

    // main event loop
    let mut running = true;
    while running {
        wl_core.event.dispatch_pending(&mut wl_core.state)?;

        // check for commands from the IPC
        if let Ok(raw) = rx.try_recv() {
            let raw = raw.trim().to_string();
            println!("[wallbash] loading {}", raw);

            let cmd = if raw == "stop" {
                Command::Stop
            } else if raw.starts_with("set ") {
                let path = raw[4..].trim().to_string();
                Command::Set(path)
            } else {
                Command::Set(raw)
            };

            match cmd {
                Command::Set(path) => {
                    match set_wallpaper(
                        &path,
                        &vk_core,
                        &vk_surfchain,
                        wl_core.state.layer_width,
                        wl_core.state.layer_height,
                        &mut wallpaper,
                    ) {
                        Ok(()) => println!("[wallbash] wallpaper set..."),
                        Err(e) => eprintln!("[wallbash] error {}", e),
                    }
                }
                Command::Stop => {
                    println!("[wallbash] stopping daemon...");
                    running = false;
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    // clean shutdowon 
    if let Some(tex) = wallpaper.take() {
        unsafe {
            vk_core.device.destroy_image_view(tex._view, None);
            vk_core.device.destroy_image(tex.image, None);
            vk_core.device.free_memory(tex._memory, None);
        }
    }
    unsafe {
        vk_core.device.destroy_device(None);
        vk_core.instance.destroy_instance(None);
    }

    println!("[wallbash] daemon stopped.");
    Ok(())
}

