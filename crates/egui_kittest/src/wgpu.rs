use core::{iter::once, time::Duration};
use std::sync::Arc;

use egui::TexturesDelta;
use egui_wgpu::{RenderState, ScreenDescriptor, WgpuSetup, wgpu};
use image::RgbaImage;

use crate::texture_to_image::texture_to_image;

/// The format of the offscreen render target.
///
/// Matches what `RenderState::create` used to pick for a surface-less render
/// state ([`egui_wgpu::preferred_framebuffer_format`] over its `[Rgba8Unorm]`
/// no-surface stub).
const OFFSCREEN_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Timeout for waiting on the GPU to finish rendering.
///
/// Windows will reset native drivers after 2 seconds of being stuck (known was TDR - timeout detection & recovery).
/// However, software rasterizers like lavapipe may not do that and take longer if there's a lot of work in flight.
/// In the end, what we really want to protect here against is undetected errors that lead to device loss
/// and therefore infinite waits it happens occasionally on MacOS/Metal as of writing.
pub(crate) const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default wgpu setup used for the wgpu renderer.
pub fn default_wgpu_setup() -> egui_wgpu::WgpuSetup {
    // No display handle needed for headless testing — we don't present to a window.
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();

    // WebGPU not supported yet since we rely on blocking screenshots.
    setup
        .instance_descriptor
        .backends
        .remove(wgpu::Backends::BROWSER_WEBGPU);

    // Prefer software rasterizers.
    setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
        let mut adapters = adapters.iter().collect::<Vec<_>>();

        // Adapters are already sorted by preferred backend by wgpu, but let's be explicit.
        adapters.sort_by_key(|a| match a.get_info().backend {
            wgpu::Backend::Metal => 0,
            wgpu::Backend::Vulkan => 1,
            wgpu::Backend::Dx12 => 2,
            wgpu::Backend::Gl => 4,
            wgpu::Backend::BrowserWebGpu => 6,
            wgpu::Backend::Noop => 7,
        });

        // Prefer CPU adapters, otherwise if we can't, prefer discrete GPU over integrated GPU.
        adapters.sort_by_key(|a| match a.get_info().device_type {
            wgpu::DeviceType::Cpu => 0, // CPU is the best for our purposes!
            wgpu::DeviceType::DiscreteGpu => 1,
            wgpu::DeviceType::Other
            | wgpu::DeviceType::IntegratedGpu
            | wgpu::DeviceType::VirtualGpu => 2,
        });

        adapters
            .first()
            .map(|a| (*a).clone())
            .ok_or_else(|| "No adapter found".to_owned())
    }));

    egui_wgpu::WgpuSetup::CreateNew(setup)
}

pub fn create_render_state(
    setup: WgpuSetup,
    options: egui_wgpu::RendererOptions,
) -> egui_wgpu::RenderState {
    // No display handle needed for headless testing — we don't present to a window.
    let instance = pollster::block_on(setup.new_instance());

    pollster::block_on(egui_wgpu::RenderState::create(
        &egui_wgpu::WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        },
        &instance,
        None,
        options,
    ))
    .expect("Failed to create render state")
}

/// Utility to render snapshots from a [`Harness`](crate::Harness) using [`egui_wgpu`].
pub struct WgpuTestRenderer {
    render_state: RenderState,

    /// The offscreen target format. [`wgpu::TextureFormat::Rgba8Unorm`] unless
    /// overridden — e.g. to `Rgba16Float` for extended-range numeric tests
    /// (plan §8 PR 3 deliverable 2: an explicit override, not the decider —
    /// a surfaceless viewport stays `FellBack { NoSurface }`).
    target_format: wgpu::TextureFormat,

    /// MSAA sample count for the offscreen pass; 1 = off (default). The
    /// pipeline is built for this count, and with more than one the pass
    /// renders into a same-format multi-sampled intermediate that resolves
    /// into the target (plan §9).
    msaa_samples: u32,
}

impl Default for WgpuTestRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl WgpuTestRenderer {
    /// Create a new [`WgpuTestRenderer`] with the default setup.
    pub fn new() -> Self {
        Self::with_options_and_target(
            egui_wgpu::RendererOptions::PREDICTABLE,
            OFFSCREEN_TARGET_FORMAT,
            1,
        )
    }

    /// Create a new [`WgpuTestRenderer`] with the given setup.
    pub fn from_setup(setup: WgpuSetup) -> Self {
        Self {
            render_state: create_render_state(setup, egui_wgpu::RendererOptions::PREDICTABLE),
            target_format: OFFSCREEN_TARGET_FORMAT,
            msaa_samples: 1,
        }
    }

