// --------------------------------------------------------------------- / tittu
// wallbash
// a transition module for HyDE
//


// --------------------------------------------------------------------- / imports

use crate::{
    filters::{image_view, linear_sampler},
    vulkan::{VulkanCore, VulkanTexture},
};
use ash::vk;
use std::{error::Error, time::Instant};


// --------------------------------------------------------------------- / datatypes

pub struct CubicBezier {
    x1: f32, y1: f32,
    x2: f32, y2: f32,
}

pub struct TransitionConfig {
    pub kind: String,
    pub duration_ms: u64,
    pub bezier: CubicBezier,
}

pub enum TransitionResources {
    None,
    Zoom(ZoomResources),
}

pub struct TransitionCore {
    pub cfg: TransitionConfig,
    pub prev: VulkanTexture,
    pub start: Instant,
    pub total_frames: u32,
    pub current_frame: u32,
    pub scale: String,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub background: Option<VulkanTexture>,
    pub resources: TransitionResources,
}

pub struct ZoomResources {
    sampler: vk::Sampler,
    prev_view: vk::ImageView,
    next_view: vk::ImageView,
    output_view: vk::ImageView,
    output_image: vk::Image,
    desc_pool: vk::DescriptorPool,
    desc_set: vk::DescriptorSet,
    old_w: f32, old_h: f32,
    new_w: f32, new_h: f32,
    dst_w: f32, dst_h: f32,
}

pub struct TransitionNone;

pub struct TransitionPipeline {
    pub module: vk::ShaderModule,
    pub pipeline: vk::Pipeline,
    pub desc_layout: vk::DescriptorSetLayout,
    pub pipe_layout: vk::PipelineLayout,
}

pub struct TransitionZoom {
    pipeline: TransitionPipeline,
}

pub struct TransitionRegistry {
    transitions: Vec<Box<dyn Transition>>,
}


// --------------------------------------------------------------------- / bezier

impl CubicBezier {
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    pub fn apply(&self, t: f32) -> f32 {
        if t <= 0.0 { return 0.0; }
        if t >= 1.0 { return 1.0; }
        let mut s = t;
        for _ in 0..8 {
            let x   = Self::sample(self.x1, self.x2, s) - t;
            let dx  = Self::sample_dx(self.x1, self.x2, s);
            if dx.abs() < 1e-6 { break; }
            s -= x / dx;
            s  = s.clamp(0.0, 1.0);
        }
        Self::sample(self.y1, self.y2, s)
    }

    fn sample(a: f32, b: f32, s: f32) -> f32 {
        let s2 = s * s;
        let s3 = s2 * s;
        let t  = 1.0 - s;
        3.0 * t * t * s * a + 3.0 * t * s2 * b + s3
    }

    fn sample_dx(a: f32, b: f32, s: f32) -> f32 {
        let s2 = s * s;
        let t  = 1.0 - s;
        3.0 * t * t * a + 6.0 * t * s * (b - a) + 3.0 * s2 * (1.0 - b)
    }
}

impl TransitionConfig {
    pub fn parse(kind: &str, duration_ms: u64, bezier_str: &str) -> Self {
        let bezier = if bezier_str.is_empty() {
            CubicBezier::new(0.0, 0.0, 1.0, 1.0)
        } else {
            let parts: Vec<f32> = bezier_str.split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            if parts.len() == 4 {
                CubicBezier::new(
                    parts[0].clamp(0.0, 1.0), parts[1],
                    parts[2].clamp(0.0, 1.0), parts[3],
                )
            } else {
                CubicBezier::new(0.0, 0.0, 1.0, 1.0)
            }
        };
        Self { kind: kind.to_string(), duration_ms, bezier }
    }
}


// --------------------------------------------------------------------- / pipeline

impl TransitionPipeline {

    pub fn new(device: &ash::Device, spv: &[u32]) -> Result<Self, Box<dyn Error>> {
        let module = unsafe {
            device.create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(spv), None,
            )?
        };

        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let desc_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None,
            )?
        };

        let push_range = vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::COMPUTE,
            offset: 0,
            size: 44,
        };
        let set_layouts = [desc_layout];
        let pipe_layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(std::slice::from_ref(&push_range)),
                None,
            )?
        };

        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(module)
            .name(c"main");
        let pipeline_info = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(pipe_layout);
        let pipelines = unsafe {
            device.create_compute_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
        }.map_err(|(_, e)| e)?;

        Ok(Self { module, pipeline: pipelines[0], desc_layout, pipe_layout })
    }

    pub fn destroy(&self, device: &ash::Device) {
        unsafe {
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline_layout(self.pipe_layout, None);
            device.destroy_descriptor_set_layout(self.desc_layout, None);
            device.destroy_shader_module(self.module, None);
        }
    }
}


