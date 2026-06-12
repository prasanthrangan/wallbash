// --------------------------------------------------------------------- / tittu
// wallbash
// a vulkan module for HyDE
//


// --------------------------------------------------------------------- / imports

use std::ffi::c_void;
use ash::{
    vk, Entry, khr::{
        wayland_surface, surface, swapchain,
    },
};
use wayland_backend::client::ObjectId;
use wayland_client::{
    Proxy, protocol::{
        wl_display::WlDisplay, wl_surface::WlSurface,
    },
};


// --------------------------------------------------------------------- / datatypes

pub struct VulkanCore {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub graphics_family_index: u32,
    pub device: ash::Device,
    pub graphics_queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub command_buffer: vk::CommandBuffer,
}

pub struct VulkanSurfchain {
    pub surface: vk::SurfaceKHR,
    pub swapchain: vk::SwapchainKHR,
    pub swapchain_images: Vec<vk::Image>,
}

pub struct VulkanTexture {
    pub image: vk::Image,
    pub _memory: vk::DeviceMemory,
}


// --------------------------------------------------------------------- / init vulkan

pub fn vulkan_core() -> Result<VulkanCore, Box<dyn std::error::Error>> {

    // open GPU driver and get version
    let entry = unsafe { Entry::load()? };
    let version = unsafe {
        entry.try_enumerate_instance_version()?
            .unwrap_or(vk::make_api_version(0, 1, 0, 0))
    };
    println!("[v{}] vulkan driver: v{}.{}.{}",
        vk::api_version_variant(version),
        vk::api_version_major(version),
        vk::api_version_minor(version),
        vk::api_version_patch(version)
    );

    // setup root instance and enable wayland extensions (runtime)
    let app_info = vk::ApplicationInfo::default().api_version(version);
    let extensions = [c"VK_KHR_wayland_surface".as_ptr(), c"VK_KHR_surface".as_ptr()];
    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&extensions);
    let instance = unsafe { entry.create_instance(&create_info, None)? };

    // find and pick dGPU as physical device
    let list_devices = unsafe { instance.enumerate_physical_devices()? };
    let physical_device = list_devices.iter()
        .copied()
        .find(|&pdev| {
            let props = unsafe { instance.get_physical_device_properties(pdev) };
            props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU
        })
        .unwrap_or(list_devices[0]);
    println!("[v{:?}] physical device: {:?}", list_devices.len(), physical_device);

    // find and pick graphics queue family
    let queue_families = unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
    let (index, queue_family_props) = queue_families
        .iter()
        .enumerate()
        .find(|(_, q)| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        .expect("No graphics queue family found");
    let graphics_family_index = index as u32;
    println!("[v{}] queues available: {:?}", queue_family_props.queue_count, queue_family_props.queue_flags);

    // config logical device with personal GPU handle and worker queue
    let queue_info = vk::DeviceQueueCreateInfo {
        queue_family_index: graphics_family_index, // order graphics queue
        queue_count: 1, // order 1 worker for this queue
        p_queue_priorities: &1.0f32, // set high priority
        ..Default::default()
    };

    // enable swapchain extension (device feature)
    let swapchain_ext = c"VK_KHR_swapchain";
    let device_extensions = [swapchain_ext.as_ptr()];

    // create logical device
    let device_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_info))
        .enabled_extension_names(&device_extensions);
    let device = unsafe { instance.create_device(physical_device, &device_info, None)? };
    let graphics_queue = unsafe { device.get_device_queue(graphics_family_index, 0) };
    println!("[v{}] logical device: {:?} queue {:?}", graphics_family_index, device.handle(), graphics_queue);

    // create persistent command pool
    let command_pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(graphics_family_index);
    let command_pool = unsafe { device.create_command_pool(&command_pool_info, None)? };

    // allocate one reusable primary command buffer
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer = unsafe { device.allocate_command_buffers(&alloc_info)? }[0];

    Ok(VulkanCore {
        entry,
        instance,
        physical_device,
        graphics_family_index,
        device,
        graphics_queue,
        command_pool,
        command_buffer,
    })
}


// --------------------------------------------------------------------- / surface swapchain