    /// Create a new [`WgpuTestRenderer`] from an existing [`RenderState`].
    ///
    /// The offscreen target stays [`wgpu::TextureFormat::Rgba8Unorm`] and MSAA
    /// stays off; for other targets / sample counts use
    /// [`Self::with_options_and_target`] with the same render options the
    /// [`RenderState`] was built with.
    ///
    /// # Panics
    /// Panics if the [`RenderState`] has been used before.
    pub fn from_render_state(render_state: RenderState) -> Self {
        assert!(
            render_state
                .renderer
                .read()
                .texture(&egui::epaint::TextureId::Managed(0))
                .is_none(),
            "The RenderState passed in has been used before, pass in a fresh RenderState instead."
        );
        Self {
            render_state,
            target_format: OFFSCREEN_TARGET_FORMAT,
            msaa_samples: 1,
        }
    }

    /// Create a new [`WgpuTestRenderer`] with custom render options.
    ///
    /// A `msaa_samples` above 1 in `options` gets the same intermediate +
    /// resolve treatment as [`Self::with_options_and_target`].
    pub fn with_render_options(options: egui_wgpu::RendererOptions) -> Self {
        Self::with_options_and_target(
            options,
            OFFSCREEN_TARGET_FORMAT,
            options.msaa_samples.max(1),
        )
    }

    /// Create a new [`WgpuTestRenderer`] rendering into `target_format` with
    /// `msaa_samples` (1 = off), e.g. `Rgba16Float` + 4 for an MSAA-on-fp16
    /// test.
    ///
    /// `options.msaa_samples` must agree with `msaa_samples` — the multisample
    /// count is baked into the render pipeline.
    pub fn with_options_and_target(
        options: egui_wgpu::RendererOptions,
        target_format: wgpu::TextureFormat,
        msaa_samples: u32,
    ) -> Self {
        let msaa_samples = msaa_samples.max(1);
        assert_eq!(
            options.msaa_samples.max(1),
            msaa_samples,
            "RendererOptions::msaa_samples must match the requested msaa_samples"
        );
        Self {
            render_state: create_render_state(default_wgpu_setup(), options),
            target_format,
            msaa_samples,
        }
    }
}

impl crate::TestRenderer for WgpuTestRenderer {
    #[cfg(feature = "eframe")]
    fn setup_eframe(&self, cc: &mut eframe::CreationContext<'_>, frame: &mut eframe::Frame) {
        cc.wgpu_render_state = Some(self.render_state.clone());
        frame.wgpu_render_state = Some(self.render_state.clone());
    }

    fn handle_delta(&mut self, delta: &mut TexturesDelta) {
        let mut renderer = self.render_state.renderer.write();
        #[expect(clippy::iter_over_hash_type)] // Order doesn't matter here
        for (id, images) in delta.set.drain() {
            for image in images {
                renderer.update_texture(
                    &self.render_state.device,
                    &self.render_state.queue,
                    id,
                    &image,
                );
            }
        }

        #[expect(clippy::iter_over_hash_type)] // Order doesn't matter here
        for id in delta.free.drain() {
            renderer.free_texture(&id);
        }
    }

    /// Render the [`crate::Harness`] and return the resulting image.
    fn render(
        &mut self,
        ctx: &egui::Context,
        output: &egui::FullOutput,
    ) -> Result<RgbaImage, String> {
        let mut renderer = self.render_state.renderer.write();

        let mut encoder =
            self.render_state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Egui Command Encoder"),
                });

        let size = ctx.content_rect().size() * ctx.pixels_per_point();
        let screen = ScreenDescriptor {
            pixels_per_point: ctx.pixels_per_point(),
            size_in_pixels: [size.x.round() as u32, size.y.round() as u32],
        };

        let tessellated = ctx.tessellate(output.shapes.clone(), ctx.pixels_per_point());

        let user_buffers = renderer.update_buffers(
            &self.render_state.device,
            &self.render_state.queue,
            &mut encoder,
            &tessellated,
            &screen,
            self.target_format,
        );

        let texture = self
            .render_state
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Egui Texture"),
                size: wgpu::Extent3d {
                    width: screen.size_in_pixels[0],
                    height: screen.size_in_pixels[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // MSAA (plan §9): with samples > 1 the pass renders into a
        // multi-sampled intermediate — same format as the target — that
        // resolves into `texture`, which is what gets read back.
        let (view, resolve_target) = if self.msaa_samples > 1 {
            let intermediate = self
                .render_state
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Egui Texture (MSAA intermediate)"),
                    size: wgpu::Extent3d {
                        width: screen.size_in_pixels[0],
                        height: screen.size_in_pixels[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: self.msaa_samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.target_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
            (
                intermediate.create_view(&wgpu::TextureViewDescriptor::default()),
                Some(&texture_view),
            )
        } else {
            (texture_view.clone(), None)
        };

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Egui Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();

            renderer.render(&mut pass, &tessellated, &screen, self.target_format);
        }

        self.render_state
            .queue
            .submit(core::iter::chain(user_buffers, once(encoder.finish())));

        self.render_state
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(WAIT_TIMEOUT),
            })
            .map_err(|err| format!("PollError: {err}"))?;

        Ok(texture_to_image(
            &self.render_state.device,
            &self.render_state.queue,
            &texture,
        ))
    }
}