// --------------------------------------------------------------------- / interface

pub trait Transition: Send + Sync {
    fn name(&self) -> &'static str;

    fn prepare(
        &self,
        vk_core: &VulkanCore,
        prev: &VulkanTexture,
        next: &VulkanTexture,
        output: &VulkanTexture,
    ) -> Result<TransitionResources, Box<dyn Error>>;

    fn render_frame(
        &self,
        vk_core: &VulkanCore,
        resources: &TransitionResources,
        t: f32,
        mode: &str,
        anchor_x: f32,
        anchor_y: f32,
    ) -> Result<(), Box<dyn Error>>;

    fn cleanup(&self, device: &ash::Device, resources: TransitionResources);
    fn destroy(&self, device: &ash::Device);
}


// --------------------------------------------------------------------- / instant cut

impl Transition for TransitionNone {
    fn name(&self) -> &'static str { "none" }
    fn destroy(&self, _: &ash::Device) {}
    fn cleanup(&self, _device: &ash::Device, _resources: TransitionResources) {}

    fn prepare(
        &self,
        vk_core: &VulkanCore,
        _prev: &VulkanTexture,
        next: &VulkanTexture,
        output: &VulkanTexture,
    ) -> Result<TransitionResources, Box<dyn Error>> {
        vk_core.record_commands(|cmd| {

            vk_core.image_barrier(cmd, next.image,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::AccessFlags::SHADER_READ, vk::AccessFlags::TRANSFER_READ,
                vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::TRANSFER,
            );
            vk_core.image_barrier(cmd, output.image,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::AccessFlags::SHADER_READ, vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::TRANSFER,
            );

            let region = vk::ImageCopy::default()
                .src_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                .dst_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                .extent(vk::Extent3D { width: next.width, height: next.height, depth: 1 });
            unsafe {
                vk_core.device.cmd_copy_image(
                    cmd,
                    next.image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    output.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            vk_core.image_barrier(cmd, next.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::AccessFlags::TRANSFER_READ, vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::COMPUTE_SHADER,
            );
            vk_core.image_barrier(cmd, output.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::COMPUTE_SHADER,
            );
        })?;

        Ok(TransitionResources::None)
    }

    fn render_frame(
        &self,
        _vk_core: &VulkanCore,
        _resources: &TransitionResources,
        _t: f32,
        _mode: &str,
        _anchor_x: f32,
        _anchor_y: f32,
    ) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}


// --------------------------------------------------------------------- / zoom

impl TransitionZoom {
    fn new(device: &ash::Device) -> Result<Self, Box<dyn Error>> {
        let spv = spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/zoom.comp.spv")));
        Ok(Self { pipeline: TransitionPipeline::new(device, &spv)? })
    }

    fn dispatch_frame(
        &self,
        vk_core: &VulkanCore,
        res: &ZoomResources,
        t: f32,
        max_zoom: f32,
        mode: &str,
        anchor_x: f32,
        anchor_y: f32,
    ) -> Result<(), Box<dyn Error>> {
        let device = &vk_core.device;
        let w = res.dst_w as u32;
        let h = res.dst_h as u32;

        vk_core.record_commands(|cmd| {
            vk_core.image_barrier(cmd, res.output_image,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::GENERAL,
                vk::AccessFlags::SHADER_READ, vk::AccessFlags::SHADER_WRITE,
                vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER,
            );

            unsafe {
                device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, self.pipeline.pipeline);
                device.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, self.pipeline.pipe_layout, 0, &[res.desc_set], &[]);

                let mode_id: i32 = match mode {
                    "cover"    => 0,
                    "fit"      => 1,
                    "original" => 2,
                    _          => 0,
                };
                let mut push_data = [0u8; 44];
                push_data[ 0.. 4].copy_from_slice(&t.to_ne_bytes());
                push_data[ 4.. 8].copy_from_slice(&max_zoom.to_ne_bytes());
                push_data[ 8..12].copy_from_slice(&res.old_w.to_ne_bytes());
                push_data[12..16].copy_from_slice(&res.old_h.to_ne_bytes());
                push_data[16..20].copy_from_slice(&res.new_w.to_ne_bytes());
                push_data[20..24].copy_from_slice(&res.new_h.to_ne_bytes());
                push_data[24..28].copy_from_slice(&res.dst_w.to_ne_bytes());
                push_data[28..32].copy_from_slice(&res.dst_h.to_ne_bytes());
                push_data[32..36].copy_from_slice(&mode_id.to_ne_bytes());
                push_data[36..40].copy_from_slice(&anchor_x.to_ne_bytes());
                push_data[40..44].copy_from_slice(&anchor_y.to_ne_bytes());

                device.cmd_push_constants(cmd, self.pipeline.pipe_layout, vk::ShaderStageFlags::COMPUTE, 0, &push_data);
                device.cmd_dispatch(cmd, (w + 15) / 16, (h + 15) / 16, 1);
            }

            vk_core.image_barrier(cmd, res.output_image,
                vk::ImageLayout::GENERAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::AccessFlags::SHADER_WRITE, vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER,
            );
        })?;

        Ok(())
    }
}

