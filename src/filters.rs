// --------------------------------------------------------------------- / tittu
// wallbash
// a filter module for HyDE
//


// --------------------------------------------------------------------- / imports

use crate::vulkan::{VulkanCore, VulkanTexture, create_texture};
use std::error::Error;
use ash::vk;


// --------------------------------------------------------------------- / generic compute

pub fn compute_pipeline(
    device: &ash::Device,
    spv: &[u32],
    bindings: &[vk::DescriptorSetLayoutBinding],
) -> Result<(vk::ShaderModule, vk::Pipeline, vk::DescriptorSetLayout), Box<dyn Error>> {

    // Shader module
    let create_info = vk::ShaderModuleCreateInfo::default().code(spv);
    let module = unsafe { device.create_shader_module(&create_info, None)? };

    // Descriptor set layout
    let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(bindings);
    let desc_layout = unsafe { device.create_descriptor_set_layout(&layout_info, None)? };

    // Pipeline layout
    let set_layouts = [desc_layout];
    let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&pipeline_layout_info, None)? };

    // Compute pipeline
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(c"main");
    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);
    let pipelines = unsafe {
        device.create_compute_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
    }.expect("Failed to create compute pipeline");
    let pipeline = pipelines[0];

    unsafe { device.destroy_pipeline_layout(pipeline_layout, None) };
    Ok((module, pipeline, desc_layout))
}


// --------------------------------------------------------------------- / build filter

pub fn filter_pipeline(
    device: &ash::Device,
    filter: &str,
) -> Result<(vk::ShaderModule, vk::Pipeline, vk::DescriptorSetLayout), Box<dyn Error>> {
    match filter {
        "blur" => {
            let blur_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/blur.comp.spv"));
            let blur_words: Vec<u32> = blur_bytes
                .chunks_exact(4)
                .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

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
            compute_pipeline(device, &blur_words, &bindings)
        }
        _ => Err(format!("unknown filter: {}", filter).into()),
    }
}


// --------------------------------------------------------------------- / blur texture

pub fn blur_texture(
    vk_core: &VulkanCore,
    input_texture: &VulkanTexture,
    width: u32,
    height: u32,
    blur_pipeline: vk::Pipeline,
    blur_desc_layout: vk::DescriptorSetLayout,
) -> Result<VulkanTexture, Box<dyn Error>> {

    // create the output texture (same size, with STORAGE + TRANSFER_SRC)
    let (output_image, output_memory) = create_texture(
        &vk_core.instance,
        &vk_core.device,
        vk_core.physical_device,
        width,
        height,
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
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
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
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
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
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: 1,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_IMAGE,
            descriptor_count: 1,
        },
    ];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(&pool_sizes);
    let desc_pool = unsafe { vk_core.device.create_descriptor_pool(&pool_info, None)? };

    let set_layouts = [blur_desc_layout];
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
    let set_layouts2 = [blur_desc_layout];
    let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts2);
    let pipeline_layout = unsafe { vk_core.device.create_pipeline_layout(&pipeline_layout_info, None)? };

    // record compute commands
    unsafe {
        vk_core.device.reset_command_pool(vk_core.command_pool, vk::CommandPoolResetFlags::empty())?;
    }
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
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        vk_core.device.cmd_pipeline_barrier(
            vk_core.command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }

    // bind pipeline and descriptor set
    unsafe {
        vk_core.device.cmd_bind_pipeline(
            vk_core.command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            blur_pipeline,
        );
        vk_core.device.cmd_bind_descriptor_sets(
            vk_core.command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline_layout,
            0,
            &[desc_set],
            &[],
        );
    }

    // dispatch
    let group_x = (width + 15) / 16;
    let group_y = (height + 15) / 16;
    unsafe { vk_core.device.cmd_dispatch(vk_core.command_buffer, group_x, group_y, 1) };

    // transition output to SHADER_READ_ONLY_OPTIMAL for later blitting
    let barrier2 = vk::ImageMemoryBarrier::default()
        .image(output_image)
        .old_layout(vk::ImageLayout::GENERAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        vk_core.device.cmd_pipeline_barrier(
            vk_core.command_buffer,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier2],
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
        width,
        height,
    })
}

