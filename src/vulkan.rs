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
}

pub struct VulkanSurfchain {
    pub swapchain: vk::SwapchainKHR,
    pub swapchain_images: Vec<vk::Image>,
}

pub struct VulkanTexture {
    pub image: vk::Image,
    pub _memory: vk::DeviceMemory,
    pub _view: vk::ImageView,
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

    // create logical device with personal GPU handle and worker queue
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

    Ok(VulkanCore {
        entry,
        instance,
        physical_device,
        graphics_family_index,
        device,
        graphics_queue,
    })
}


// --------------------------------------------------------------------- / surface swapchain

pub fn vulkan_surfchain(
    entry: &ash::Entry,
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
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

    // create vulkan surface object for wayland window
    let wayland_surface_loader = wayland_surface::Instance::new(entry, instance);
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
    let chosen_extent = caps.current_extent;
    println!("[v] surface config: {:?} | {:?}", chosen_format, chosen_extent);

    // configure swapchain
    let swapchain_loader = swapchain::Device::new(instance, device);
    let extent = vk::Extent2D { width, height };
    let image_count = 2;

    let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface)
        .min_image_count(image_count)
        .image_format(chosen_format.format)
        .image_color_space(chosen_format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
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
        swapchain,
        swapchain_images: images,
    })
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
    graphics_family_index: u32,
    buffer: vk::Buffer,
    image: vk::Image,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {

    // create a temporary command pool
    let command_pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(graphics_family_index)
        .flags(vk::CommandPoolCreateFlags::TRANSIENT);
    let command_pool = unsafe { device.create_command_pool(&command_pool_info, None)? };

    // create a temporary command buffer
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer = unsafe { device.allocate_command_buffers(&alloc_info)? }[0];

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

    // clean up the command pool
    unsafe { device.destroy_command_pool(command_pool, None) };

    Ok(())
}


// --------------------------------------------------------------------- / read texture

pub fn view_texture(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
) -> Result<vk::ImageView, Box<dyn std::error::Error>> {

    // describe the view
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // create the view to read texture
    let view = unsafe { device.create_image_view(&view_info, None)? };
    Ok(view)
}


// --------------------------------------------------------------------- / vulkan wrapper

pub fn vulkan_pipeline(
    instance: &ash::Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    graphics_family_index: u32,
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<VulkanTexture, Box<dyn std::error::Error>> {

    // copy raw image data to staging buffer
    let (buffer, memory) = create_buffer(instance, device, physical_device, pixel_bytes)?;

    // allocate vram for image data
    let (texture, vram) = create_texture(instance, device, physical_device, width, height)?;

    // load pixed data from buffer to texture
    load_texture(device, graphics_queue, graphics_family_index, buffer, texture, width, height)?;

    // sample/read the texture
    let view = view_texture(device, texture, vk::Format::R8G8B8A8_SRGB)?;

    // drop the staging buffer (no longer needed)
    unsafe {
        device.destroy_buffer(buffer, None);
        device.free_memory(memory, None);
    }

    Ok(VulkanTexture {
        image: texture,
        _memory: vram,
        _view: view,
    })
}


// --------------------------------------------------------------------- / draw wallpaper

pub fn draw_wallpaper(
    instance: &ash::Instance,
    device: &ash::Device,
    graphics_queue: vk::Queue,
    graphics_family_index: u32,
    swapchain: vk::SwapchainKHR,
    swapchain_images: &[vk::Image],
    texture_image: vk::Image,
    texture_width: u32,
    texture_height: u32,
    swapchain_extent_width: u32,
    swapchain_extent_height: u32,
) -> Result<(), Box<dyn std::error::Error>> {

    // acquire swapchain image
    let swapchain_loader = ash::khr::swapchain::Device::new(instance, device);
    let (image_index, _) = unsafe {
        swapchain_loader.acquire_next_image(
            swapchain,
            u64::MAX,
            vk::Semaphore::null(),
            vk::Fence::null(),
        )
    }?;
    let target_image = swapchain_images[image_index as usize];

    // create a temporary command pool
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(graphics_family_index)
        .flags(vk::CommandPoolCreateFlags::TRANSIENT);
    let command_pool = unsafe { device.create_command_pool(&pool_info, None)? };

    // create a temporary command buffer
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer = unsafe { device.allocate_command_buffers(&alloc_info)? }[0];

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

    // preserve aspect ratios and fill screen
    let src_aspect = texture_width as f64 / texture_height as f64;
    let dst_aspect = swapchain_extent_width as f64 / swapchain_extent_height as f64;
    let (src_x, src_y, src_w, src_h) = if src_aspect > dst_aspect {
        // Image is wider than screen → crop left/right
        let new_width = (texture_height as f64 * dst_aspect) as u32;
        let x = (texture_width - new_width) / 2;
        (x, 0, new_width, texture_height)
    } else {
        // Image is taller than screen → crop top/bottom
        let new_height = (texture_width as f64 / dst_aspect) as u32;
        let y = (texture_height - new_height) / 2;
        (0, y, texture_width, new_height)
    };

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
            vk::Offset3D { x: 0, y: 0, z: 0 },
            vk::Offset3D {
                x: swapchain_extent_width as i32,
                y: swapchain_extent_height as i32,
                z: 1,
            },
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
    unsafe { swapchain_loader.queue_present(graphics_queue, &present_info) }?;

    // clean up the command pool
    unsafe { device.destroy_command_pool(command_pool, None) };

    Ok(())
}