impl Transition for TransitionZoom {
    fn name(&self) -> &'static str { "zoom" }
    fn destroy(&self, device: &ash::Device) { self.pipeline.destroy(device); }

    fn prepare(
        &self,
        vk_core: &VulkanCore,
        prev: &VulkanTexture,
        next: &VulkanTexture,
        output: &VulkanTexture,
    ) -> Result<TransitionResources, Box<dyn Error>> {
        let device = &vk_core.device;
        let sampler = linear_sampler(device)?;

        let prev_view = image_view(device, prev.image,   vk::Format::R8G8B8A8_SRGB)?;
        let next_view = image_view(device, next.image,   vk::Format::R8G8B8A8_SRGB)?;
        let output_view = image_view(device, output.image, vk::Format::R8G8B8A8_UNORM)?;

        let pool_sizes = [
            vk::DescriptorPoolSize { ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER, descriptor_count: 2 },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::STORAGE_IMAGE,          descriptor_count: 1 },
        ];
        let desc_pool = unsafe {
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&pool_sizes), None,
            )?
        };
        let set_layouts = [self.pipeline.desc_layout];
        let desc_set = unsafe {
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(desc_pool)
                    .set_layouts(&set_layouts),
            )?[0]
        };

        let prev_info = vk::DescriptorImageInfo::default().sampler(sampler).image_view(prev_view).image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let next_info = vk::DescriptorImageInfo::default().sampler(sampler).image_view(next_view).image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let output_info = vk::DescriptorImageInfo::default().image_view(output_view).image_layout(vk::ImageLayout::GENERAL);
        let prev_infos = [prev_info];
        let next_infos = [next_info];
        let output_infos = [output_info];
        let writes = [
            vk::WriteDescriptorSet::default().dst_set(desc_set).dst_binding(0).descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).image_info(&prev_infos),
            vk::WriteDescriptorSet::default().dst_set(desc_set).dst_binding(1).descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).image_info(&next_infos),
            vk::WriteDescriptorSet::default().dst_set(desc_set).dst_binding(2).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).image_info(&output_infos),
        ];
        unsafe { device.update_descriptor_sets(&writes, &[]); }

        Ok(TransitionResources::Zoom(ZoomResources {
            sampler, prev_view, next_view, output_view,
            output_image: output.image,
            desc_pool, desc_set,
            old_w: prev.width as f32, old_h: prev.height as f32,
            new_w: next.width as f32, new_h: next.height as f32,
            dst_w: output.width as f32, dst_h: output.height as f32,
        }))
    }

    fn render_frame(
        &self,
        vk_core: &VulkanCore,
        resources: &TransitionResources,
        t: f32,
        mode: &str,
        anchor_x: f32,
        anchor_y: f32,
    ) -> Result<(), Box<dyn Error>> {
        let res = match resources {
            TransitionResources::Zoom(r) => r,
            _ => return Err("zoom_focus: prepare() was not called with matching resources".into()),
        };
        self.dispatch_frame(vk_core, res, t, 0.2, mode, anchor_x, anchor_y)
    }

    fn cleanup(&self, device: &ash::Device, resources: TransitionResources) {
        if let TransitionResources::Zoom(r) = resources {
            unsafe {
                device.destroy_descriptor_pool(r.desc_pool, None);
                device.destroy_image_view(r.prev_view, None);
                device.destroy_image_view(r.next_view, None);
                device.destroy_image_view(r.output_view, None);
                device.destroy_sampler(r.sampler, None);
            }
        }
    }
}


// --------------------------------------------------------------------- / factory

impl TransitionRegistry {
    pub fn new(device: &ash::Device) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            transitions: vec![
                Box::new(TransitionZoom::new(device)?),
            ],
        })
    }

    pub fn get(&self, name: &str) -> &dyn Transition {
        self.transitions.iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
            .unwrap_or(&TransitionNone)
    }

    pub fn destroy(&self, device: &ash::Device) {
        for t in &self.transitions { t.destroy(device); }
    }
}

fn spv_words(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4)
        .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