pub fn vulkan_surfchain(
    entry: &ash::Entry,
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    graphics_family_index: u32,
    device: &ash::Device,
    disp: &WlDisplay,
    surf: &WlSurface,
    width: u32,
    height: u32,
) -> Result<VulkanSurfchain, Box<dyn std::error::Error>> {

    // get raw wayland pointers
    let wl_display_ptr = {
        let id: ObjectId = disp.id().into();
        id.as_ptr() as *mut c_void
    };
    let wl_surface_ptr = {
        let id: ObjectId = surf.id().into();
        id.as_ptr() as *mut c_void
    };

    // check queue family for wayland support
    let wayland_surface_loader = wayland_surface::Instance::new(entry, instance);
    let supports_present = unsafe {
        wayland_surface_loader.get_physical_device_wayland_presentation_support(
            physical_device,
            graphics_family_index,
            &mut *wl_display_ptr,
        )};
    if !supports_present {
        return Err("selected graphics queue family does not support wayland presentation".into());
    }

    // create vulkan surface object for wayland window
    let create_info = vk::WaylandSurfaceCreateInfoKHR::default()
        .display(wl_display_ptr)
        .surface(wl_surface_ptr);
    let surface = unsafe { wayland_surface_loader.create_wayland_surface(&create_info, None)? };
    println!("[v] vulkan surface: {:#?} x {:#?} >> {:#?}", disp.id(), surf.id(), surface);

    // query surface capabilities and formats
    let surface_loader = surface::Instance::new(entry, instance);
    let caps = unsafe {
        surface_loader.get_physical_device_surface_capabilities(physical_device, surface)?
    };
    let formats = unsafe {
        surface_loader.get_physical_device_surface_formats(physical_device, surface)?
    };

    // configure surface
    let chosen_format = formats
        .iter()
        .find(|f| {
            f.format == vk::Format::R8G8B8A8_SRGB &&
            f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .copied().unwrap_or(formats[0]);
    let extent = if caps.current_extent.width != u32::MAX {
        caps.current_extent
    } else {
        vk::Extent2D {
            width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        }
    };
    println!("[v] surface config: {:?} | {:?}", chosen_format, extent);

    // configure swapchain
    let swapchain_loader = swapchain::Device::new(instance, device);
    let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface)
        .min_image_count(2.max(caps.min_image_count))
        .image_format(chosen_format.format)
        .image_color_space(chosen_format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(vk::PresentModeKHR::FIFO)
        .clipped(true);

    // create the swapchain (ability to present rendering results to a surface)
    let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_create_info, None)? };
    let images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };
    println!("[v{}] swapchain: {}x{}", images.len(), width, height);

    Ok(VulkanSurfchain {
        surface,
        swapchain,
        swapchain_images: images,
    })
}


// --------------------------------------------------------------------- / destroy surfchain

pub fn destroy_surfchain(
    entry: &ash::Entry,
    instance: &ash::Instance,
    device: &ash::Device,
    surfchain: &mut VulkanSurfchain,
) {
    let swapchain_loader = ash::khr::swapchain::Device::new(instance, device);
    unsafe {
        swapchain_loader.destroy_swapchain(surfchain.swapchain, None);
    }
    let surface_loader = surface::Instance::new(entry, instance);
    unsafe {
        surface_loader.destroy_surface(surfchain.surface, None);
    }
}


// --------------------------------------------------------------------- / staging buffer

pub fn create_buffer(
    instance: &ash::Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    data: &[u8],
) -> Result<(vk::Buffer, vk::DeviceMemory), Box<dyn std::error::Error>> {

    // configure and create buffer
    let size = data.len() as u64;
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC) // use as source for copy operations
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { device.create_buffer(&buffer_info, None)? };

    // configure memory type
    let mem_requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    let mem_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };

    let memory_type_index = mem_properties.memory_types[..]
        .iter()
        .enumerate()
        .find(|(i, mt)| {
            mem_requirements.memory_type_bits & (1 << i) != 0
                && mt.property_flags.contains(
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                )
        })
        .map(|(i, _)| i as u32)
        .expect("No suitable memory type found for staging buffer");

    // allocate memory
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type_index);
    let memory = unsafe { device.allocate_memory(&alloc_info, None)? };

    // attach memory to buffer
    unsafe { device.bind_buffer_memory(buffer, memory, 0) }?;

    // map memory and copy data
    let ptr = unsafe {
        device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty())?
    } as *mut u8;
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len()); }
    unsafe { device.unmap_memory(memory) };

    Ok((buffer, memory))
}


// --------------------------------------------------------------------- / allocate vram

