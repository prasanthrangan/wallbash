// --------------------------------------------------------------------- / tittu
// wallbash
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

    // remove any stale socket file from a previous run
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
    mode: &str,
) -> Result<(), Box<dyn std::error::Error>> {

    // load the wallpaper
    let img = image::open(path)?;
    let pixel_format = img.to_rgba8();
    let pixel_bytes = pixel_format.into_raw();

    // call the vulkan pipeline
    let texture = vulkan::vulkan_pipeline(
        vk_core,
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

    // create a blurred version for fit/original modes
    let blurred_bg = if mode != "cover" {
        let bg = vulkan::blur_texture(
            vk_core,
            wallpaper.as_ref().unwrap(),
            img.width(),
            img.height(),
        )?;
        Some(bg)
    } else {
        None
    };

    // prepare the background parameter for draw_wallpaper
    let blur_bg = blurred_bg.as_ref().map(|b| (b.image, img.width(), img.height()));

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
        blur_bg,
        mode,
    )?;

    // destroy the temporary blurred texture (if any)
    if let Some(bg) = blurred_bg {
        unsafe {
            vk_core.device.destroy_image(bg.image, None);
            vk_core.device.free_memory(bg._memory, None);
        }
    }

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
            } else if raw.starts_with("set") {
                let args: Vec<&str> = raw[3..].split('\x01').collect();
                let mode = args[0].to_string();
                let anchor_x: f32 = args[1].parse().unwrap_or(0.5);
                let anchor_y: f32 = args[2].parse().unwrap_or(0.5);
                let path = args[3..].join("\x01");
                let resolved = std::fs::canonicalize(&path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or(path);

                println!("[wallbash] loading '{}' ({}-{}x{})", resolved, mode, anchor_x, anchor_y);
                match set_wallpaper(
                    &resolved,
                    &vk_core,
                    &vk_surfchain,
                    wl_core.state.layer_width,
                    wl_core.state.layer_height,
                    &mut wallpaper,
                    anchor_x,
                    anchor_y,
                    &mode,
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
                            &mode,
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
    unsafe { vk_core.device.destroy_pipeline(vk_core.blur_pipeline, None); }
    unsafe { vk_core.device.destroy_descriptor_set_layout(vk_core.blur_desc_layout, None); }
    unsafe { vk_core.device.destroy_shader_module(vk_core.blur_module, None); }
    unsafe { vk_core.device.destroy_device(None); }
    unsafe { vk_core.instance.destroy_instance(None); }

    println!("[wallbash] daemon stopped.");
    Ok(())
}

