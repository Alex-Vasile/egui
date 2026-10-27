//! Wide-gamut callback demo: saturation outside the sRGB gamut.
//!
//! Opts into [`SurfacePreference::WideGamut`] and paints a 2×3 grid of
//! primaries through a wgpu callback whose pipeline is built lazily against
//! whatever `target_format` the callback receives — the same per-format
//! pattern as the `custom3d` demo. The bottom row is the surface's native
//! primaries as full-range bytes; the top row is the three sRGB primaries
//! converted at runtime into whatever color space the surface was actually
//! granted. The grant is parsed from the `surface_target` diagnostics
//! string ([`egui::ViewportInfo::surface_target`], e.g.
//! "`Bgra8Unorm + DisplayP3`"): PR 1's structured per-viewport decision is
//! not app-reachable — the painter accessor lives below eframe — so that
//! string is the channel, and its color-space suffix maps 1:1 onto the
//! conversion table. `Srgb` and unrecognized spaces convert with the
//! identity matrix (unrecognized says so in the hint).
//!
//! That makes the grid a self-contained check: identical rows mean the
//! surface is not wide-gamut (the bytes are re-read as-is); the top row
//! visibly muted relative to the bottom means wide gamut is working — on a
//! P3-referred surface the bottom row IS the Display P3 primaries, outside
//! the sRGB gamut. (The P3 primaries are *defined* by their xy
//! chromaticities — (0.680, 0.320) etc. Those are coordinates in the CIE
//! chromaticity diagram, not colors: encoding them as linear RGB and
//! applying the sRGB transfer yields mixes like the orange (215, 153, 0) on
//! any surface.)
//!
//! The canvas is sandwiched between bold "sRGB primaries" / "display
//! primaries" labels; below it the panel shows the live surface decision
//! ([`egui::ViewportInfo::surface_status`] / `surface_target`) and a hint.
//!
//! Everything here stays SDR — the u8 surface clamps at 1.0; HDR brightness
//! is PR 3. The wide-gamut effect needs a P3-capable display.

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

struct WideGamutApp;

impl WideGamutApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;

        let shader = wgpu_render_state
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("hdr_wide_gamut"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });

        // The six grid cell colors travel to the fragment shader through a
        // small uniform buffer so the conversion can follow the granted
        // color space at runtime (see [`GrantedSpace`]).
        let cells_bgl =
            wgpu_render_state
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("hdr_wide_gamut cells layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: Some(wgpu::BufferSize::new(96).unwrap()),
                        },
                        count: None,
                    }],
                });
        let cells = wgpu_render_state
            .device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("hdr_wide_gamut cells"),
                size: 96,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            });
        let cells_bind_group =
            wgpu_render_state
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hdr_wide_gamut cells"),
                    layout: &cells_bgl,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &cells,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });
        let pipeline_layout =
            wgpu_render_state
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("hdr_wide_gamut"),
                    bind_group_layouts: &[Some(&cells_bgl)],
                    immediate_size: 0,
                });

        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(QuadRenderResources {
                pipeline: None,
                shader,
                pipeline_layout,
                cells,
                cells_bind_group,
                uploaded: [egui::Color32::BLACK; 6],
            });

        Some(Self)
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

        let space = GrantedSpace::from_target(target.as_deref());

        egui::CentralPanel::default().show(ui, |ui| {
            // The canvas IS the 2×3 grid: it takes the top ~60% of the panel
            // (sizing it to the full available space would push every widget
            // after it past the clip rect, invisibly) and the shader splits
            // it pixel-flush into six full-size cells. The bold labels
            // sandwich it, naming the rows.
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("sRGB primaries").strong());
            });
            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    ui.available_size() * egui::vec2(1.0, 0.6),
                    egui::Sense::hover(),
                );
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    WideGamutQuad {
                        cells: grid_cells(space),
                    },
                ));
            });
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("display primaries").strong());
            });

            ui.label(format!("Surface status: {status:?}"));
            ui.label(format!(
                "Surface target: {}",
                target.as_deref().unwrap_or("(unknown)")
            ));
            let mut hint = String::from(
                "The check: identical rows mean no wide gamut — the surface \
                 re-reads the top row's bytes as-is. The top row visibly \
                 muted relative to the bottom row means wide gamut is \
                 working: on a P3-referred surface the top row reads as the \
                 familiar sRGB primaries while the bottom row is the Display \
                 P3 primaries, outside the sRGB gamut.",
            );
            if space == GrantedSpace::Unrecognized {
                hint.push_str(
                    " The granted color space is not recognized — the top \
                     row is showing raw sRGB bytes.",
                );
            }
            let hint = ui.label(hint);

            // Layout probe: the readout must lay out inside the visible
            // screen — a canvas consuming the whole panel would push every
            // widget after it below the window, invisibly (egui's panel rect
            // itself expands with overflowing content, so the screen rect is
            // the only honest boundary). The hint label is the last widget,
            // so its placed rect is the canary.
            let hint_rect = hint.rect;
            let screen = ui.ctx().content_rect();
            assert!(
                hint_rect.is_finite()
                    && hint_rect.height() > 0.0
                    && screen.contains_rect(hint_rect),
                "the status readout must lay out on screen \
                 (hint label {hint_rect:?}, screen {screen:?})"
            );
            log::info!(
                "hdr_wide_gamut readout laid out: hint label {hint_rect:?} on screen {screen:?}"
            );
        });
    }
}

