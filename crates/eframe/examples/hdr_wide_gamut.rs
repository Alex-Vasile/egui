//! Wide-gamut callback demo: saturation outside the sRGB gamut.
//!
//! Opts into [`SurfacePreference::WideGamut`] and paints a large quad through
//! a wgpu callback whose pipeline is built lazily against whatever
//! `target_format` the callback receives — the same per-format pattern as the
//! `custom3d` demo. The quad draws the surface's native primaries as
//! full-range bytes: (255, 0, 0), (0, 255, 0), (0, 0, 255). On a P3-referred
//! surface those bytes ARE the Display P3 primaries and display outside the
//! sRGB gamut; an SDR fallback surface displays the same bytes as the
//! ordinary sRGB primaries. (The P3 primaries are *defined* by their xy
//! chromaticities — (0.680, 0.320) etc. Those are coordinates in the CIE
//! chromaticity diagram, not colors: encoding them as linear RGB and applying
//! the sRGB transfer yields mixes like the orange (215, 153, 0) on any
//! surface.)
//!
//! The side panel shows the live surface decision
//! ([`egui::ViewportInfo::surface_status`] / `surface_target`) plus an
//! "sRGB red (converted)" vs "P3 red (native)" swatch row — the same physical
//! red written once in the surface's P3-referred encoding (≈ (234, 51, 35))
//! and once as the native primary (255, 0, 0). On a P3 display the native
//! primary is visibly deeper; on an SDR fallback both swatches just display
//! their raw bytes, so machines without a P3 display can still tell which
//! path produced which bytes.
//!
//! The effect needs a P3-capable display. Everything here stays SDR — the u8
//! surface clamps at 1.0; HDR brightness is PR 3.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![expect(rustdoc::missing_crate_level_docs, clippy::unwrap_used)] // it's an example

use eframe::egui;
use eframe::egui_wgpu::{self, SurfacePreference, wgpu};

fn main() -> Result<(), eframe::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,naga=warn,wgpu_core=warn,wgpu_hal=warn"),
    )
    .init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // The fork opts in unconditionally (plan §2).
            surface_preference: SurfacePreference::WideGamut,
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "hdr_wide_gamut",
        options,
        Box::new(|cc| {
            Ok(Box::new(
                WideGamutApp::new(cc)
                    .expect("the wgpu render state must be available on the native wgpu path"),
            ) as Box<dyn eframe::App>)
        }),
    )
}

struct WideGamutApp {
    /// sRGB red mapped into the surface's P3-referred encoding, computed in
    /// [`Self::new`] from [`SRGB_TO_P3_LINEAR`] — bytes (234, 51, 35).
    srgb_red_converted: egui::Color32,
}

impl WideGamutApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;

        let shader = wgpu_render_state
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("hdr_wide_gamut"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });

        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(QuadRenderResources {
                pipeline: None,
                shader,
            });

        Some(Self {
            srgb_red_converted: srgb_red_converted_to_p3(),
        })
    }
}

impl eframe::App for WideGamutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx();

        let (status, target) = ctx.input(|input| {
            let info = input.raw.viewports.get(&egui::ViewportId::ROOT);
            (
                info.and_then(|v| v.surface_status),
                info.and_then(|v| v.surface_target.clone()),
            )
        });

        egui::CentralPanel::default().show(ui, |ui| {
            // The canvas gets the top 60% of the panel: sizing it to the full
            // available space would push every widget after it past the clip
            // rect, invisibly.
            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    ui.available_size() * egui::vec2(1.0, 0.6),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .add(egui_wgpu::Callback::new_paint_callback(rect, WideGamutQuad));
            });

            ui.label(format!("Surface status: {status:?}"));
            ui.label(format!(
                "Surface target: {}",
                target.as_deref().unwrap_or("(unknown)")
            ));
            ui.label(
                "The quad above holds the surface's native primaries as \
                 full-range bytes. On a P3-referred surface they display \
                 outside the sRGB gamut; on an SDR fallback the same bytes \
                 read as ordinary sRGB primaries. Needs a P3-capable display.",
            );
            ui.separator();
            let swatch_row = ui.horizontal(|ui| {
                // sRGB red (1, 0, 0) converted into the surface's P3-referred
                // encoding: the bytes that make a P3 surface show the
                // ordinary sRGB red.
                swatch(ui, "sRGB red (converted)", self.srgb_red_converted);
                // The native primary the quad's red band writes — the same
                // bytes on any surface, deeper only on a P3 panel.
                swatch(ui, "P3 red (native)", egui::Color32::from_rgb(255, 0, 0));
            });

            // Layout probe: the readout must lay out inside the visible
            // screen — a canvas consuming the whole panel would push every
            // widget after it below the window, invisibly (egui's panel rect
            // itself expands with overflowing content, so the screen rect is
            // the only honest boundary). The swatch row is the last widget,
            // so its placed rect is the canary.
            let row_rect = swatch_row.response.rect;
            let screen = ui.ctx().content_rect();
            assert!(
                row_rect.is_finite() && row_rect.height() > 0.0 && screen.contains_rect(row_rect),
                "the status readout must lay out on screen \
                 (swatch row {row_rect:?}, screen {screen:?})"
            );
            log::info!(
                "hdr_wide_gamut readout laid out: swatch row {row_rect:?} on screen {screen:?}"
            );
        });
    }
}

