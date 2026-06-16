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
    pub width: u32,
    pub height: u32,
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


// --------------------------------------------------------------------- / record commands

impl VulkanCore {
    pub(crate) fn record_commands(
        &self,
        f: impl FnOnce(vk::CommandBuffer),
    ) -> Result<(), Box<dyn std::error::Error>> {
        unsafe {
            self.device
                .reset_command_pool(self.command_pool, vk::CommandPoolResetFlags::empty())?;
        }

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device
                .begin_command_buffer(self.command_buffer, &begin_info)?;
        }

        f(self.command_buffer);
        unsafe {
            self.device.end_command_buffer(self.command_buffer)?;
        }

        let submit_info = vk::SubmitInfo::default()
            .command_buffers(std::slice::from_ref(&self.command_buffer));
        let fence = unsafe {
            self.device
                .create_fence(&vk::FenceCreateInfo::default(), None)?
        };
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)?;
            self.device
                .wait_for_fences(&[fence], true, u64::MAX)?;
            self.device.destroy_fence(fence, None);
        }

        Ok(())
    }
}


// --------------------------------------------------------------------- / memory type

impl VulkanCore {
    fn memory_type(
        &self,
        type_bits: u32,
        required_flags: vk::MemoryPropertyFlags,
    ) -> u32 {
        let props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        props.memory_types[..]
            .iter()
            .enumerate()
            .find(|(i, mt)| {
                (type_bits & (1 << i)) != 0
                    && mt.property_flags.contains(required_flags)
            })
            .map(|(i, _)| i as u32)
            .expect("No suitable memory type found")
    }
}


// --------------------------------------------------------------------- / image barrier

impl VulkanCore {
    fn image_barrier(
        &self,
        command_buffer: vk::CommandBuffer,
        image: vk::Image,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
        src_access: vk::AccessFlags,
        dst_access: vk::AccessFlags,
        src_stage: vk::PipelineStageFlags,
        dst_stage: vk::PipelineStageFlags,
    ) {
        let barrier = vk::ImageMemoryBarrier::default()
            .image(image)
            .old_layout(old_layout)
            .new_layout(new_layout)
            .src_access_mask(src_access)
            .dst_access_mask(dst_access)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0, level_count: 1,
                base_array_layer: 0, layer_count: 1,
            });
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                src_stage,
                dst_stage,
                vk::DependencyFlags::empty(),
                &[], &[],
                &[barrier],
            );
        }
    }
}


// --------------------------------------------------------------------- / fill background

impl VulkanCore {
    fn fill_background(
        &self,
        command_buffer: vk::CommandBuffer,
        target_image: vk::Image,
        background: (vk::Image, u32, u32),
        layer_width: u32,
        layer_height: u32,
    ) {
        let (bg_image, bg_w, bg_h) = background;

        let bg_aspect = bg_w as f64 / bg_h as f64;
        let scr_aspect = layer_width as f64 / layer_height as f64;
        let (bg_sx, bg_sy, bg_sw, bg_sh) = if bg_aspect > scr_aspect {
            let new_w = (bg_h as f64 * scr_aspect) as u32;
            let x = (bg_w - new_w) / 2;
            (x, 0, new_w, bg_h)
        } else {
            let new_h = (bg_w as f64 / scr_aspect) as u32;
            let y = (bg_h - new_h) / 2;
            (0, y, bg_w, new_h)
        };

        self.image_barrier(
            command_buffer,
            bg_image,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::AccessFlags::SHADER_READ,
            vk::AccessFlags::TRANSFER_READ,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        );

        let bg_blit = vk::ImageBlit::default()
            .src_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0, base_array_layer: 0, layer_count: 1,
            })
            .src_offsets([
                vk::Offset3D { x: bg_sx as i32, y: bg_sy as i32, z: 0 },
                vk::Offset3D { x: (bg_sx + bg_sw) as i32, y: (bg_sy + bg_sh) as i32, z: 1 },
            ])
            .dst_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0, base_array_layer: 0, layer_count: 1,
            })
            .dst_offsets([
                vk::Offset3D { x: 0, y: 0, z: 0 },
                vk::Offset3D { x: layer_width as i32, y: layer_height as i32, z: 1 },
            ]);
        unsafe {
            self.device.cmd_blit_image(
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
}


// --------------------------------------------------------------------- / create buffer

impl VulkanCore {
    pub fn create_buffer(
        &self,
        data: &[u8],
    ) -> Result<(vk::Buffer, vk::DeviceMemory), Box<dyn std::error::Error>> {
        let size = data.len() as u64;
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe { self.device.create_buffer(&buffer_info, None)? };
        let mem_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };

        let memory_type_index = self.memory_type(
            mem_requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { self.device.allocate_memory(&alloc_info, None)? };

        unsafe { self.device.bind_buffer_memory(buffer, memory, 0) }?;

        let ptr = unsafe {
            self.device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty())?
        } as *mut u8;
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len()) };
        unsafe { self.device.unmap_memory(memory) };

        Ok((buffer, memory))
    }
}


