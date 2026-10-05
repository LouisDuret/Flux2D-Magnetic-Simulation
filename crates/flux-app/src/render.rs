//! Rendu du champ dans le canevas egui, via un callback wgpu.

use eframe::egui_wgpu::{self, wgpu};
use flux_solver::Field;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub size: f32,
    pub n: u32,
    pub delta_a: f32,
    pub b_max: f32,
    pub flags: u32,
    pub line_px: f32,
    pub pad: [f32; 2],
}

pub const FLAG_LINES: u32 = 1;
pub const FLAG_MAP: u32 = 2;
pub const FLAG_SRGB: u32 = 4;

pub struct FieldRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// Potentiel aux nœuds et groupe de liaisons, recréés quand la grille change.
    potential: Option<(usize, wgpu::Buffer, wgpu::BindGroup)>,
}

impl FieldRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> FieldRenderer {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flux field"),
            entries: &[entry(0, wgpu::BufferBindingType::Uniform), entry(1, wgpu::BufferBindingType::Storage { read_only: true })],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::include_wgsl!("field.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flux field"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        FieldRenderer { pipeline, layout, uniforms, potential: None }
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, field: &Field) {
        if field.a.is_empty() {
            return;
        }
        if self.potential.as_ref().is_none_or(|(len, ..)| *len != field.a.len()) {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 4 * field.a.len() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: buffer.as_entire_binding() },
                ],
            });
            self.potential = Some((field.a.len(), buffer, bind));
        }
        queue.write_buffer(&self.potential.as_ref().unwrap().1, 0, bytemuck::cast_slice(&field.a));
    }
}

pub struct FieldCallback(pub Uniforms);

impl egui_wgpu::CallbackTrait for FieldCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(r) = resources.get::<FieldRenderer>() {
            queue.write_buffer(&r.uniforms, 0, bytemuck::bytes_of(&self.0));
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(r) = resources.get::<FieldRenderer>()
            && let Some((_, _, bind)) = &r.potential
        {
            pass.set_pipeline(&r.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
