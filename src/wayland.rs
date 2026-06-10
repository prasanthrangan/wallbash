// --------------------------------------------------------------------- / tittu
// wallbash
// a wayland module for HyDE
//


// --------------------------------------------------------------------- / imports

use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, Proxy,
    protocol::{
        wl_display, wl_registry, wl_compositor, wl_surface, wl_output,
    },
};


// --------------------------------------------------------------------- / datatypes

pub struct WaylandCore {
    pub display: wl_display::WlDisplay,
    pub event: wayland_client::EventQueue<AppData>,
    pub state: AppData,
    pub surface: wl_surface::WlSurface,
}

pub struct AppData {
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    monitors: Vec<Monitor>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    pub layer_width: u32,
    pub layer_height: u32,
}

pub struct Monitor {
    _output: wl_output::WlOutput,
    name: Option<String>,
    width: i32,
    height: i32,
    refresh: i32,
    detected: bool,
}


// --------------------------------------------------------------------- / implementations

impl Dispatch<wl_registry::WlRegistry, ()> for AppData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<AppData>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(name, version, qh, ());
                    state.compositor = Some(compositor);
                    println!("[{}] ✓ {} (v{})", name, interface, version);
                }
                "zwlr_layer_shell_v1" => {
                    let layer_shell = registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(name, version, qh, ());
                    state.layer_shell = Some(layer_shell);
                    println!("[{}] ✓ {} (v{})", name, interface, version);
                }
                "wl_output" => {
                    let index = state.monitors.len();
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, version, qh, index);
                    state.monitors.push(Monitor {
                        _output: output,
                        name: None,
                        width: 0,
                        height: 0,
                        refresh: 0,
                        detected: false,
                    });
                    println!("[{}] ✓ {} (v{})", name, interface, version);
                }
                _ => println!("[{}] {} (v{})", name, interface, version)
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for AppData {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppData>,
    ) {}
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for AppData {
    fn event(
        _: &mut Self,
        _: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppData>,
    ) {}
}

impl Dispatch<wl_output::WlOutput, usize> for AppData {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<AppData>,
    ) {
        let monitor = &mut state.monitors[*index];
        match event {
            wl_output::Event::Mode { width, height, refresh, .. } => {
                monitor.width = width;
                monitor.height = height;
                monitor.refresh = refresh;
            }
            wl_output::Event::Name { name } => {
                monitor.name = Some(name);
            }
            wl_output::Event::Done => {
                if !monitor.detected {
                    println!("[{}] monitor: {}x{}@{:.1}Hz ({})", index,
                    monitor.width,
                    monitor.height,
                    monitor.refresh as f32 / 1000.0,
                    monitor.name.as_deref().unwrap_or("unknown"));
                    monitor.detected = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for AppData {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppData>,
    ) {}
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for AppData {
    fn event(
        state: &mut Self,
        proxy: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<AppData>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, width, height } = event {
            state.layer_width = width;
            state.layer_height = height;
            proxy.ack_configure(serial);
            println!("[{}] layer surface: {}x{}", serial, width, height);
        }
    }
}


// --------------------------------------------------------------------- / wayland

pub fn wayland_core() -> Result<WaylandCore, Box<dyn std::error::Error>> {

    // initialize wayland connection
    let connection = Connection::connect_to_env()?;
    let display = connection.display();
    println!("[w] {} (v{})", wl_display::WlDisplay::interface().name, display.version());

    let mut event = connection.new_event_queue();
    let handle = event.handle();
    let mut state: AppData = AppData {
        compositor: None,
        layer_shell: None,
        monitors: Vec::new(),
        layer_surface: None,
        layer_width: 0,
        layer_height: 0,
    };

    // sync registry for globals events
    display.get_registry(&handle, ());
    event.roundtrip(&mut state)?;

    // create wayland surface
    let compositor = state.compositor.as_ref().expect("wl_compositor not found");
    let layer_shell = state.layer_shell.as_ref().expect("zwlr_layer_shell_v1 not found");
    let surface = compositor.create_surface(&handle, ());
    println!("[w] {} (v{})", wl_surface::WlSurface::interface().name, surface.version());

    // configure wayland layer
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Background,
        "wallbash".to_string(),
        &handle,
        (),
    );
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_anchor(
        zwlr_layer_surface_v1::Anchor::Top|
        zwlr_layer_surface_v1::Anchor::Bottom|
        zwlr_layer_surface_v1::Anchor::Left|
        zwlr_layer_surface_v1::Anchor::Right
    );
    println!("[w] {} (v{})", zwlr_layer_surface_v1::ZwlrLayerSurfaceV1::interface().name, layer_surface.version());

    // commit and sync layer config
    surface.commit();
    state.layer_surface = Some(layer_surface);
    event.roundtrip(&mut state)?;

    Ok(WaylandCore {
        display,
        event,
        state,
        surface,
    })
}

