// --------------------------------------------------------------------- / tittu
// wallbash
// a daemon module for HyDE
//


// --------------------------------------------------------------------- / imports

use crate::{ipc, wayland, vulkan, filters, transitions, colors};
use ash::vk;
use std::{
    io::Write, time::Instant, collections::VecDeque,
    os::unix::net::UnixStream,
    sync::{mpsc,Arc},
};


// --------------------------------------------------------------------- / datatypes

struct DaemonState {
    wl_core: wayland::WaylandCore,
    vk_core: vulkan::VulkanCore,
    vk_surfchain: Option<vulkan::VulkanSurfchain>,
    wallpaper: Option<vulkan::VulkanTexture>,
    blur_state: BlurState,
    transition_state: TransitionState,
    decoded_cache: VecDeque<(String, image::DynamicImage)>,
}

struct BlurState {
    device: Arc<ash::Device>,
    module: vk::ShaderModule,
    pipeline: vk::Pipeline,
    desc_layout: vk::DescriptorSetLayout,
}

struct TransitionState {
    device: Arc<ash::Device>,
    scratch: vulkan::VulkanTexture,
    registry: transitions::TransitionRegistry,
    pending: Option<transitions::TransitionCore>,
}


// --------------------------------------------------------------------- / benchmarking

fn timer<F, R>(label: &str, f: F) -> R
where F: FnOnce() -> R {
    let start = Instant::now();
    let result = f();
    println!("[perf] {}: {:.2?}", label, start.elapsed());
    result
}


// --------------------------------------------------------------------- / blur state

impl BlurState {
    fn new(vk_core: &vulkan::VulkanCore) -> Result<Self, Box<dyn std::error::Error>> {
        let device = Arc::clone(&vk_core.device);
        let (module, pipeline, desc_layout) = filters::filter_pipeline(&device, "blur")?;
        Ok(Self { device, module, pipeline, desc_layout })
    }
}

impl Drop for BlurState {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_descriptor_set_layout(self.desc_layout, None);
            self.device.destroy_shader_module(self.module, None);
        }
    }
}


// --------------------------------------------------------------------- / transition state

impl TransitionState {
    fn new(
        vk_core: &vulkan::VulkanCore,
        layer_width: u32,
        layer_height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let device = Arc::clone(&vk_core.device);
        let (img, mem) = vk_core.create_texture(
            layer_width,
            layer_height,
            ash::vk::ImageUsageFlags::TRANSFER_SRC | ash::vk::ImageUsageFlags::STORAGE,
            ash::vk::Format::R8G8B8A8_UNORM,
        )?;
        let scratch = vulkan::VulkanTexture {
            image: img,
            _memory: mem,
            width: layer_width,
            height: layer_height,
        };
        let registry = transitions::TransitionRegistry::new(&device)?;
        Ok(Self {
            device,
            scratch,
            registry,
            pending: None,
        })
    }
}

impl TransitionState {
    pub fn start(
        &mut self,
        vk_core: &vulkan::VulkanCore,
        old: vulkan::VulkanTexture,
        new_tex: &vulkan::VulkanTexture,
        bezier: &str,
        scale: ipc::ScalingMode,
        anchor_x: f32,
        anchor_y: f32,
        background: Option<vulkan::VulkanTexture>,
    ) {
        let cfg = transitions::TransitionConfig::parse("zoom", 400, bezier);
        let target_fps = 60.0_f64;
        let total_frames = ((cfg.duration_ms as f64 / 1000.0) * target_fps).ceil() as u32;

        let anim = self.registry.get(&cfg.kind);
        let resources = match anim.prepare(vk_core, &old, new_tex, &self.scratch) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[wallbash] error preparing transition: {}", e);
                transitions::TransitionResources::None
            }
        };

        self.pending = Some(transitions::TransitionCore {
            cfg,
            prev: old,
            start: Instant::now(),
            total_frames,
            current_frame: 0,
            scale: scale.as_str().to_string(),
            anchor_x,
            anchor_y,
            background,
            resources,
        });
    }
}

impl TransitionState {
    pub fn advance(
        &mut self,
        vk_core: &vulkan::VulkanCore,
        wl_core: &wayland::WaylandCore,
        vk_surfchain: &vulkan::VulkanSurfchain,
    ) {
        let pending = match self.pending.as_mut() {
            Some(p) => p,
            None => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                return;
            }
        };

