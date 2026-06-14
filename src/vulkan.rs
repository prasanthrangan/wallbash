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
    pub blur_module: vk::ShaderModule,
    pub blur_pipeline: vk::Pipeline,
    pub blur_desc_layout: vk::DescriptorSetLayout,
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

pub enum PipelineType<'a> {
    Compute,
    Graphics {
        vert_spv: &'a [u32],
        extent: vk::Extent2D,
        render_pass: vk::RenderPass,
    },
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
    println!("[v] instance: {:?}", instance.handle());

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

    // enable swapchain extension
    let swapchain_ext = c"VK_KHR_swapchain";
    let device_extensions = [swapchain_ext.as_ptr()];

    // create logical device and queue
    let device_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_info))
        .enabled_extension_names(&device_extensions);
    let device: ash::Device = unsafe { instance.create_device(physical_device, &device_info, None)? };
    let graphics_queue = unsafe { device.get_device_queue(graphics_family_index, 0) };
    println!("[v{}] logical device {:?} >> graphics queue {:?}", graphics_family_index, device.handle(), graphics_queue);

    // create persistent command pool and buffer
    let command_pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(graphics_family_index);
    let command_pool = unsafe { device.create_command_pool(&command_pool_info, None)? };

    // allocate one reusable primary command buffer
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffer = unsafe { device.allocate_command_buffers(&alloc_info)? }[0];
    println!("[v] command pool {:?} >> command buffer {:?}", command_pool, command_buffer);

    // build blur compute pipeline
    let blur_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/blur.comp.spv"));
    let blur_words = unsafe {
        std::slice::from_raw_parts(blur_bytes.as_ptr() as *const u32, blur_bytes.len() / 4)
    };

    // descriptor layout: binding 0 = sampler, binding 1 = storage image
    let bindings = [
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE),
        vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE),
    ];
    let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    let blur_desc_layout = unsafe { device.create_descriptor_set_layout(&layout_info, None)? };

    let (blur_module, blur_pipeline) = build_pipeline(
        &device,
        PipelineType::Compute,
        &blur_words,
        blur_desc_layout,
    )?;

    Ok(VulkanCore {
        entry,
        instance,
        physical_device,
        graphics_family_index,
        device,
        graphics_queue,
        command_pool,
        command_buffer,
        blur_module: blur_module[0],
        blur_pipeline,
        blur_desc_layout,
    })
}


// --------------------------------------------------------------------- / build pipeline

fn build_pipeline(
    device: &ash::Device,
    pipeline_type: PipelineType,
    frag_spv: &[u32],
    desc_layout: vk::DescriptorSetLayout,
) -> Result<(Vec<vk::ShaderModule>, vk::Pipeline), Box<dyn std::error::Error>> {
    let mut modules = Vec::new();

    let stages = match pipeline_type {
        PipelineType::Compute => {
            let module = load_shader(device, frag_spv)?;
            modules.push(module);
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(c"main");
            vec![stage]
        }
        PipelineType::Graphics { vert_spv, extent: _, .. } => {
            let vert_module = load_shader(device, vert_spv)?;
            let frag_module = load_shader(device, frag_spv)?;
            modules.push(vert_module);
            modules.push(frag_module);
            let vert_stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(c"main");
            let frag_stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(c"main");
            vec![vert_stage, frag_stage]
        }
    };

    let set_layouts = [desc_layout];
    let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&pipeline_layout_info, None)? };

    let pipeline = match pipeline_type {
        PipelineType::Compute => {
            let info = vk::ComputePipelineCreateInfo::default()
                .stage(stages[0])      // pass by value, not reference
                .layout(pipeline_layout);
            let pipelines = unsafe {
                device.create_compute_pipelines(vk::PipelineCache::null(), &[info], None)
            }.unwrap();
            pipelines[0]
        }
        PipelineType::Graphics { extent, render_pass, .. } => {
            let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
            let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

            let viewport = vk::Viewport {
                x: 0.0, y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
                min_depth: 0.0, max_depth: 1.0,
            };
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            };
            let viewports = [viewport];
            let scissors = [scissor];
            let viewport_state = vk::PipelineViewportStateCreateInfo::default()
                .viewports(&viewports)
                .scissors(&scissors);

            let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::NONE)
                .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
                .line_width(1.0);

            let multisample = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1);

            let color_blend = vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false);
            let color_blends = [color_blend];
            let blend_state = vk::PipelineColorBlendStateCreateInfo::default()
                .attachments(&color_blends);

            let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
                .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);

            let info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex_input)
                .input_assembly_state(&input_assembly)
                .viewport_state(&viewport_state)
                .rasterization_state(&rasterizer)
                .multisample_state(&multisample)
                .color_blend_state(&blend_state)
                .dynamic_state(&dynamic_state)
                .layout(pipeline_layout)
                .render_pass(render_pass)
                .subpass(0);

            let pipelines = unsafe {
                device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
            }.unwrap();
            pipelines[0]
        }
    };

    unsafe { device.destroy_pipeline_layout(pipeline_layout, None) };

    Ok((modules, pipeline))
}