// --------------------------------------------------------------------- / create texture

impl VulkanCore {
    pub fn create_texture(
        &self,
        width: u32,
        height: u32,
        usage: vk::ImageUsageFlags,
        format: vk::Format,
    ) -> Result<(vk::Image, vk::DeviceMemory), Box<dyn std::error::Error>> {
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

        let image = unsafe { self.device.create_image(&image_info, None)? };
        let mem_requirements = unsafe { self.device.get_image_memory_requirements(image) };

        let memory_type_index = self.memory_type(
            mem_requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        );

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { self.device.allocate_memory(&alloc_info, None)? };

        unsafe { self.device.bind_image_memory(image, memory, 0) }?;

        Ok((image, memory))
    }
}


// --------------------------------------------------------------------- / load texture

impl VulkanCore {
    fn load_texture(
        &self,
        buffer: vk::Buffer,
        image: vk::Image,
        width: u32,
        height: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unsafe {
            self.device
                .reset_command_pool(self.command_pool, vk::CommandPoolResetFlags::empty())?;
        }

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device
                .begin_command_buffer(self.command_buffer, &begin_info)?;
        }

        // transition texture to transfer dst
        self.image_barrier(
            self.command_buffer,
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        );

        // copy buffer to image
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
            self.device.cmd_copy_buffer_to_image(
                self.command_buffer,
                buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }

        // transition texture to shader read only
        self.image_barrier(
            self.command_buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        );

        unsafe {
            self.device.end_command_buffer(self.command_buffer)?;
        }

        let submit_info = vk::SubmitInfo::default()
            .command_buffers(std::slice::from_ref(&self.command_buffer));
        let fence = unsafe {
            self.device
                .create_fence(&vk::FenceCreateInfo::default(), None)?
        };
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)?;
            self.device
                .wait_for_fences(&[fence], true, u64::MAX)?;
            self.device.destroy_fence(fence, None);
        }

        Ok(())
    }
}


// --------------------------------------------------------------------- / upload texture

impl VulkanCore {
    pub fn upload_texture(
        &self,
        pixel_bytes: &[u8],
        width: u32,
        height: u32,
    ) -> Result<VulkanTexture, Box<dyn std::error::Error>> {

        // copy raw image data to staging buffer
        let (buffer, memory) = self.create_buffer(pixel_bytes)?;

        // allocate vram for image data
        let (texture, vram) = self.create_texture(
            width,
            height,
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
            vk::Format::R8G8B8A8_SRGB,
        )?;

        // load pixel data from buffer to texture
        self.load_texture(buffer, texture, width, height)?;

        // drop the staging buffer (no longer needed)
        unsafe {
            self.device.destroy_buffer(buffer, None);
            self.device.free_memory(memory, None);
        }

        Ok(VulkanTexture {
            image: texture,
            _memory: vram,
            width,
            height,
        })
    }
}


// --------------------------------------------------------------------- / draw wallpaper

