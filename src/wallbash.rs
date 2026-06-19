// --------------------------------------------------------------------- / tittu
// wallbash
// a daemon module for HyDE
//


// --------------------------------------------------------------------- / imports

use crate::{vulkan, wayland, filters};
use std::{
    os::unix::net::{UnixListener, UnixStream},
    io::{BufRead, BufReader},
    sync::mpsc,time::Instant,
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


// --------------------------------------------------------------------- / timer

fn timer<F, R>(label: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let result = f();
    println!("[perf] {}: {:.2?}", label, start.elapsed());
    result
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
    effect: impl FnOnce(&vulkan::VulkanTexture) -> Option<vulkan::VulkanTexture>,
) -> Result<(), Box<dyn std::error::Error>> {

    // load the wallpaper
    let (img, pixel_bytes) = timer("load+decode", || {
        let img = image::open(path)?;
        let rgba = img.to_rgba8();
        let bytes = rgba.into_raw();
        Ok::<_, Box<dyn std::error::Error>>((img, bytes))
    })?;

    // call the vulkan pipeline
    let texture = timer("upload", || {
        vk_core.upload_texture(&pixel_bytes, img.width(), img.height())
    })?;

    // drop the old texture resources (if any)
    if let Some(old_tex) = wallpaper.take() {
        unsafe {
            vk_core.device.destroy_image(old_tex.image, None);
            vk_core.device.free_memory(old_tex._memory, None);
        }
    }
    *wallpaper = Some(texture);

    // create a blurred version for fit/original modes
    let background_texture = timer("effect+draw", || {
        let bg = effect(wallpaper.as_ref().unwrap());
        let bg_params = bg.as_ref().map(|b| (b.image, b.width, b.height));

        vk_core.draw_wallpaper(
            vk_surfchain,
            wallpaper.as_ref().unwrap(),
            layer_width,
            layer_height,
            anchor_x,
            anchor_y,
            bg_params,
            mode,
        )?;

        Ok::<_, Box<dyn std::error::Error>>(bg)
    })?;

    // destroy the temporary blurred texture (if any)
    if let Some(bg) = background_texture {
        unsafe {
            vk_core.device.destroy_image(bg.image, None);
            vk_core.device.free_memory(bg._memory, None);
        }
    }

    Ok(())
}


// --------------------------------------------------------------------- / surfchain

fn set_surfchain(
    vk_core: &vulkan::VulkanCore,
    wl_core: &wayland::WaylandCore,
    old: Option<vulkan::VulkanSurfchain>,
) -> Result<vulkan::VulkanSurfchain, Box<dyn std::error::Error>> {

    // destroy only the swapchain (level 1) – no filter resources
    if let Some(old_sc) = old {
        vulkan::destroy_wallbash(
            vk_core,
            vulkan::VulkanCleanup {
                surfchain: Some(old_sc),
                filter_module: None,
                filter_pipeline: None,
                filter_desc_layout: None,
                wallpaper_texture: None,
            },
            1,
        );
    }

    vulkan::vulkan_surfchain(
        &vk_core.entry,
        &vk_core.instance,
        vk_core.physical_device,
        vk_core.graphics_family_index,
        &vk_core.device,
        &wl_core.display,
        &wl_core.surface,
        wl_core.state.layer_width,
        wl_core.state.layer_height,
    )
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
    let (blur_module, blur_pipeline, blur_desc_layout) = filters::filter_pipeline(&vk_core.device, "blur")?;
    let mut vk_surfchain = Some(set_surfchain(&vk_core, &wl_core, None)?);

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
                println!("[wallbash] loading '{}' ({}|ax:{:?}|ay:{:?})", resolved, mode, anchor_x, anchor_y);

                // apply blur for non cover mode
                let effect = |tex: &vulkan::VulkanTexture| {
                    if mode != "cover" {
                        filters::blur_texture(
                            &vk_core,
                            tex,
                            tex.width,
                            tex.height,
                            blur_pipeline,
                            blur_desc_layout,
                        )
                        .ok()
                    } else {
                        None
                    }
                };

                match set_wallpaper(
                    &resolved,
                    &vk_core,
                    vk_surfchain.as_ref().unwrap(),
                    wl_core.state.layer_width,
                    wl_core.state.layer_height,
                    &mut wallpaper,
                    anchor_x,
                    anchor_y,
                    &mode,
                    effect,
                ) {
                    Ok(()) => println!("[wallbash] wallpaper set."),
                    Err(e) if e.to_string().contains("out of date") => {
                        println!("[wallbash] swapchain out of date, recreating...");

                        // destroy old swapchain and surface
                        match vulkan::vulkan_surfchain(
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
                            Ok(new_sc) => {
                                // Destroy the old swapchain (now that we have a working new one)
                                let old = vk_surfchain.take().unwrap();
                                vulkan::destroy_wallbash(
                                    &vk_core,
                                    vulkan::VulkanCleanup {
                                        surfchain: Some(old),
                                        filter_module: None,
                                        filter_pipeline: None,
                                        filter_desc_layout: None,
                                        wallpaper_texture: None,
                                    },
                                    1,
                                );
                                vk_surfchain = Some(new_sc);
                            }
                            Err(e2) => {
                                eprintln!("[wallbash] failed to recreate swapchain {}", e2);
                                continue;
                            }
                        }

                        // retry setting the wallpaper once
                        if let Err(e3) = set_wallpaper(
                            &resolved,
                            &vk_core,
                            vk_surfchain.as_ref().unwrap(),
                            wl_core.state.layer_width,
                            wl_core.state.layer_height,
                            &mut wallpaper,
                            anchor_x,
                            anchor_y,
                            &mode,
                            effect,
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
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    // clean shutdowon
    vulkan::destroy_wallbash(
        &vk_core,
        vulkan::VulkanCleanup {
            surfchain: vk_surfchain.take(),
            filter_module: Some(blur_module),
            filter_pipeline: Some(blur_pipeline),
            filter_desc_layout: Some(blur_desc_layout),
            wallpaper_texture: wallpaper.take(),
        },
        2,
    );

    println!("[wallbash] daemon stopped.");
    Ok(())
}