fn swatch(ui: &mut egui::Ui, name: &str, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, color);
    ui.label(name);
}

/// Linear-light sRGB → Display P3 conversion, both D65-referred: the product
/// `M_P3⁻¹ · M_sRGB` of each system's RGB→XYZ matrix (built from the CIE xy
/// chromaticities of the primaries and the D65 white point (0.3127, 0.3290)).
/// Published verbatim in CSS Color 4 ("predefined-display-p3"); rows sum to 1
/// because both systems share the D65 white. Column 1 applied to sRGB red
/// (1, 0, 0) gives the P3-linear color (0.82246, 0.03319, 0.01708).
const SRGB_TO_P3_LINEAR: [[f32; 3]; 3] = [
    [0.822_461_97, 0.177_538_03, 0.0],
    [0.033_194_2, 0.966_805_8, 0.0],
    [0.017_082_63, 0.072_397_44, 0.910_519_93],
];

/// The sRGB transfer function (IEC 61966-2-1), same as the shader's
/// `srgb_encode`: the u8 surface formats are non-Srgb, so values written to
/// them must arrive already encoded.
fn srgb_encode(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        12.92 * channel
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB red expressed in the P3-referred encoding: bytes (234, 51, 35).
fn srgb_red_converted_to_p3() -> egui::Color32 {
    // Column 1 of the matrix: sRGB red (1, 0, 0) in P3-linear terms.
    let linear = [
        SRGB_TO_P3_LINEAR[0][0],
        SRGB_TO_P3_LINEAR[1][0],
        SRGB_TO_P3_LINEAR[2][0],
    ];
    let [r, g, b] = linear.map(|c| (srgb_encode(c) * 255.0).round() as u8);
    egui::Color32::from_rgb(r, g, b)
}

struct QuadRenderResources {
    /// The render pipeline for the format it was built for, created in
    /// `prepare` on first use and re-created when the target format changes
    /// (the window may have moved to a different monitor).
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    shader: wgpu::ShaderModule,
}

impl QuadRenderResources {
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
                label: Some("hdr_wide_gamut"),
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
        &self.pipeline.as_ref().unwrap().1
    }
}

struct WideGamutQuad;

impl egui_wgpu::CallbackTrait for WideGamutQuad {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut QuadRenderResources = resources.get_mut().unwrap();
        resources.pipeline_for(device, target_format);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        target_format: wgpu::TextureFormat,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &QuadRenderResources = resources.get().unwrap();
        match resources.pipeline.as_ref() {
            Some((format, pipeline)) if *format == target_format => {
                render_pass.set_pipeline(pipeline);
                render_pass.draw(0..6, 0..1);
            }
            _ => {
                // `prepare` ran earlier in this frame and created (or
                // re-created) the pipeline for the target format, so this is
                // unreachable in practice.
                log::warn!("hdr_wide_gamut has no pipeline for target format {target_format:?}");
            }
        }
    }
}

const SHADER: &str = r"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
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

// The surface's native primaries as full-range linear values: on a
// P3-referred surface these bytes are the Display P3 primaries, on an SDR
// fallback the same bytes are the sRGB primaries. These are NOT the P3 xy
// chromaticities (0.680, 0.320) etc. — those are CIE chromaticity-diagram
// coordinates that define the primaries, not colors.
var<private> PRIMARIES: array<vec3<f32>, 3> = array(
    vec3<f32>(1.0, 0.0, 0.0), // red
    vec3<f32>(0.0, 1.0, 0.0), // green
    vec3<f32>(0.0, 0.0, 1.0), // blue
);

// The sRGB transfer function (IEC 61966-2-1). The u8 surface format is
// non-Srgb, so anything written to it must arrive pre-encoded; on the
// primaries themselves it is the identity (0.0 and 1.0 are fixed points),
// kept so real content dropped in later encodes correctly.
fn srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let low = vec3<f32>(12.92) * linear;
    let high = vec3<f32>(1.055) * pow(linear, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(low, high, linear > vec3<f32>(0.0031308));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var out: VertexOut;
    let uv = QUAD[vertex_index];
    out.position = vec4<f32>(uv * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let band = min(u32(in.uv.x * 3.0), 2u);
    return vec4<f32>(srgb_encode(PRIMARIES[band]), 1.0);
}
";
