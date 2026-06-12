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


// --------------------------------------------------------------------- / listener

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
    anchor_x: f32,
    anchor_y: f32,
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
        vk_core.command_pool,
        vk_core.command_buffer,
        &pixel_bytes,
        img.width(),
        img.height(),
    )?;

    // drop the old texture resources (if any)
    if let Some(old_tex) = wallpaper.take() {
        unsafe {
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
        vk_core.command_pool,
        vk_core.command_buffer,
        vk_surfchain.swapchain,
        &vk_surfchain.swapchain_images,
        wallpaper.as_ref().unwrap().image,
        img.width(),
        img.height(),
        layer_width,
        layer_height,
        anchor_x,
        anchor_y,
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
    let mut vk_surfchain = vulkan::vulkan_surfchain(
        &vk_core.entry,
        &vk_core.instance,
        vk_core.physical_device,
        vk_core.graphics_family_index,
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
            println!("[wallbash] received {}", raw);

            if raw == "stop" {
                println!("[wallbash] stopping daemon.");
                running = false;
            } else if raw.starts_with("set ") {
                let args: Vec<&str> = raw[4..].trim().split_whitespace().collect();
                let path = args[..args.len()-2].join(" ");
                let anchor_x: f32 = args[args.len()-2].parse().unwrap_or(0.5);
                let anchor_y: f32 = args[args.len()-1].parse().unwrap_or(0.5);
                let resolved = std::fs::canonicalize(&path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or(path);

                println!("[wallbash] loading {} (anchor {}/{})", resolved, anchor_x, anchor_y);
                match set_wallpaper(
                    &resolved,
                    &vk_core,
                    &vk_surfchain,
                    wl_core.state.layer_width,
                    wl_core.state.layer_height,
                    &mut wallpaper,
                    anchor_x,
                    anchor_y,
                ) {
                    Ok(()) => println!("[wallbash] wallpaper set."),
                    Err(e) if e.to_string().contains("out of date") => {
                        println!("[wallbash] swapchain out of date, recreating...");

                        // destroy old swapchain and surface
                        vulkan::destroy_surfchain(
                            &vk_core.entry,
                            &vk_core.instance,
                            &vk_core.device,
                            &mut vk_surfchain,
                        );

                        // create a new one with the current layer dimensions
                        vk_surfchain = match vulkan::vulkan_surfchain(
                            &vk_core.entry,
                            &vk_core.instance,
                            vk_core.physical_device,
                            vk_core.graphics_family_index,
                            &vk_core.device,
                            &wl_core.display,
                            &wl_core.surface,
                            wl_core.state.layer_width,
                            wl_core.state.layer_height,
                        ) {
                            Ok(sc) => sc,
                            Err(e2) => {
                                eprintln!("[wallbash] failed to recreate swapchain {}", e2);
                                continue;
                            }
                        };

                        // retry setting the wallpaper once
                        if let Err(e3) = set_wallpaper(
                            &resolved,
                            &vk_core,
                            &vk_surfchain,
                            wl_core.state.layer_width,
                            wl_core.state.layer_height,
                            &mut wallpaper,
                            anchor_x,
                            anchor_y,
                        ) {
                            eprintln!("[wallbash] error after swapchain recreation {}", e3);
                        }
                    }
                    Err(e) => eprintln!("[wallbash] error {}", e),
                }
            } else {
                eprintln!("[wallbash] unknown {}", raw);
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    // clean shutdowon
    unsafe { vk_core.device.device_wait_idle()?; }
    if let Some(tex) = wallpaper.take() {
        unsafe {
            vk_core.device.destroy_image(tex.image, None);
            vk_core.device.free_memory(tex._memory, None);
        }
    }
    vulkan::destroy_surfchain(&vk_core.entry, &vk_core.instance, &vk_core.device, &mut vk_surfchain);
    unsafe { vk_core.device.destroy_command_pool(vk_core.command_pool, None); }
    unsafe { vk_core.device.destroy_device(None); }
    unsafe { vk_core.instance.destroy_instance(None); }

    println!("[wallbash] daemon stopped.");
    Ok(())
}