/// The color space the surface was actually granted, parsed from the
/// app-visible diagnostics string
/// ([`egui::ViewportInfo::surface_target`], e.g. "`Bgra8Unorm + DisplayP3`").
///
/// PR 1's structured per-viewport decision is not app-reachable — the
/// painter accessor lives below eframe — so this diagnostics string is the
/// channel. That string is pairing-table data (`SurfaceTarget`'s `Display`
/// impl writes "`format + color_space`"), and its suffix matches
/// [`wgpu::SurfaceColorSpace`]'s `Debug` name 1:1, so this is a table
/// lookup, not guesswork.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GrantedSpace {
    /// The fallback: bytes are re-read as ordinary sRGB.
    Srgb,
    /// The wide-gamut grant: bytes are read as Display P3.
    DisplayP3,
    /// Anything else (or no string at all): the conversion falls back to
    /// identity and the hint says so — raw bytes, loudly shown, never
    /// silently wrong.
    Unrecognized,
}

impl GrantedSpace {
    fn from_target(target: Option<&str>) -> Self {
        match target.and_then(|t| t.rsplit_once(" + ").map(|(_, space)| space)) {
            Some("Srgb") => Self::Srgb,
            Some("DisplayP3") => Self::DisplayP3,
            _ => Self::Unrecognized,
        }
    }
}

/// Linear-light conversion from sRGB into the granted space's primaries, per
/// column: column j maps sRGB primary j (red (1, 0, 0), green (0, 1, 0),
/// blue (0, 0, 1)).
fn srgb_to_granted_linear(space: GrantedSpace) -> [[f32; 3]; 3] {
    match space {
        // Identity: those spaces re-read the bytes as-is.
        GrantedSpace::Srgb | GrantedSpace::Unrecognized => {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        }
        // Display P3 (both D65-referred): `M_P3⁻¹ · M_sRGB`, published
        // verbatim in CSS Color 4 ("predefined-display-p3"); rows sum to 1
        // because both systems share the D65 white. NOT built from the P3 xy
        // chromaticities (0.680, 0.320) etc. — those are CIE
        // chromaticity-diagram coordinates that define the primaries, not
        // colors.
        GrantedSpace::DisplayP3 => [
            [0.822_461_97, 0.177_538_03, 0.0],
            [0.033_194_2, 0.966_805_8, 0.0],
            [0.017_082_63, 0.072_397_44, 0.910_519_93],
        ],
    }
}