        if pending.current_frame <= pending.total_frames {
            let raw_t = (pending.current_frame as f32 / pending.total_frames as f32).clamp(0.0, 1.0);
            let t = pending.cfg.bezier.apply(raw_t);

            let anim = self.registry.get(&pending.cfg.kind);
            if let Err(e) = anim.render_frame(
                &vk_core,
                &pending.resources,
                t,
                &pending.scale,
                pending.anchor_x,
                pending.anchor_y,
            ) { eprintln!("[wallbash] transition frame error: {}", e); }

            let bg_params = pending.background.as_ref().map(|b| (b.image, b.width, b.height));
            if let Err(e) = vk_core.draw_wallpaper(
                vk_surfchain,
                &self.scratch,
                wl_core.state.layer_width,
                wl_core.state.layer_height,
                pending.anchor_x,
                pending.anchor_y,
                bg_params,
                &pending.scale,
            ) { eprintln!("[wallbash] error drawing transition frame: {}", e); }

            pending.current_frame += 1;
            let frame_duration = std::time::Duration::from_secs_f64(1.0 / 60.0);
            let elapsed = pending.start.elapsed();
            let target = frame_duration.mul_f64((pending.current_frame + 1) as f64);
            if elapsed < target {
                std::thread::sleep(target - elapsed);
            }
        } else {
            self.cancel(vk_core);
        }
    }
}

impl TransitionState {
    pub fn cancel(&mut self, vk_core: &vulkan::VulkanCore) {
        if let Some(mut core) = self.pending.take() {
            vk_core.destroy_texture(&core.prev);
            if let Some(bg) = core.background.take() {
                vk_core.destroy_texture(&bg);
            }
            let anim = self.registry.get(&core.cfg.kind);
            anim.cleanup(&vk_core.device, core.resources);
        }
    }
}

impl Drop for TransitionState {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_image(self.scratch.image, None);
            self.device.free_memory(self.scratch._memory, None);
        }
        self.registry.destroy(&self.device);
    }
}


// --------------------------------------------------------------------- / daemon state

impl DaemonState {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let wl_core = wayland::wayland_core()?;
        let vk_core = vulkan::vulkan_core()?;
        let blur_state = BlurState::new(&vk_core)?;
        let vk_surfchain = Some(vulkan::vulkan_surfchain(
            &vk_core, &wl_core.display, &wl_core.surface, wl_core.state.layer_width, wl_core.state.layer_height
        )?);

        let transition_state = TransitionState::new(
            &vk_core,
            wl_core.state.layer_width,
            wl_core.state.layer_height,
        )?;

        Ok(Self {
            wl_core,
            vk_core,
            vk_surfchain,
            wallpaper: None,
            blur_state,
            transition_state,
            decoded_cache: VecDeque::new(),
        })
    }
}

impl DaemonState {
    fn load(&mut self, path: &str) -> Result<(image::DynamicImage, Vec<u8>), Box<dyn std::error::Error>> {
        let cache_hit = self.decoded_cache.iter().position(|(p, _)| p == path);
        let label = if cache_hit.is_some() { "load-cached" } else { "load+decode" };
        
        timer(label, || -> Result<_, Box<dyn std::error::Error>> {
            let img = if let Some(idx) = cache_hit {
                self.decoded_cache.remove(idx).expect("cache index invalid")
            } else {
                (path.to_string(), image::open(path)?)
            }.1;
            
            let bytes = img.to_rgba8().into_raw();
            self.decoded_cache.push_back((path.to_string(), img.clone()));
            if self.decoded_cache.len() > 30 { self.decoded_cache.pop_front(); }
            
            Ok((img, bytes))
        })
    }
}

