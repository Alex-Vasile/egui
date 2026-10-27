#![expect(clippy::unwrap_used)] // TODO(emilk): avoid unwraps

use core::num::NonZeroU64;

use eframe::{
    egui_wgpu::wgpu::util::DeviceExt as _,
    egui_wgpu::{self, wgpu},
};

pub struct Custom3d {
    angle: f32,
}

impl Custom3d {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        // Get the WGPU render state from the eframe creation context. This can also be retrieved
        // from `eframe::Frame` when you don't have a `CreationContext` available.
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;

        let device = &wgpu_render_state.device;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("custom3d"),
            source: wgpu::ShaderSource::Wgsl(include_str!("./custom3d_wgpu_shader.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("custom3d"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("custom3d"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("custom3d"),
            contents: bytemuck::cast_slice(&[0.0_f32; 4]), // 16 bytes aligned!
            // Mapping at creation (as done by the create_buffer_init utility) doesn't require us to to add the MAP_WRITE usage
            // (this *happens* to workaround this bug )
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("custom3d"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // The render pipeline is created in `prepare` on first use, for the
        // `target_format` of the render pass the callback paints into: the
        // format is decided per viewport and can change when the window moves
        // between monitors, so it cannot be known at creation time.
        //
        // Because the graphics pipeline must have the same lifetime as the egui render pass,
        // instead of storing the pipeline in our `Custom3D` struct, we insert it into the
        // `paint_callback_resources` type map, which is stored alongside the render pass.
        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(TriangleRenderResources {
                pipeline: None,
                pipeline_layout,
                shader,
                bind_group,
                uniform_buffer,
            });

        Some(Self { angle: 0.0 })
    }
}

impl crate::DemoApp for Custom3d {
    fn demo_ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // TODO(emilk): Use `ScrollArea::content_margin`
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("The triangle is being painted using ");
                    ui.hyperlink_to("WGPU", "https://wgpu.rs");
                    ui.label(" (Portable Rust graphics API awesomeness)");
                });
                ui.label(
                    "It's not a very impressive demo, but it shows you can embed 3D inside of egui.",
                );

                egui::Frame::canvas(ui.style()).show(ui, |ui| {
                    self.custom_painting(ui);
                });
                ui.label("Drag to rotate!");
                ui.add(egui_demo_lib::egui_github_link_file!());
            });
        });
    }
}

// Callbacks in egui_wgpu have 3 stages:
// * prepare (per callback impl)
// * finish_prepare (once)
// * paint (per callback impl)
//
// The prepare callback is called every frame before paint and is given access to the wgpu
// Device and Queue, which can be used, for instance, to update buffers and uniforms before
// rendering.
// If [`egui_wgpu::Renderer`] has [`egui_wgpu::FinishPrepareCallback`] registered,
// it will be called after all `prepare` callbacks have been called.
// You can use this to update any shared resources that need to be updated once per frame
// after all callbacks have been processed.
//
// On both prepare methods you can use the main `CommandEncoder` that is passed-in,
// return an arbitrary number of user-defined `CommandBuffer`s, or both.
// The main command buffer, as well as all user-defined ones, will be submitted together
// to the GPU in a single call.
//
// The paint callback is called after finish prepare and is given access to egui's main render pass,
// which can be used to issue draw commands.
struct CustomTriangleCallback {
    angle: f32,
}

impl egui_wgpu::CallbackTrait for CustomTriangleCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut TriangleRenderResources = resources.get_mut().unwrap();
        resources.pipeline_for(device, target_format);
        resources.prepare(device, queue, self.angle);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        target_format: wgpu::TextureFormat,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &TriangleRenderResources = resources.get().unwrap();
        match resources.pipeline.as_ref() {
            Some((format, pipeline)) if *format == target_format => {
                resources.paint(render_pass, pipeline);
            }
            _ => {
                // `prepare` ran earlier in this frame and creates (or re-creates)
                // the pipeline for the target format, so this is unreachable in
                // practice.
                log::warn!("custom3d has no pipeline for target format {target_format:?}");
            }
        }
    }
}

impl Custom3d {
    fn custom_painting(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(300.0), egui::Sense::drag());

        self.angle += response.drag_motion().x * 0.01;
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            CustomTriangleCallback { angle: self.angle },
        ));
    }
}

struct TriangleRenderResources {
    /// The render pipeline for the format it was built for, created in
    /// `prepare` on first use and re-created when the target format changes.
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
}

impl TriangleRenderResources {
    /// Returns the pipeline for `target_format`, (re-)creating it if needed.
    fn pipeline_for(
        &mut self,
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        if self
            .pipeline
            .as_ref()
            .is_none_or(|(format, _)| *format != target_format)
        {
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("custom3d"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: None,
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(target_format.into())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            self.pipeline = Some((target_format, pipeline));
        }

        // Set above
        &self.pipeline.as_ref().unwrap().1
    }

    fn prepare(&self, _device: &wgpu::Device, queue: &wgpu::Queue, angle: f32) {
        // Update our uniform buffer with the angle from the UI
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[angle, 0.0, 0.0, 0.0]),
        );
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        // Draw our triangle!
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