// --------------------------------------------------------------------- / load shader

fn load_shader(device: &ash::Device, spv: &[u32]) -> Result<vk::ShaderModule, Box<dyn std::error::Error>> {
    let create_info = vk::ShaderModuleCreateInfo::default().code(spv);
    let module = unsafe { device.create_shader_module(&create_info, None)? };
    Ok(module)
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


// --------------------------------------------------------------------- / vulkan wrapper

pub fn vulkan_pipeline(
    vk_core: &VulkanCore,
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<VulkanTexture, Box<dyn std::error::Error>> {

    // copy raw image data to staging buffer
    let (buffer, memory) = create_buffer(
        &vk_core.instance,
        &vk_core.device,
        vk_core.physical_device,
        pixel_bytes,
    )?;

    // allocate vram for image data
    let (texture, vram) = create_texture(
        &vk_core.instance,
        &vk_core.device,
        vk_core.physical_device,
        width, height,
        vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
        vk::Format::R8G8B8A8_SRGB,
    )?;

    // load pixel data from buffer to texture
    load_texture(
        &vk_core.device,
        vk_core.graphics_queue,
        vk_core.command_pool,
        vk_core.command_buffer,
        buffer,
        texture,
        width,
        height,
    )?;

    // drop the staging buffer (no longer needed)
    unsafe {
        vk_core.device.destroy_buffer(buffer, None);
        vk_core.device.free_memory(memory, None);
    }

    Ok(VulkanTexture {
        image: texture,
        _memory: vram,
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
    usage: vk::ImageUsageFlags,
    format: vk::Format,
) -> Result<(vk::Image, vk::DeviceMemory), Box<dyn std::error::Error>> {

    // describe the image
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D { width, height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
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


// --------------------------------------------------------------------- / blur texture

pub fn blur_texture(
    vk_core: &VulkanCore,
    input_texture: &VulkanTexture,
    width: u32,
    height: u32,
) -> Result<VulkanTexture, Box<dyn std::error::Error>> {

    // create the output texture (same size, with STORAGE + TRANSFER_SRC)
    let (output_image, output_memory) = create_texture(
        &vk_core.instance,
        &vk_core.device,
        vk_core.physical_device,
        width, height,
        vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::STORAGE,
        vk::Format::R8G8B8A8_UNORM,
    )?;

    // create image views and a sampler
    let input_view = {
        let view_info = vk::ImageViewCreateInfo::default()
            .image(input_texture.image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_SRGB)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0, level_count: 1,
                base_array_layer: 0, layer_count: 1,
            });
        unsafe { vk_core.device.create_image_view(&view_info, None)? }
    };
    let output_view = {
        let view_info = vk::ImageViewCreateInfo::default()
            .image(output_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0, level_count: 1,
                base_array_layer: 0, layer_count: 1,
            });
        unsafe { vk_core.device.create_image_view(&view_info, None)? }
    };
    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
    let sampler = unsafe { vk_core.device.create_sampler(&sampler_info, None)? };

    // descriptor set (bindings 0=sampler, 1=storage image)
    let pool_sizes = [
        vk::DescriptorPoolSize { ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER, descriptor_count: 1 },
        vk::DescriptorPoolSize { ty: vk::DescriptorType::STORAGE_IMAGE, descriptor_count: 1 },
    ];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(&pool_sizes);
    let desc_pool = unsafe { vk_core.device.create_descriptor_pool(&pool_info, None)? };

    let set_layouts = [vk_core.blur_desc_layout];
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(desc_pool)
        .set_layouts(&set_layouts);
    let desc_sets = unsafe { vk_core.device.allocate_descriptor_sets(&alloc_info)? };
    let desc_set = desc_sets[0];

    let input_image_info = vk::DescriptorImageInfo::default()
        .sampler(sampler)
        .image_view(input_view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
    let output_image_info = vk::DescriptorImageInfo::default()
        .image_view(output_view)
        .image_layout(vk::ImageLayout::GENERAL);
    let input_image_infos = [input_image_info];
    let output_image_infos = [output_image_info];
    let write_descriptors = [
        vk::WriteDescriptorSet::default()
            .dst_set(desc_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&input_image_infos),
        vk::WriteDescriptorSet::default()
            .dst_set(desc_set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&output_image_infos),
    ];
    unsafe { vk_core.device.update_descriptor_sets(&write_descriptors, &[]) };

    // pipeline layout (no push constants, same descriptor layout as the pipeline)
    let set_layouts2 = [vk_core.blur_desc_layout];
    let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts2);
    let pipeline_layout = unsafe { vk_core.device.create_pipeline_layout(&pipeline_layout_info, None)? };

    // record compute commands
    unsafe { vk_core.device.reset_command_pool(vk_core.command_pool, vk::CommandPoolResetFlags::empty()) }?;
    let begin_info = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe { vk_core.device.begin_command_buffer(vk_core.command_buffer, &begin_info) }?;

    // transition output to GENERAL
    let barrier = vk::ImageMemoryBarrier::default()
        .image(output_image)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::GENERAL)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0, level_count: 1,
            base_array_layer: 0, layer_count: 1,
        });
    unsafe {
        vk_core.device.cmd_pipeline_barrier(
            vk_core.command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &[], &[], &[barrier],
        );
    }

    // bind pipeline and descriptor set
    unsafe {
        vk_core.device.cmd_bind_pipeline(vk_core.command_buffer, vk::PipelineBindPoint::COMPUTE, vk_core.blur_pipeline);
        vk_core.device.cmd_bind_descriptor_sets(
            vk_core.command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0, &[desc_set], &[],
        );
    }

    // dispatch
    let group_x = (width  + 15) / 16;
    let group_y = (height + 15) / 16;
    unsafe { vk_core.device.cmd_dispatch(vk_core.command_buffer, group_x, group_y, 1); }

    // transition output to SHADER_READ_ONLY_OPTIMAL for later blitting
    let barrier2 = vk::ImageMemoryBarrier::default()
        .image(output_image)
        .old_layout(vk::ImageLayout::GENERAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0, level_count: 1,
            base_array_layer: 0, layer_count: 1,
        });
    unsafe {
        vk_core.device.cmd_pipeline_barrier(
            vk_core.command_buffer,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[], &[], &[barrier2],
        );
    }
    unsafe { vk_core.device.end_command_buffer(vk_core.command_buffer) }?;

    // submit
    let command_buffers = [vk_core.command_buffer];
    let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
    let fence = unsafe { vk_core.device.create_fence(&vk::FenceCreateInfo::default(), None) }?;
    unsafe { vk_core.device.queue_submit(vk_core.graphics_queue, &[submit_info], fence) }?;
    unsafe { vk_core.device.wait_for_fences(&[fence], true, u64::MAX) }?;
    unsafe { vk_core.device.destroy_fence(fence, None) };

    // cleanup transient objects
    unsafe {
        vk_core.device.destroy_sampler(sampler, None);
        vk_core.device.destroy_image_view(input_view, None);
        vk_core.device.destroy_image_view(output_view, None);
        vk_core.device.destroy_descriptor_pool(desc_pool, None);
        vk_core.device.destroy_pipeline_layout(pipeline_layout, None);
    }

    Ok(VulkanTexture {
        image: output_image,
        _memory: output_memory,
    })
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
    blur_bg: Option<(vk::Image, u32, u32)>,
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

    // if the mode requires background, fill with blurred wallpaper or black
    if needs_clear {
        match blur_bg {
            Some((bg_image, bg_w, bg_h)) => {
                // Compute a cover-crop rectangle for the blurred background
                let bg_aspect = bg_w as f64 / bg_h as f64;
                let scr_aspect = swapchain_extent_width as f64 / swapchain_extent_height as f64;
                let (bg_sx, bg_sy, bg_sw, bg_sh) = if bg_aspect > scr_aspect {
                    // background wider → crop left/right (centered)
                    let new_w = (bg_h as f64 * scr_aspect) as u32;
                    let x = (bg_w - new_w) / 2;
                    (x, 0, new_w, bg_h)
                } else {
                    // background taller → crop top/bottom (centered)
                    let new_h = (bg_w as f64 / scr_aspect) as u32;
                    let y = (bg_h - new_h) / 2;
                    (0, y, bg_w, new_h)
                };

                // Transition bg image to TRANSFER_SRC_OPTIMAL
                let bg_barrier = vk::ImageMemoryBarrier::default()
                    .image(bg_image)
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_access_mask(vk::AccessFlags::SHADER_READ)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0, level_count: 1,
                        base_array_layer: 0, layer_count: 1,
                    });
                unsafe {
                    device.cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::TOP_OF_PIPE,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[], &[], &[bg_barrier],
                    );
                }

                // Blit the cropped background to cover the entire screen
                let bg_blit = vk::ImageBlit::default()
                    .src_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .src_offsets([
                        vk::Offset3D { x: bg_sx as i32, y: bg_sy as i32, z: 0 },
                        vk::Offset3D { x: (bg_sx + bg_sw) as i32, y: (bg_sy + bg_sh) as i32, z: 1 },
                    ])
                    .dst_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .dst_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: swapchain_extent_width as i32, y: swapchain_extent_height as i32, z: 1 },
                    ]);
                unsafe {
                    device.cmd_blit_image(
                        command_buffer,
                        bg_image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        target_image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[bg_blit],
                        vk::Filter::LINEAR,
                    );
                }
            }
            None => {
                // fallback to solid black
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