impl DaemonState {
    fn set_command(
        &mut self,
        palette: ipc::PaletteMode,
        bezier: String,
        scale: ipc::ScalingMode,
        ax: f32,
        ay: f32,
        path: String
    ) -> Result<(), Box<dyn std::error::Error>> {
        let resolved = std::fs::canonicalize(&path).map(|p| p.to_string_lossy().to_string()).unwrap_or(path);
        println!("[wallbash] loading '{}' | {:?} | {:?} | bz:{} | a(x,y):({:.1},{:.1})", resolved, palette, scale, bezier, ax, ay);

        self.transition_state.cancel(&self.vk_core);
        let (img, pixel_bytes) = self.load(&resolved)?;
        println!("[wallbash] decoded images: {}/30", self.decoded_cache.len());

        let texture = timer("upload", || self.vk_core.upload_texture(&pixel_bytes, img.width(), img.height()))?;

        let background = if scale != ipc::ScalingMode::Cover {
            timer("blur", || {
                filters::blur_texture(
                    &self.vk_core,
                    &texture,
                    texture.width,
                    texture.height,
                    self.blur_state.pipeline,
                    self.blur_state.desc_layout
                ).ok().map(|b| vulkan::VulkanTexture { image: b.image, _memory: b._memory, width: b.width, height: b.height })
            })
        } else { None };

        if palette != ipc::PaletteMode::Skip {
            let p_str = palette.as_str();
            std::thread::spawn(move || { let _ = timer("dcols", || colors::dcol(&img, p_str)); });
        }

        let old_tex = self.wallpaper.replace(texture);
        let new_tex = self.wallpaper.as_ref().ok_or_else(|| "[error] wallpaper state corruption")?;

        if let Some(old) = old_tex {
            self.transition_state.start(&self.vk_core, old, new_tex, &bezier, scale, ax, ay, background);
        } else {
            let bg_params = background.as_ref().map(|b| (b.image, b.width, b.height));
            self.vk_core.draw_wallpaper(
            self.vk_surfchain.as_ref().unwrap(),
            new_tex,
            self.wl_core.state.layer_width,
            self.wl_core.state.layer_height,
            ax,
            ay,
            bg_params,
            scale.as_str())?;
        }

        println!("[wallbash] wallpaper set.");
        Ok(())
    }
}

impl Drop for DaemonState {
    fn drop(&mut self) {
        vulkan::destroy_wallbash(
            &self.vk_core,
            vulkan::VulkanCleanup {
                surfchain: self.vk_surfchain.take(),
                filter_module: None,
                filter_pipeline: None,
                filter_desc_layout: None,
                wallpaper_texture: self.wallpaper.take(),
            },
            2,
        );
        println!("[wallbash] GPU resources safely released.");
    }
}


// --------------------------------------------------------------------- / daemon run

pub fn run(socket_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if UnixStream::connect(socket_path).is_ok() {
        return Err("Daemon is already running.".into());
    }
    let rx: mpsc::Receiver<ipc::IpcMessage> = ipc::start_ipc(socket_path)?;

    let mut state = DaemonState::new()?;
    println!("[wallbash] ready, press Ctrl+C to quit.");

    let mut running = true;
    while running {
        state.wl_core.event.dispatch_pending(&mut state.wl_core.state)?;

        if let Ok(mut msg) = rx.try_recv() {
            match ipc::Command::parse_raw(&msg.cmd) {
                Ok(ipc::Command::Stop) => {
                    println!("[wallbash] stopping daemon.");
                    running = false;
                }
                Ok(ipc::Command::Set { .. }) => {
                    let mut last_msg = msg;
                    while let Ok(new_msg) = rx.try_recv() {
                        if matches!(ipc::Command::parse_raw(&new_msg.cmd), Ok(ipc::Command::Set { .. })) {
                            last_msg = new_msg;
                        } else {
                            break;
                        }
                    }
                    if let Ok(ipc::Command::Set { palette, bezier, scale, anchor_x, anchor_y, path }) = ipc::Command::parse_raw(&last_msg.cmd) {
                        let result = state.set_command(palette, bezier, scale, anchor_x, anchor_y, path);
                        let _ = last_msg.stream.write(&[0u8]);
                        let _ = last_msg.stream.set_nonblocking(true);
                        if result.is_err() { continue; }
                    }
                }
                Err(e) => {
                    eprintln!("[ipc] invalid command: {}", e);
                    let _ = msg.stream.write(&[0u8]);
                }
            }
        }
        state.transition_state.advance(&state.vk_core, &state.wl_core, state.vk_surfchain.as_ref().unwrap());
    }

    println!("[wallbash] daemon stopped.");
    Ok(())
}