/// The sRGB transfer function (IEC 61966-2-1): the u8 surface formats are
/// non-Srgb, so values written to them must arrive already encoded.
fn srgb_encode(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        12.92 * channel
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// The six grid cell colors for `space`: the top row is the sRGB primaries
/// converted into the granted space's encoding, the bottom row the surface's
/// native primaries as full-range bytes.
fn grid_cells(space: GrantedSpace) -> [egui::Color32; 6] {
    let m = srgb_to_granted_linear(space);
    let encode = |channel: f32| (srgb_encode(channel) * 255.0).round() as u8;

    let mut cells = [egui::Color32::BLACK; 6];
    for col in 0..3 {
        cells[col] =
            egui::Color32::from_rgb(encode(m[0][col]), encode(m[1][col]), encode(m[2][col]));
    }
    cells[3] = egui::Color32::from_rgb(255, 0, 0);
    cells[4] = egui::Color32::from_rgb(0, 255, 0);
    cells[5] = egui::Color32::from_rgb(0, 0, 255);
    cells
}

struct QuadRenderResources {
    /// The render pipeline for the format it was built for, created in
    /// `prepare` on first use and re-created when the target format changes
    /// (the window may have moved to a different monitor).
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    shader: wgpu::ShaderModule,
    /// Layout binding the cell-color uniform; shared by every pipeline this
    /// resource builds.
    pipeline_layout: wgpu::PipelineLayout,
    /// The six grid cell colors, read by the fragment shader.
    cells: wgpu::Buffer,
    cells_bind_group: wgpu::BindGroup,
    /// The cell colors currently uploaded to `cells`, so `prepare` only
    /// touches the GPU buffer when the decision changes.
    uploaded: [egui::Color32; 6],
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
                // Binds the cell-color uniform uploaded in `prepare`.
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
}

struct WideGamutQuad {
    /// This frame's six grid cell colors, derived from the granted color
    /// space (see [`GrantedSpace`]).
    cells: [egui::Color32; 6],
}

impl egui_wgpu::CallbackTrait for WideGamutQuad {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut QuadRenderResources = resources.get_mut().unwrap();
        if resources.uploaded != self.cells {
            // The shader reads sRGB-encoded floats (the u8 surface format is
            // non-Srgb): one rgba32float per cell.
            let mut bytes = [0u8; 6 * 4 * 4];
            for (cell, out) in self.cells.iter().zip(bytes.chunks_mut(16)) {
                for (channel, slot) in [cell.r(), cell.g(), cell.b(), 255]
                    .into_iter()
                    .zip(out.chunks_mut(4))
                {
                    slot.copy_from_slice((f32::from(channel) / 255.0).to_le_bytes().as_slice());
                }
            }
            queue.write_buffer(&resources.cells, 0, &bytes);
            resources.uploaded = self.cells;
        }
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
                render_pass.set_bind_group(0, &resources.cells_bind_group, &[]);
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

// The six grid cell colors (sRGB-encoded, 0-1), uploaded from the app when
// the granted color space changes: indices 0-2 are the top row — the sRGB
// primaries converted into the granted space's encoding — and 3-5 the bottom
// row, the surface's native primaries as full-range bytes.
@group(0) @binding(0)
var<uniform> CELLS: array<vec4<f32>, 6>;

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
    // The 2×3 grid of full-size cells, pixel-flush: the row split at height
    // / 2, the column splits at width / 3 — every fragment gets exactly one
    // cell's color, so there are no seam lines. In wgpu's NDC, y=+1 is the
    // TOP of the framebuffer, so with uv = (ndc + 1) * 0.5 from the vertex
    // stage, uv.y >= 0.5 is the top half on screen: the converted sRGB row
    // (cells 0-2) goes there, the native row (cells 3-5) below.
    let col = min(u32(in.uv.x * 3.0), 2u);
    let top = in.uv.y >= 0.5;
    let idx = select(col + 3u, col, top);
    return vec4<f32>(CELLS[idx].rgb, 1.0);
}
";