pub fn create_texture(
    instance: &ash::Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    width: u32,
    height: u32,
) -> Result<(vk::Image, vk::DeviceMemory), Box<dyn std::error::Error>> {

    // describe the image
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_SRGB)
        .extent(vk::Extent3D { width, height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    // create image handle
    let image = unsafe { device.create_image(&image_info, None)? };

    // how much memory does the image need?
    let mem_requirements = unsafe { device.get_image_memory_requirements(image) };
    let mem_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };

    // choose the memory type
    let memory_type_index = mem_properties.memory_types[..]
        .iter()
        .enumerate()
        .find(|(i, mt)| {
            mem_requirements.memory_type_bits & (1 << i) != 0
                && mt.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .map(|(i, _)| i as u32)
        .expect("No suitable memory type for texture image");

    // allocate vram for the image
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type_index);
    let memory = unsafe { device.allocate_memory(&alloc_info, None)? };

    // attach the vram to the image
    unsafe { device.bind_image_memory(image, memory, 0) }?;

    Ok((image, memory))
}


// --------------------------------------------------------------------- / write texture

pub fn load_texture(
    device: &ash::Device,
    graphics_queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    buffer: vk::Buffer,
    image: vk::Image,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {

    // reuse the persistent command buffer
    unsafe { device.reset_command_pool(command_pool, vk::CommandPoolResetFlags::empty()) }?;

    // begin recording commands
    let begin_info = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe { device.begin_command_buffer(command_buffer, &begin_info) }?;

    // make the texture writable
    let barrier1 = vk::ImageMemoryBarrier::default()
        .image(image)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier1],
        );
    }

    // copy the buffer into the image
    let region = vk::BufferImageCopy::default()
        .buffer_offset(0)
        .buffer_row_length(0)
        .buffer_image_height(0)
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
        .image_extent(vk::Extent3D { width, height, depth: 1 });
    unsafe {
        device.cmd_copy_buffer_to_image(
            command_buffer,
            buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
    }

    // make the texture readable by shaders
    let barrier2 = vk::ImageMemoryBarrier::default()
        .image(image)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier2],
        );
    }

    // finish recording commands
    unsafe { device.end_command_buffer(command_buffer) }?;

    // submit recording commands
    let submit_info = vk::SubmitInfo::default()
        .command_buffers(std::slice::from_ref(&command_buffer));
    let fence = unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }?;
    unsafe { device.queue_submit(graphics_queue, &[submit_info], fence) }?;
    unsafe { device.wait_for_fences(&[fence], true, u64::MAX) }?;
    unsafe { device.destroy_fence(fence, None) };

    Ok(())
}


// --------------------------------------------------------------------- / vulkan wrapper

