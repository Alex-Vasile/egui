//! The paint-callback path through the offscreen harness: a callback receives
//! the actual render-pass `target_format`, lazily builds its pipeline for it
//! (the per-format pattern the windowed callbacks use), and the pixels it
//! writes end up in the readback and the snapshot.

#![cfg(feature = "snapshot")]
#![cfg(feature = "wgpu")]

use egui::Vec2;
use egui_kittest::Harness;
use image::Rgba;

/// What the callback's constant color (`vec4<f32>(0.2, 0.4, 0.8, 1.0)`,
/// hardcoded in the shader below) becomes in the `Rgba8Unorm` offscreen
/// target the harness renders into (raw values: unorm conversion of
/// `f * 255`).
///
/// Kept in-gamut on purpose: u8 goldens are blind to anything above 1.0, so
/// a >1.0 color would make the golden meaningless.
const CALLBACK_COLOR_U8: Rgba<u8> = Rgba([51, 102, 204, 255]);

#[test]
fn wgpu_paint_callback_matches_target_format() {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(64.0, 64.0))
        .build_ui(|ui| {
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                ui.ctx().content_rect(),
                SolidQuadCallback,
            ));
        });

    let image = harness.render().expect("the wgpu render should succeed");

    // The callback must own every pixel of the canvas: check the center and
    // the four corners, well clear of any scissor rounding at the edges.
    let mut points = vec![(image.width() / 2, image.height() / 2)];
    for (x, y) in [(8, 8), (56, 8), (8, 56), (56, 56)] {
        points.push((x, y));
    }
    for (x, y) in points {
        let pixel = image.get_pixel(x, y);
        assert_eq!(
            pixel, &CALLBACK_COLOR_U8,
            "pixel ({x}, {y}) must be the callback's color — the one-format pipeline \
             built for the render pass's actual target format painted it"
        );
    }

    harness.snapshot("wgpu_paint_callback");
}

struct SolidResources {
    pipeline: Option<(
        egui_wgpu::wgpu::TextureFormat,
        egui_wgpu::wgpu::RenderPipeline,
    )>,
    shader: egui_wgpu::wgpu::ShaderModule,
}

impl SolidResources {
    /// Returns the pipeline for `target_format`, (re-)creating it if needed.
    fn pipeline_for(
        &mut self,
        device: &egui_wgpu::wgpu::Device,
        target_format: egui_wgpu::wgpu::TextureFormat,
    ) -> &egui_wgpu::wgpu::RenderPipeline {
        use egui_wgpu::wgpu;

        if self
            .pipeline
            .as_ref()
            .is_none_or(|(format, _)| *format != target_format)
        {
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("kittest_paint_callback"),
                // No bindings — the shader needs nothing but the vertex index.
                layout: None,
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
        &self.pipeline.as_ref().expect("pipeline was just set").1
    }
}

struct SolidQuadCallback;

impl egui_wgpu::CallbackTrait for SolidQuadCallback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        _queue: &egui_wgpu::wgpu::Queue,
        target_format: egui_wgpu::wgpu::TextureFormat,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        use egui_wgpu::wgpu;

        // First use: insert the resources for the paint step. The pipeline
        // itself is created per target format.
        if resources.get_mut::<SolidResources>().is_none() {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kittest_paint_callback"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
            resources.insert(SolidResources {
                pipeline: None,
                shader,
            });
        }

        let resources: &mut SolidResources = resources
            .get_mut()
            .expect("resources were just inserted above");
        resources.pipeline_for(device, target_format);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        target_format: egui_wgpu::wgpu::TextureFormat,
        render_pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &SolidResources = resources
            .get()
            .expect("prepare ran earlier in this frame and inserted the resources");
        match resources.pipeline.as_ref() {
            Some((format, pipeline)) if *format == target_format => {
                render_pass.set_pipeline(pipeline);
                render_pass.draw(0..6, 0..1);
            }
            _ => {
                // `prepare` ran earlier in this frame and created the
                // pipeline for the target format, so this is unreachable.
                log::warn!("kittest callback has no pipeline for target format {target_format:?}");
            }
        }
    }
}

const SHADER: &str = r"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
};

// Fullscreen quad from two triangles; the egui render pass scissors it to the
// callback rect.
var<private> QUAD: array<vec2<f32>, 6> = array(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(QUAD[vertex_index] * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.2, 0.4, 0.8, 1.0);
}
";