impl VulkanCore {
    pub fn draw_wallpaper(
        &self,
        surfchain: &VulkanSurfchain,
        texture: &VulkanTexture,
        layer_width: u32,
        layer_height: u32,
        anchor_x: f32,
        anchor_y: f32,
        background: Option<(vk::Image, u32, u32)>,
        mode: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let swapchain_loader = ash::khr::swapchain::Device::new(&self.instance, &self.device);
        let (image_index, _suboptimal) = match unsafe {
            swapchain_loader.acquire_next_image(
                surfchain.swapchain,
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
        let target_image = surfchain.swapchain_images[image_index as usize];

            self.record_commands(|command_buffer| {

                // transition texture to transfer source
                self.image_barrier(
                    command_buffer,
                    texture.image,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::AccessFlags::SHADER_READ,
                    vk::AccessFlags::TRANSFER_READ,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                );

                // transition swapchain image to transfer destination
                self.image_barrier(
                    command_buffer,
                    target_image,
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::AccessFlags::empty(),
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                );

                // compute source and destination rectangles based on mode
                let (src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h, needs_clear) =
                    mode_set(
                        texture.width, texture.height,
                        layer_width, layer_height,
                        anchor_x, anchor_y,
                        mode,
                    );

                if needs_clear {
                    let bg = background.expect("background must exist for fit/original mode");
                    self.fill_background(command_buffer, target_image, bg, layer_width, layer_height);
                }

                // blit main wallpaper
                let blit_region = vk::ImageBlit::default()
                    .src_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0, base_array_layer: 0, layer_count: 1,
                    })
                    .src_offsets([
                        vk::Offset3D { x: src_x as i32, y: src_y as i32, z: 0 },
                        vk::Offset3D { x: (src_x + src_w) as i32, y: (src_y + src_h) as i32, z: 1 },
                    ])
                    .dst_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0, base_array_layer: 0, layer_count: 1,
                    })
                    .dst_offsets([
                        vk::Offset3D { x: dst_x, y: dst_y, z: 0 },
                        vk::Offset3D { x: dst_x + dst_w as i32, y: dst_y + dst_h as i32, z: 1 },
                    ]);
                unsafe {
                    self.device.cmd_blit_image(
                        command_buffer,
                        texture.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        target_image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[blit_region],
                        vk::Filter::LINEAR,
                    );
                }

                // transition swapchain image to present layout
                self.image_barrier(
                    command_buffer,
                    target_image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::PRESENT_SRC_KHR,
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::AccessFlags::MEMORY_READ,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                );
            })?;

        // present the final image
        let present_info = vk::PresentInfoKHR::default()
            .swapchains(std::slice::from_ref(&surfchain.swapchain))
            .image_indices(std::slice::from_ref(&image_index));

        let result = unsafe { swapchain_loader.queue_present(self.graphics_queue, &present_info) };
        match result {
            Ok(_) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                Err("swapchain out of date (present)".into())
            }
            Err(vk::Result::SUBOPTIMAL_KHR) => Ok(()),
            Err(e) => Err(Box::new(e)),
        }
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


// --------------------------------------------------------------------- / destroy core

pub fn destroy_wallbash(
    vk_core: &VulkanCore,
    surfchain: Option<&mut VulkanSurfchain>,
    filter_module: Option<vk::ShaderModule>,
    filter_pipeline: Option<vk::Pipeline>,
    filter_desc_layout: Option<vk::DescriptorSetLayout>,
    level: u32,
) {

    // destroy filter resources if provided and level ≥ 0
    if let (Some(module), Some(pipeline), Some(desc)) = (filter_module, filter_pipeline, filter_desc_layout) {
        unsafe {
            vk_core.device.destroy_pipeline(pipeline, None);
            vk_core.device.destroy_descriptor_set_layout(desc, None);
            vk_core.device.destroy_shader_module(module, None);
        }
    }

    // destroy swapchain and surface (level ≥ 1)
    if level >= 1 {
        if let Some(sc) = surfchain {
            unsafe {
                let swapchain_loader = ash::khr::swapchain::Device::new(&vk_core.instance, &vk_core.device);
                swapchain_loader.destroy_swapchain(sc.swapchain, None);
                let surface_loader = ash::khr::surface::Instance::new(&vk_core.entry, &vk_core.instance);
                surface_loader.destroy_surface(sc.surface, None);
            }
        }
    }

    // destroy core Vulkan objects (level ≥ 2)
    if level >= 2 {
        unsafe {
            vk_core.device.device_wait_idle().expect("device wait failed");
            vk_core.device.destroy_command_pool(vk_core.command_pool, None);
            vk_core.device.destroy_device(None);
            vk_core.instance.destroy_instance(None);
        }
    }
}

