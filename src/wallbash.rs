// --------------------------------------------------------------------- / tittu
// wallbash
// a daemon module for HyDE
//


// --------------------------------------------------------------------- / imports

use crate::{vulkan, wayland, filters, colors};
use ash::vk;
use std::{
    os::unix::net::{UnixListener, UnixStream},
    io::{BufRead, BufReader},
    sync::mpsc, time::Instant,
};


// --------------------------------------------------------------------- / datatypes

enum Command {
    Stop,
    Status,
    Set { palette: String, mode: String, anchor_x: f32, anchor_y: f32, path: String },
}

struct DaemonState {
    vk_core: vulkan::VulkanCore,
    wl_core: wayland::WaylandCore,
    vk_surfchain: Option<vulkan::VulkanSurfchain>,
    wallpaper: Option<vulkan::VulkanTexture>,
    blur_module: vk::ShaderModule,
    blur_pipeline: vk::Pipeline,
    blur_desc_layout: vk::DescriptorSetLayout,
}


// --------------------------------------------------------------------- / implementations

impl Command {
    fn parse_raw(raw: &str) -> Self {
        let raw = raw.trim();
        if raw == "stop"   { return Command::Stop; }
        if raw == "status" { return Command::Status; }
        if raw.starts_with("set") {
            let payload = &raw[3..];
            let mut parts = payload.splitn(5, '\x01');
            let palette = parts.next().unwrap().to_string();
            let mode = parts.next().unwrap().to_string();
            let anchor_x = parts.next().unwrap().parse().unwrap();
            let anchor_y = parts.next().unwrap().parse().unwrap();
            let path = parts.next().unwrap().to_string();
            return Command::Set { palette, mode, anchor_x, anchor_y, path };
        }
        panic!("unknown internal command: {}", raw);
    }
}

impl DaemonState {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let wl_core = wayland::wayland_core()?;
        let vk_core = vulkan::vulkan_core()?;
        let (blur_module, blur_pipeline, blur_desc_layout) = filters::filter_pipeline(&vk_core.device, "blur")?;
        let vk_surfchain = Some(set_surfchain(&vk_core, &wl_core, None)?);
        Ok(Self {
            vk_core,
            wl_core,
            vk_surfchain,
            wallpaper: None,
            blur_module,
            blur_pipeline,
            blur_desc_layout,
        })
    }
}


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
    palette: &String,
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

    // generate colors
    if palette != "skip" {
        let dcol = timer("dcols", || colors::dcol(&img));
        colors::print_palette(dcol, &palette);
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


// --------------------------------------------------------------------- / command

impl DaemonState {
    fn set_command(
        &mut self,
        palette: String,
        mode: String,
        anchor_x: f32,
        anchor_y: f32,
        path: String,
    ) -> Result<(), ()> {
        let resolved = std::fs::canonicalize(&path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(path);
        println!("[wallbash] loading '{}' ({}|{}|ax:{:?}|ay:{:?})", resolved, palette, mode, anchor_x, anchor_y);

        let effect = |tex: &vulkan::VulkanTexture| {
            if mode != "cover" {
                filters::blur_texture(
                    &self.vk_core,
                    tex,
                    tex.width,
                    tex.height,
                    self.blur_pipeline,
                    self.blur_desc_layout,
                )
                .ok()
            } else { None }
        };

        match set_wallpaper(
            &resolved,
            &self.vk_core,
            self.vk_surfchain.as_ref().unwrap(),
            self.wl_core.state.layer_width,
            self.wl_core.state.layer_height,
            &mut self.wallpaper,
            anchor_x,
            anchor_y,
            &mode,
            effect,
            &palette,
        ) {
            Ok(()) => {
                println!("[wallbash] wallpaper set.");
                Ok(())
            }
            Err(e) if e.to_string().contains("out of date") => {
                println!("[wallbash] swapchain out of date, recreating...");

                match vulkan::vulkan_surfchain(
                    &self.vk_core.entry,
                    &self.vk_core.instance,
                    self.vk_core.physical_device,
                    self.vk_core.graphics_family_index,
                    &self.vk_core.device,
                    &self.wl_core.display,
                    &self.wl_core.surface,
                    self.wl_core.state.layer_width,
                    self.wl_core.state.layer_height,
                ) {
                    Ok(new_sc) => {
                        let old = self.vk_surfchain.take().unwrap();
                        vulkan::destroy_wallbash(
                            &self.vk_core,
                            vulkan::VulkanCleanup {
                                surfchain: Some(old),
                                filter_module: None,
                                filter_pipeline: None,
                                filter_desc_layout: None,
                                wallpaper_texture: None,
                            },
                            1,
                        );
                        self.vk_surfchain = Some(new_sc);
                    }
                    Err(e2) => {
                        eprintln!("[wallbash] failed to recreate swapchain {}", e2);
                        return Err(());
                    }
                }

                if let Err(e3) = set_wallpaper(
                    &resolved,
                    &self.vk_core,
                    self.vk_surfchain.as_ref().unwrap(),
                    self.wl_core.state.layer_width,
                    self.wl_core.state.layer_height,
                    &mut self.wallpaper,
                    anchor_x,
                    anchor_y,
                    &mode,
                    effect,
                    &palette,
                ) {
                    eprintln!("[wallbash] error after swapchain recreation {}", e3);
                }
                Ok(())
            }
            Err(e) => {
                eprintln!("[wallbash] error {}", e);
                Ok(())
            }
        }
    }
}


// --------------------------------------------------------------------- / daemon

pub fn run(socket_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if UnixStream::connect(socket_path).is_ok() {
        return Err("Daemon is already running.".into());
    }
    let _ = std::fs::remove_file(socket_path);
    let rx = start_ipc(socket_path)?;

    let mut state = DaemonState::new()?;
    println!("[wallbash] ready, press Ctrl+C to quit.");

    let mut running = true;
    while running {
        state.wl_core.event.dispatch_pending(&mut state.wl_core.state)?;
        if let Ok(raw) = rx.try_recv() {
            match Command::parse_raw(&raw) {
                Command::Stop => {
                    println!("[wallbash] stopping daemon.");
                    running = false;
                }
                Command::Status => {
                    println!("[wallbash] daemon is running.");
                }
                Command::Set { palette, mode, anchor_x, anchor_y, path } => {
                    if state.set_command(palette, mode, anchor_x, anchor_y, path).is_err()
                    {
                        continue;
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    vulkan::destroy_wallbash(
        &state.vk_core,
        vulkan::VulkanCleanup {
            surfchain: state.vk_surfchain.take(),
            filter_module: Some(state.blur_module),
            filter_pipeline: Some(state.blur_pipeline),
            filter_desc_layout: Some(state.blur_desc_layout),
            wallpaper_texture: state.wallpaper.take(),
        },
        2,
    );

    println!("[wallbash] daemon stopped.");
    Ok(())
}