pub fn vulkan_pipeline(
    instance: &ash::Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<VulkanTexture, Box<dyn std::error::Error>> {

    // copy raw image data to staging buffer
    let (buffer, memory) = create_buffer(instance, device, physical_device, pixel_bytes)?;

    // allocate vram for image data
    let (texture, vram) = create_texture(instance, device, physical_device, width, height)?;

    // load pixed data from buffer to texture
    load_texture(device, graphics_queue, command_pool, command_buffer, buffer, texture, width, height)?;

    // drop the staging buffer (no longer needed)
    unsafe {
        device.destroy_buffer(buffer, None);
        device.free_memory(memory, None);
    }

    Ok(VulkanTexture {
        image: texture,
        _memory: vram,
    })
}


// --------------------------------------------------------------------- / set mode

fn mode_set(
    img_w: u32,
    img_h: u32,
    scr_w: u32,
    scr_h: u32,
    anchor_x: f32,
    anchor_y: f32,
    mode: &str,
) -> (u32, u32, u32, u32, i32, i32, u32, u32, bool) {

    if mode == "fit" {
        let scale = (scr_w as f64 / img_w as f64).min(scr_h as f64 / img_h as f64);
        let sw = (img_w as f64 * scale) as u32;
        let sh = (img_h as f64 * scale) as u32;
        let dx = ((scr_w - sw) as f32 * anchor_x) as i32;
        let dy = ((scr_h - sh) as f32 * anchor_y) as i32;
        return (0, 0, img_w, img_h, dx, dy, sw, sh, true);
    }
    if mode == "original" {
        if img_w <= scr_w && img_h <= scr_h {
            let dx = ((scr_w - img_w) as f32 * anchor_x) as i32;
            let dy = ((scr_h - img_h) as f32 * anchor_y) as i32;
            return (0, 0, img_w, img_h, dx, dy, img_w, img_h, true);
        }
    }
    let src_aspect = img_w as f64 / img_h as f64;
    let dst_aspect = scr_w as f64 / scr_h as f64;
    let (sx, sy, sw, sh) = if src_aspect > dst_aspect {
        let new_w = (img_h as f64 * dst_aspect) as u32;
        let max_x = (img_w - new_w) as f32;
        let x = (max_x * anchor_x) as u32;
        (x, 0, new_w, img_h)
    } else {
        let new_h = (img_w as f64 / dst_aspect) as u32;
        let max_y = (img_h - new_h) as f32;
        let y = (max_y * anchor_y) as u32;
        (0, y, img_w, new_h)
    };
    (sx, sy, sw, sh, 0, 0, scr_w, scr_h, false)
}


// --------------------------------------------------------------------- / draw wallpaper

pub fn draw_wallpaper(
    instance: &ash::Instance,
    device: &ash::Device,
    graphics_queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    swapchain: vk::SwapchainKHR,
    swapchain_images: &[vk::Image],
    texture_image: vk::Image,
    texture_width: u32,
    texture_height: u32,
    swapchain_extent_width: u32,
    swapchain_extent_height: u32,
    anchor_x: f32,
    anchor_y: f32,
    mode: &str,
) -> Result<(), Box<dyn std::error::Error>> {

    // acquire swapchain image
    let swapchain_loader = ash::khr::swapchain::Device::new(instance, device);
    let (image_index, _suboptimal) = match unsafe {
        swapchain_loader.acquire_next_image(
            swapchain,
            u64::MAX,
            vk::Semaphore::null(),
            vk::Fence::null(),
        )
    } {
        Ok((idx, suboptimal)) => (idx, suboptimal),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
            return Err("swapchain out of date (acquire)".into());
        }
        Err(e) => return Err(Box::new(e)),
    };
    let target_image = swapchain_images[image_index as usize];

    // reuse the persistent command buffer
    unsafe { device.reset_command_pool(command_pool, vk::CommandPoolResetFlags::empty()) }?;

    // begin recording commands
    let begin_info = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe { device.begin_command_buffer(command_buffer, &begin_info) }?;

    // transition texture read-only access to source image transfer command
    let texture_barrier = vk::ImageMemoryBarrier::default()
        .image(texture_image)
        .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .src_access_mask(vk::AccessFlags::SHADER_READ)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // transition swapchain unknown layout to destination image transfer command
    let swapchain_barrier = vk::ImageMemoryBarrier::default()
        .image(target_image)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // execute both source and destination barriers
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[texture_barrier, swapchain_barrier],
        );
    }

    // compute source and destination rectangles based on mode
    let (src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h, needs_clear) = mode_set(
        texture_width, texture_height,
        swapchain_extent_width, swapchain_extent_height,
        anchor_x, anchor_y,
        mode,
    );

    // if the mode requires black bars, clear the image first
    if needs_clear {
        let clear_color = vk::ClearColorValue { float32: [0.0, 0.0, 0.0, 1.0] };
        let clear_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0, level_count: 1,
            base_array_layer: 0, layer_count: 1,
        };
        unsafe {
            device.cmd_clear_color_image(
                command_buffer,
                target_image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &clear_color,
                &[clear_range],
            );
        }
    }

    // record the blit command
    let blit_region = vk::ImageBlit::default()
        .src_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .src_offsets([
            vk::Offset3D {
                x: src_x as i32,
                y: src_y as i32,
                z: 0,
            },
            vk::Offset3D {
                x: (src_x + src_w) as i32,
                y: (src_y + src_h) as i32,
                z: 1,
            },
        ])
        .dst_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .dst_offsets([
            vk::Offset3D { x: dst_x, y: dst_y, z: 0 },
            vk::Offset3D { x: dst_x + dst_w as i32, y: dst_y + dst_h as i32, z: 1 },
        ]);
    unsafe {
        device.cmd_blit_image(
            command_buffer,
            texture_image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            target_image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[blit_region],
            vk::Filter::LINEAR,
        );
    }

    // transition swapchain to readable layout for compositor
    let present_barrier = vk::ImageMemoryBarrier::default()
        .image(target_image)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::MEMORY_READ)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[present_barrier],
        );
    }

    // finish recording commands
    unsafe { device.end_command_buffer(command_buffer) }?;

    // submit recording commands
    let submit_info = vk::SubmitInfo::default()
        .command_buffers(std::slice::from_ref(&command_buffer));
    let fence = unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }?;
    unsafe { device.queue_submit(graphics_queue, &[submit_info], fence) }?;
    unsafe { device.wait_for_fences(&[fence], true, u64::MAX) }?;
    unsafe { device.destroy_fence(fence, None) };

    // present the image
    let present_info = vk::PresentInfoKHR::default()
        .swapchains(std::slice::from_ref(&swapchain))
        .image_indices(std::slice::from_ref(&image_index));

    let result = unsafe { swapchain_loader.queue_present(graphics_queue, &present_info) };
    match result {
        Ok(_) => return Ok(()),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {return Err("swapchain out of date (present)".into())}
        Err(vk::Result::SUBOPTIMAL_KHR) => return Ok(()),
        Err(e) => return Err(Box::new(e)),
    }
}

